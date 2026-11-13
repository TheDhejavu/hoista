use crate::error::{Error, Result};

/// A development build keeps its keys apart from an installed release's.
fn service() -> &'static str {
    match std::env::var("HOISTA_CHANNEL").ok().as_deref() {
        Some("dev") => "ai.hoista.desktop.dev",
        Some("stable") => "ai.hoista.desktop",
        _ if cfg!(debug_assertions) => "ai.hoista.desktop.dev",
        _ => "ai.hoista.desktop",
    }
}
const MAX_REMOTE_ID_BYTES: usize = 64;

/// This computer's key for each remote, in the system keychain.
pub struct Keys;

impl Keys {
    pub fn read(&self, remote_id: &str) -> Result<Option<String>> {
        match entry(remote_id)?.get_password() {
            Ok(key) => Ok(Some(key)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    pub fn write(&self, remote_id: &str, key: &str) -> Result<()> {
        Ok(entry(remote_id)?.set_password(key)?)
    }

    pub fn remove(&self, remote_id: &str) -> Result<()> {
        match entry(remote_id)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(error.into()),
        }
    }
}

fn entry(remote_id: &str) -> Result<keyring::Entry> {
    let valid = !remote_id.is_empty()
        && remote_id.len() <= MAX_REMOTE_ID_BYTES
        && remote_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-');
    if !valid {
        return Err(Error::new(
            "remote.not_found",
            "That remote isn’t in your list any more.",
        ));
    }
    Ok(keyring::Entry::new(
        service(),
        &format!("remote-{remote_id}"),
    )?)
}
