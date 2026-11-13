//! Which kind of Hoista this is, so builds on one computer never share files.
//!
//! A release build is the `stable` channel; a build with debug assertions
//! (`cargo build`, `cargo run`, `npm run tauri dev`) is the `dev` channel. Each
//! channel keeps its own state folder, its own Keychain entries and its own MCP
//! port, so a development build can run beside an installed release and neither
//! can read, lock or overwrite the other's data. `HOISTA_CHANNEL=dev` or
//! `HOISTA_CHANNEL=stable` overrides the build's own choice.

/// A kind of Hoista that has its own files, credentials and ports.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Channel {
    /// An installed release.
    Stable,
    /// A development build.
    Dev,
}

/// The channel this process belongs to.
pub fn current() -> Channel {
    from_choice(
        std::env::var("HOISTA_CHANNEL").ok().as_deref(),
        cfg!(debug_assertions),
    )
}

fn from_choice(choice: Option<&str>, debug_build: bool) -> Channel {
    match choice {
        Some("dev") => Channel::Dev,
        Some("stable") => Channel::Stable,
        _ if debug_build => Channel::Dev,
        _ => Channel::Stable,
    }
}

impl Channel {
    /// The state folder's name inside the home folder.
    pub fn state_folder_name(self) -> &'static str {
        match self {
            Self::Stable => ".hoista",
            Self::Dev => ".hoista-dev",
        }
    }

    /// The name Keychain Access shows on macOS.
    pub fn keychain_label(self) -> &'static str {
        match self {
            Self::Stable => "Hoista",
            Self::Dev => "Hoista Dev",
        }
    }

    /// The service name for credential stores that use reverse-DNS names.
    pub fn credential_service(self) -> &'static str {
        match self {
            Self::Stable => "ai.hoista",
            Self::Dev => "ai.hoista.dev",
        }
    }

    /// The loopback port MCP over HTTP listens on.
    pub fn mcp_port(self) -> u16 {
        match self {
            Self::Stable => 8788,
            Self::Dev => 8789,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_builds_are_dev_and_release_builds_are_stable() {
        assert_eq!(from_choice(None, true), Channel::Dev);
        assert_eq!(from_choice(None, false), Channel::Stable);
    }

    #[test]
    fn the_environment_overrides_the_build() {
        assert_eq!(from_choice(Some("stable"), true), Channel::Stable);
        assert_eq!(from_choice(Some("dev"), false), Channel::Dev);
        assert_eq!(from_choice(Some("nonsense"), false), Channel::Stable);
    }

    #[test]
    fn channels_share_nothing() {
        let (stable, dev) = (Channel::Stable, Channel::Dev);
        assert_ne!(stable.state_folder_name(), dev.state_folder_name());
        assert_ne!(stable.keychain_label(), dev.keychain_label());
        assert_ne!(stable.credential_service(), dev.credential_service());
        assert_ne!(stable.mcp_port(), dev.mcp_port());
    }
}
