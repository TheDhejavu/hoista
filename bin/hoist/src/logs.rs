//! `hoist logs`: a project's recent logs, or live ones with `--follow`.

use crate::{
    context::{self, Context},
    ui,
};
use anyhow::{Context as _, bail};
use clap::Args;
use hoista_server::{
    application::engine::{LogCapture, LogFrame},
    client::ClientError,
    domain::logs::LogLevel,
};
use std::io::Write;

const MAX_FILTER_BYTES: usize = 256;
const RECONNECT_WAIT: std::time::Duration = std::time::Duration::from_secs(1);

#[derive(Args)]
pub(crate) struct LogsArgs {
    /// Project name or id. Default: the project linked to this folder.
    project: Option<String>,
    /// The resource, when the project has more than one.
    #[arg(long, short = 'r', value_name = "RESOURCE")]
    resource: Option<String>,
    /// Keep printing new lines as they come.
    #[arg(long, short = 'f')]
    follow: bool,
    /// Only lines containing this text.
    #[arg(long, value_name = "TEXT")]
    filter: Option<String>,
    /// Only lines at this level.
    #[arg(long, value_parser = ["debug", "info", "warn", "error"])]
    level: Option<String>,
    /// From this long ago (30m, 2h, 1d) or this time (RFC 3339). Not with --follow.
    #[arg(long, value_name = "WHEN", conflicts_with = "follow")]
    since: Option<String>,
}

/// `2h` or an RFC 3339 time, as an RFC 3339 time.
fn since(value: &str) -> anyhow::Result<String> {
    if chrono::DateTime::parse_from_rfc3339(value).is_ok() {
        return Ok(value.to_owned());
    }
    let (count, unit) = value.split_at(value.len().saturating_sub(1));
    let count: i64 = count
        .parse()
        .map_err(|_| ui::Usage("Write --since as 30m, 2h, 1d or an RFC 3339 time.".into()))?;
    let ago = match unit {
        "s" => chrono::Duration::try_seconds(count),
        "m" => chrono::Duration::try_minutes(count),
        "h" => chrono::Duration::try_hours(count),
        "d" => chrono::Duration::try_days(count),
        _ => return ui::usage("Write --since as 30m, 2h, 1d or an RFC 3339 time."),
    };
    let Some(ago) = ago else {
        return ui::usage("That --since is too far back.");
    };
    Ok((chrono::Utc::now() - ago).to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
}

fn level(entry: LogLevel) -> String {
    let text = format!("{entry:?}").to_lowercase();
    match entry {
        LogLevel::Error => console::style(format!("{text:<5}")).red().to_string(),
        LogLevel::Warn => console::style(format!("{text:<5}")).yellow().to_string(),
        _ => console::style(format!("{text:<5}")).dim().to_string(),
    }
}

fn write_frame(output: &mut impl Write, frame: &LogFrame, json: bool) -> Result<(), ClientError> {
    let result = if json {
        serde_json::to_writer(&mut *output, frame)
            .map_err(std::io::Error::other)
            .and_then(|()| writeln!(output))
    } else {
        match frame {
            LogFrame::Ready { .. } => {
                ui::note("Following. Ctrl-C to stop.");
                Ok(())
            }
            LogFrame::Log { entry } => writeln!(
                output,
                "{} {} {} {}",
                console::style(ui::clean(
                    entry.timestamp.as_deref().unwrap_or(&entry.observed_at)
                ))
                .dim(),
                level(entry.level),
                console::style(format!("[{}]", ui::clean(&entry.source))).dim(),
                ui::clean(&entry.message)
            ),
            LogFrame::End { .. } => Ok(()),
        }
    };
    result
        .and_then(|()| output.flush())
        .map_err(|_| ClientError::Rejected("Couldn't write the logs out.".into()))
}

pub(crate) async fn run(context: &Context, args: LogsArgs) -> anyhow::Result<()> {
    if args
        .filter
        .as_ref()
        .is_some_and(|text| text.len() > MAX_FILTER_BYTES)
    {
        return ui::usage("Use a --filter of at most 256 characters.");
    }
    let client = context.client().await?;
    let project = context::project(
        context,
        &client,
        args.project.as_deref(),
        "Name the project: hoist logs PROJECT.",
    )
    .await?;
    let resource = context::resource(
        context,
        &client,
        args.resource.as_deref(),
        Some(&project.id),
    )
    .await?;
    let json = ui::mode().json;
    let mut query = vec![("resource".to_owned(), ui::text(&resource, "id"))];
    if let Some(text) = &args.filter {
        query.push(("q".into(), text.clone()));
    }
    let encode = |query: &[(String, String)]| {
        context::query(
            &query
                .iter()
                .map(|(key, value)| (key.as_str(), value.as_str()))
                .collect::<Vec<_>>(),
        )
    };
    if args.follow {
        if let Some(level) = &args.level {
            query.push(("level".into(), level.clone()));
        }
        let path = format!(
            "{}?{}",
            context::path(&["projects", &project.id, "logs", "live"]),
            encode(&query)
        );
        let follow = async {
            loop {
                let mut reason = String::new();
                let mut cleanup = false;
                client
                    .follow_logs(&path, |frame| {
                        if let LogFrame::End {
                            reason: value,
                            cleanup_complete,
                        } = &frame
                        {
                            reason = value.clone();
                            cleanup = *cleanup_complete;
                        }
                        write_frame(&mut std::io::stdout().lock(), &frame, json)
                    })
                    .await?;
                if reason != "logs.session_expired" || !cleanup {
                    bail!(
                        "The log stream stopped ({}). Run it again to keep following.",
                        ui::clean(&reason)
                    );
                }
                ui::note("Reconnecting. Lines between the two sessions aren't replayed.");
                tokio::time::sleep(RECONNECT_WAIT).await;
            }
        };
        return tokio::select! {
            result = follow => result,
            signal = tokio::signal::ctrl_c() => { signal.context("Couldn't listen for Ctrl-C")?; Ok(()) }
        };
    }
    query.push((
        "since".into(),
        match &args.since {
            Some(value) => since(value)?,
            None => "1970-01-01T00:00:00Z".into(),
        },
    ));
    let mut capture: LogCapture = serde_json::from_value(
        client
            .get_json(&format!(
                "{}?{}",
                context::path(&["projects", &project.id, "logs"]),
                encode(&query)
            ))
            .await?,
    )
    .context("The logs didn't read")?;
    if let Some(wanted) = &args.level {
        let wanted: LogLevel = serde_json::from_value(serde_json::json!(wanted))?;
        capture.items.retain(|entry| entry.level == wanted);
    }
    let mut output = std::io::stdout().lock();
    if json {
        serde_json::to_writer_pretty(&mut output, &capture)?;
        writeln!(output)?;
    } else {
        if capture.items.is_empty() {
            ui::note("No log lines in that range.");
        }
        for entry in capture.items {
            write_frame(&mut output, &LogFrame::Log { entry }, false)?;
        }
    }
    if !capture.cleanup_complete
        || !matches!(
            capture.end_reason.as_str(),
            "logs.capture_complete" | "logs.capture_limit"
        )
    {
        bail!(
            "The provider didn't send every line ({}).",
            ui::clean(&capture.end_reason)
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn since_takes_relative_times_and_rfc3339() {
        assert_eq!(
            since("2026-01-01T00:00:00Z").unwrap(),
            "2026-01-01T00:00:00Z"
        );
        for value in ["30m", "2h", "1d", "45s"] {
            assert!(chrono::DateTime::parse_from_rfc3339(&since(value).unwrap()).is_ok());
        }
        for value in ["", "h", "2w", "yesterday"] {
            assert!(since(value).is_err());
        }
    }

    #[test]
    fn provider_lines_print_on_one_line_without_controls() {
        console::set_colors_enabled(false);
        let mut bytes = vec![];
        write_frame(
            &mut bytes,
            &LogFrame::Log {
                entry: hoista_server::domain::logs::LogEntry {
                    timestamp: None,
                    observed_at: "now".into(),
                    level: LogLevel::Info,
                    source: "console".into(),
                    message: "\x1b[2Jhello\nforged".into(),
                },
            },
            false,
        )
        .unwrap();
        let output = String::from_utf8(bytes).unwrap();
        assert!(!output.contains('\x1b'));
        assert_eq!(output.lines().count(), 1);
    }
}
