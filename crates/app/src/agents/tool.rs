//! The coding tools Hoista knows how to set up.

/// Coding tools Hoista knows how to set up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    /// Claude Code: skills in `~/.claude/skills`, rules in `~/.claude/settings.json`.
    ClaudeCode,
    /// Codex: skills in `~/.codex/skills`.
    Codex,
}

impl Tool {
    /// The tool an agent's name refers to ("claude-code", "Claude Code", "codex").
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        let plain: String = name
            .chars()
            .filter(char::is_ascii_alphanumeric)
            .collect::<String>()
            .to_ascii_lowercase();
        if plain.contains("claudecode") {
            Some(Self::ClaudeCode)
        } else if plain.contains("codex") {
            Some(Self::Codex)
        } else {
            None
        }
    }

    /// Its name as people know it.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::ClaudeCode => "Claude Code",
            Self::Codex => "Codex",
        }
    }
}
