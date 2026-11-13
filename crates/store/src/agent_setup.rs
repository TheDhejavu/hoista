//! Sets up a coding tool to use Hoista, in one step the person agrees to:
//! Hoista's skills and, for Claude Code, a rule that lets Hoista's tools run.
//!
//! Hoista asks before any delete itself, so Claude Code's own block would only
//! hide that question. The settings file is backed up before it's changed.

use crate::application::agents::tool::Tool;
use std::path::{Path, PathBuf};

/// Hoista's skills, as shipped in the repository's `skills/` folder.
pub const SKILLS: [(&str, &str); 5] = [
    ("hoista", include_str!("../../../skills/hoista/SKILL.md")),
    (
        "hoista-setup",
        include_str!("../../../skills/hoista-setup/SKILL.md"),
    ),
    (
        "hoista-deploy",
        include_str!("../../../skills/hoista-deploy/SKILL.md"),
    ),
    (
        "hoista-fix",
        include_str!("../../../skills/hoista-fix/SKILL.md"),
    ),
    (
        "hoista-manage",
        include_str!("../../../skills/hoista-manage/SKILL.md"),
    ),
];

/// The Claude Code permission rule that lets every Hoista tool run without
/// Claude Code's own prompt or block.
pub const CLAUDE_RULE: &str = "mcp__hoista";

/// The person's home folder.
#[must_use]
pub fn home() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

/// Sets the tool up under `home`. Returns one sentence per thing done.
///
/// # Errors
/// Reports folders or files that couldn't be written, and a settings file
/// that isn't valid JSON (left untouched).
pub fn set_up(tool: Tool, home: &Path) -> std::io::Result<Vec<String>> {
    let base = match tool {
        Tool::ClaudeCode => home.join(".claude"),
        Tool::Codex => home.join(".codex"),
    };
    let skills = base.join("skills");
    for (name, text) in SKILLS {
        let folder = skills.join(name);
        std::fs::create_dir_all(&folder)?;
        std::fs::write(folder.join("SKILL.md"), text)?;
    }
    let mut done = vec![format!(
        "{}: added Hoista's {} skills.",
        tool.label(),
        SKILLS.len()
    )];
    if tool == Tool::ClaudeCode && allow_hoista(&base.join("settings.json"))? {
        done.push("Claude Code: Hoista's tools no longer need Claude Code's own OK; Hoista still asks you before deleting anything.".into());
    }
    Ok(done)
}

/// Adds the Hoista rule to `permissions.allow`, keeping everything else.
/// Returns false when it was already there.
fn allow_hoista(settings: &Path) -> std::io::Result<bool> {
    let text = match std::fs::read_to_string(settings) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => "{}".to_owned(),
        Err(error) => return Err(error),
    };
    let mut value: serde_json::Value =
        serde_json::from_str(if text.trim().is_empty() { "{}" } else { &text }).map_err(|_| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Claude Code's settings file isn't valid JSON, so Hoista left it alone.",
            )
        })?;
    let Some(root) = value.as_object_mut() else {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "Claude Code's settings file isn't a JSON object, so Hoista left it alone.",
        ));
    };
    let permissions = root
        .entry("permissions")
        .or_insert_with(|| serde_json::json!({}));
    let Some(permissions) = permissions.as_object_mut() else {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "Claude Code's permissions setting isn't an object, so Hoista left it alone.",
        ));
    };
    let allow = permissions
        .entry("allow")
        .or_insert_with(|| serde_json::json!([]));
    let Some(allow) = allow.as_array_mut() else {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "Claude Code's allow list isn't a list, so Hoista left it alone.",
        ));
    };
    if allow.iter().any(|rule| rule == CLAUDE_RULE) {
        return Ok(false);
    }
    allow.push(serde_json::Value::String(CLAUDE_RULE.to_owned()));
    if let Some(folder) = settings.parent() {
        std::fs::create_dir_all(folder)?;
    }
    if settings.exists() {
        std::fs::copy(settings, settings.with_extension("json.before-hoista"))?;
    }
    let mut pretty = serde_json::to_string_pretty(&value).map_err(std::io::Error::other)?;
    pretty.push('\n');
    std::fs::write(settings, pretty)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_map_to_tools() {
        assert_eq!(Tool::from_name("claude-code"), Some(Tool::ClaudeCode));
        assert_eq!(Tool::from_name("Claude Code"), Some(Tool::ClaudeCode));
        assert_eq!(Tool::from_name("codex"), Some(Tool::Codex));
        assert_eq!(Tool::from_name("Cursor"), None);
    }

    #[test]
    fn claude_code_keeps_its_settings_and_gets_one_rule() {
        let home = tempfile::tempdir().expect("temp");
        let settings = home.path().join(".claude/settings.json");
        std::fs::create_dir_all(settings.parent().unwrap()).unwrap();
        std::fs::write(
            &settings,
            r#"{"model":"opus","permissions":{"allow":["Bash(npm test)"],"deny":["Read(.env)"]}}"#,
        )
        .unwrap();
        let done = set_up(Tool::ClaudeCode, home.path()).unwrap();
        assert_eq!(done.len(), 2);
        let value: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&settings).unwrap()).unwrap();
        assert_eq!(value["model"], "opus");
        assert_eq!(value["permissions"]["deny"][0], "Read(.env)");
        assert_eq!(
            value["permissions"]["allow"],
            serde_json::json!(["Bash(npm test)", CLAUDE_RULE])
        );
        assert!(
            home.path()
                .join(".claude/settings.json.before-hoista")
                .exists()
        );
        assert!(
            home.path()
                .join(".claude/skills/hoista-deploy/SKILL.md")
                .exists()
        );
        // Running again changes nothing more.
        assert_eq!(set_up(Tool::ClaudeCode, home.path()).unwrap().len(), 1);
        // A file that isn't JSON is left alone.
        std::fs::write(&settings, "not json").unwrap();
        assert!(set_up(Tool::ClaudeCode, home.path()).is_err());
        assert_eq!(std::fs::read_to_string(&settings).unwrap(), "not json");
    }
}
