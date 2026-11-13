//! Hoista's core types and rules: projects, accounts, resources, deploys,
//! environments and the checks on them, plus the person's profile and the
//! [`Secret`] type. No network, disk or database access.

pub mod backup;
pub mod database_export;
pub mod database_observation;
pub mod deletion;
pub mod domains;
pub mod environment;
pub mod leaks;
pub mod logs;
pub mod map_layout;
pub mod model;
pub mod oauth;
pub mod provenance;
pub mod provider;
pub mod schedules;

pub use provider::{Provider, UnknownProvider};

use std::fmt;
use zeroize::Zeroizing;

const MAX_NAME_CHARS: usize = 80;
const MAX_INITIALS_CHARS: usize = 4;

/// The person's name and initials, as the apps show them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Profile {
    name: String,
    initials: String,
}

/// A name or initials Hoista won't save.
#[derive(Debug, thiserror::Error)]
#[error(
    "Use a name of 1–80 characters and initials of 1–4 characters, without control characters."
)]
pub struct InvalidProfile;

impl Profile {
    /// A profile, with spaces trimmed from both ends.
    ///
    /// # Errors
    /// Returns [`InvalidProfile`] when either is empty, too long or has a
    /// control character.
    pub fn new(name: String, initials: String) -> Result<Self, InvalidProfile> {
        let name = name.trim();
        let initials = initials.trim();
        if name.is_empty()
            || initials.is_empty()
            || name.chars().count() > MAX_NAME_CHARS
            || initials.chars().count() > MAX_INITIALS_CHARS
            || name.chars().any(char::is_control)
            || initials.chars().any(char::is_control)
        {
            return Err(InvalidProfile);
        }
        Ok(Self {
            name: name.into(),
            initials: initials.into(),
        })
    }

    /// The name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The initials, shown on the account menu.
    pub fn initials(&self) -> &str {
        &self.initials
    }
}

/// A token, password or other secret value. Its memory is wiped when it's
/// dropped, and `Debug` never prints it.
pub struct Secret(Zeroizing<String>);

/// What a [`Secret`] starts with when it's a handle: the name of a vault
/// entry standing in for a token. On a server, the process agents talk to
/// holds handles; only the credential keeper holds the tokens themselves.
pub const HANDLE: &str = "\u{1}hoista-credential:";

impl Secret {
    /// Takes the value. No other copy is kept.
    pub fn new(value: String) -> Self {
        Self(Zeroizing::new(value))
    }

    /// The value itself, to hand to a vault or a provider. Don't keep it.
    pub fn expose(&self) -> &str {
        self.0.as_str()
    }

    /// A handle for the vault entry `key` (`account/<id>`).
    #[must_use]
    pub fn handle(key: &str) -> Self {
        Self::new(format!("{HANDLE}{key}"))
    }

    /// The vault entry's name, when this is a handle and not a secret.
    #[must_use]
    pub fn handle_key(&self) -> Option<&str> {
        self.expose().strip_prefix(HANDLE)
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret([REDACTED])")
    }
}

impl<'de> serde::Deserialize<'de> for Secret {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer).map(Self::new)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_rejects_empty_oversized_and_control_fields() {
        for (name, initials) in [
            (" ".into(), "A".into()),
            ("A".into(), "ABCDE".into()),
            ("a".repeat(81), "A".into()),
            ("A\nB".into(), "AB".into()),
        ] {
            assert!(Profile::new(name, initials).is_err());
        }
        let profile = Profile::new("  Ayodeji  ".into(), " AY ".into()).unwrap();
        assert_eq!(profile.name(), "Ayodeji");
        assert_eq!(profile.initials(), "AY");
    }

    #[test]
    fn secret_debug_is_redacted() {
        assert_eq!(
            format!("{:?}", Secret::new("private-value".into())),
            "Secret([REDACTED])"
        );
    }
}
