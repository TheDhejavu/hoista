//! Output and prompts for `hoist`: tables for people, JSON for scripts.
//!
//! Data goes to stdout, everything else to stderr. Prompts need stdin and
//! stderr on a terminal and no `--no-input` or `CI`; otherwise a missing
//! argument is an error naming its flag (exit code 2).

use anyhow::bail;
use serde_json::Value;
use std::io::{IsTerminal, Write};
use std::sync::OnceLock;

/// How this run prints and asks.
#[derive(Clone, Copy)]
pub(crate) struct Mode {
    pub(crate) json: bool,
    pub(crate) yes: bool,
    pub(crate) input: bool,
}

static MODE: OnceLock<Mode> = OnceLock::new();

/// Fixes the mode for this run, from the global flags.
pub(crate) fn init(json: bool, yes: bool, no_input: bool) {
    let terminal = std::io::stdin().is_terminal() && std::io::stderr().is_terminal();
    let ci = std::env::var_os("CI").is_some_and(|value| !value.is_empty() && value != "false");
    if std::env::var_os("NO_COLOR").is_some_and(|value| !value.is_empty()) || !terminal {
        console::set_colors_enabled(false);
        console::set_colors_enabled_stderr(false);
    }
    let _ = MODE.set(Mode {
        json,
        yes,
        input: terminal && !no_input && !ci,
    });
}

pub(crate) fn mode() -> Mode {
    *MODE.get().unwrap_or(&Mode {
        json: false,
        yes: false,
        input: false,
    })
}

/// Whether this run may ask questions.
pub(crate) fn can_ask() -> bool {
    mode().input
}

/// A mistake in how the command was called: exit code 2.
#[derive(Debug)]
pub(crate) struct Usage(pub(crate) String);
impl std::fmt::Display for Usage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Usage {}

/// Fails with exit code 2 and the way to pass what was missing.
pub(crate) fn usage<T>(message: impl Into<String>) -> anyhow::Result<T> {
    Err(Usage(message.into()).into())
}

/// Text from a provider or a person, safe to print: control characters escaped.
pub(crate) fn clean(value: &str) -> String {
    value
        .chars()
        .flat_map(|c| {
            if c.is_control() {
                c.escape_default().collect::<Vec<_>>()
            } else {
                vec![c]
            }
        })
        .collect()
}

/// A field of a JSON value as display text: `a.b.c` paths, `-` when absent.
pub(crate) fn text(value: &Value, path: &str) -> String {
    let mut at = value;
    for key in path.split('.').filter(|key| !key.is_empty()) {
        at = &at[key];
    }
    match at {
        Value::Null => "-".into(),
        Value::String(text) if text.is_empty() => "-".into(),
        Value::String(text) => clean(text),
        Value::Bool(true) => "yes".into(),
        Value::Bool(false) => "no".into(),
        Value::Array(items) if items.is_empty() => "-".into(),
        Value::Array(items) => items
            .iter()
            .map(|item| match item {
                Value::String(text) => clean(text),
                other => clean(&other.to_string()),
            })
            .collect::<Vec<_>>()
            .join(", "),
        other => clean(&other.to_string()),
    }
}

/// An RFC 3339 time as a short date in local time: `2026-11-09 14:05`.
pub(crate) fn when(value: &Value, path: &str) -> String {
    let raw = text(value, path);
    chrono::DateTime::parse_from_rfc3339(&raw).map_or(raw, |time| {
        time.with_timezone(&chrono::Local)
            .format("%Y-%m-%d %H:%M")
            .to_string()
    })
}

/// A table printed with aligned columns and a dim header.
pub(crate) struct Table {
    headers: Vec<String>,
    rows: Vec<Vec<String>>,
}

impl Table {
    pub(crate) fn new(headers: &[&str]) -> Self {
        Self {
            headers: headers.iter().map(|header| (*header).to_owned()).collect(),
            rows: Vec::new(),
        }
    }

    pub(crate) fn row(&mut self, cells: Vec<String>) {
        self.rows.push(cells);
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    pub(crate) fn print(&self) {
        let mut widths: Vec<usize> = self
            .headers
            .iter()
            .map(|header| console::measure_text_width(header))
            .collect();
        for row in &self.rows {
            for (index, cell) in row.iter().enumerate() {
                if let Some(width) = widths.get_mut(index) {
                    *width = (*width).max(console::measure_text_width(cell));
                }
            }
        }
        let line = |cells: &[String]| {
            let last = cells.len().saturating_sub(1);
            cells
                .iter()
                .enumerate()
                .map(|(index, cell)| {
                    if index == last {
                        cell.clone()
                    } else {
                        console::pad_str(cell, widths[index], console::Alignment::Left, None)
                            .into_owned()
                    }
                })
                .collect::<Vec<_>>()
                .join("   ")
        };
        let mut out = std::io::stdout().lock();
        let _ = writeln!(
            out,
            "{}",
            console::style(line(&self.headers))
                .dim()
                .to_string()
                .trim_end()
        );
        for row in &self.rows {
            let _ = writeln!(out, "{}", line(row).trim_end());
        }
    }
}

/// Prints `value` as JSON when `--json` is on; otherwise runs `human`.
pub(crate) fn show(value: &Value, human: impl FnOnce(&Value)) -> anyhow::Result<()> {
    if mode().json {
        println!("{}", serde_json::to_string_pretty(value)?);
    } else {
        human(value);
    }
    Ok(())
}

/// Prints a list, or `empty` when there's nothing in it.
pub(crate) fn list(
    items: &Value,
    empty: &str,
    build: impl Fn(&Value) -> Vec<String>,
    headers: &[&str],
) -> anyhow::Result<()> {
    let rows = items
        .as_array()
        .or_else(|| items["items"].as_array())
        .cloned()
        .unwrap_or_default();
    if mode().json {
        println!("{}", serde_json::to_string_pretty(&Value::Array(rows))?);
        return Ok(());
    }
    if rows.is_empty() {
        note(empty);
        return Ok(());
    }
    let mut table = Table::new(headers);
    for item in &rows {
        table.row(build(item));
    }
    table.print();
    Ok(())
}

/// Labelled fields, one per line: `Name     api`.
pub(crate) fn fields(rows: &[(&str, String)]) {
    let width = rows.iter().map(|(label, _)| label.len()).max().unwrap_or(0);
    for (label, value) in rows {
        println!(
            "{}  {value}",
            console::style(format!("{label:<width$}")).dim()
        );
    }
}

/// A finished action, on stderr: `✓ Saved`.
pub(crate) fn done(message: &str) {
    if !mode().json {
        eprintln!("{} {message}", console::style("✓").green());
    }
}

/// Progress or a hint, on stderr.
pub(crate) fn note(message: &str) {
    if !mode().json {
        eprintln!("{message}");
    }
}

/// A warning, on stderr.
pub(crate) fn warn(message: &str) {
    eprintln!("{} {message}", console::style("!").yellow());
}

fn theme() -> dialoguer::theme::ColorfulTheme {
    dialoguer::theme::ColorfulTheme::default()
}

/// Picks one of `items`. Only call when [`can_ask`] is true.
pub(crate) fn pick(prompt: &str, items: &[String]) -> anyhow::Result<usize> {
    if items.is_empty() {
        bail!("There is nothing to choose from.");
    }
    Ok(dialoguer::Select::with_theme(&theme())
        .with_prompt(prompt)
        .items(items)
        .default(0)
        .interact()?)
}

/// Picks any number of `items`. Only call when [`can_ask`] is true.
pub(crate) fn pick_many(prompt: &str, items: &[String]) -> anyhow::Result<Vec<usize>> {
    Ok(dialoguer::MultiSelect::with_theme(&theme())
        .with_prompt(format!("{prompt} (space to select, enter to confirm)"))
        .items(items)
        .interact()?)
}

/// Asks for a line of text, with an optional default.
pub(crate) fn ask(prompt: &str, default: Option<&str>) -> anyhow::Result<String> {
    let theme = theme();
    let mut input = dialoguer::Input::<String>::with_theme(&theme).with_prompt(prompt);
    if let Some(default) = default {
        input = input.default(default.to_owned());
    }
    let value = input.interact_text()?;
    if value.chars().any(char::is_control) {
        bail!("Use text without control characters.");
    }
    Ok(value.trim().to_owned())
}

/// Asks before doing something. `--yes` answers yes; with no terminal and
/// no `--yes`, the command stops and says to pass `--yes`.
pub(crate) fn confirm(prompt: &str, default: bool) -> anyhow::Result<bool> {
    if mode().yes {
        return Ok(true);
    }
    if !can_ask() {
        return usage(format!("{prompt} Pass --yes to go ahead without asking."));
    }
    Ok(dialoguer::Confirm::with_theme(&theme())
        .with_prompt(prompt)
        .default(default)
        .interact()?)
}

/// For deleting: the person types `name` exactly. `--confirm NAME` stands
/// in for it in scripts.
pub(crate) fn type_to_confirm(name: &str, given: Option<&str>) -> anyhow::Result<()> {
    if let Some(given) = given {
        if given == name {
            return Ok(());
        }
        bail!(
            "--confirm doesn't match. Type the name exactly: {}",
            clean(name)
        );
    }
    if !can_ask() {
        return usage(format!(
            "Deleting needs the name typed to confirm. Pass --confirm {}.",
            clean(name)
        ));
    }
    let typed = ask(&format!("Type {} to confirm", clean(name)), None)?;
    if typed != name {
        bail!("That doesn't match. Nothing was deleted.");
    }
    Ok(())
}

/// A required value: given, asked for, or a usage error naming `how`.
pub(crate) fn need(given: Option<String>, prompt: &str, how: &str) -> anyhow::Result<String> {
    if let Some(value) = given {
        return Ok(value);
    }
    if can_ask() {
        let value = ask(prompt, None)?;
        if value.is_empty() {
            return usage(format!("Nothing given. {how}"));
        }
        return Ok(value);
    }
    usage(how.to_owned())
}

/// Opens `url` in the browser and says so, with the address to open by hand.
pub(crate) fn open(url: &str) {
    note(&format!("Opening {}", clean(url)));
    hoista_server::stdio::open_in_browser(url);
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn provider_text_never_reaches_the_terminal_with_controls() {
        assert_eq!(clean("\x1b[2Jok\nnext"), "\\u{1b}[2Jok\\nnext");
        let value = json!({"a": {"b": "x\ty"}, "list": ["one", "two"], "empty": [], "on": true});
        assert_eq!(text(&value, "a.b"), "x\\ty");
        assert_eq!(text(&value, "list"), "one, two");
        assert_eq!(text(&value, "empty"), "-");
        assert_eq!(text(&value, "missing.deep"), "-");
        assert_eq!(text(&value, "on"), "yes");
        assert_eq!(text(&json!("whole"), ""), "whole");
    }

    #[test]
    fn times_shorten_and_unknown_text_stays() {
        assert_eq!(when(&json!({"at": "not a time"}), "at"), "not a time");
        assert_eq!(when(&json!({"at": "2026-11-09T14:05:00Z"}), "at").len(), 16);
    }
}
