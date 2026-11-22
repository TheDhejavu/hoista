//! Servers Hoista makes at a provider (a Hetzner Cloud server): the keys it
//! makes them with and keeps to reach them.

pub mod keys;
mod service;

pub use service::{DeployKey, FileWrite, LogsAsk, RunResult, ServerService, approval_target};

use crate::{domain::Secret, ports::catalog::EngineError};
use keys::ServerKey;

/// The user Hoista signs in to its servers as. It can use `sudo` without a
/// password; root can't sign in over SSH at all.
pub const SERVER_USER: &str = "hoista";

/// What a new server is made with: the key Hoista signs in with, and the
/// server's own host key, made here so Hoista knows the server it reaches is
/// the one it made from the first connection.
#[derive(Debug)]
pub struct ServerSetup {
    /// The `authorized_keys` line for [`SERVER_USER`].
    pub authorized_key: String,
    /// The server's host key, public half, as an `ssh-ed25519 …` line.
    pub host_public_key: String,
    /// The server's host key, private half, in OpenSSH's format.
    pub host_private_key: Secret,
    /// The key Hoista signs in with, in OpenSSH's format.
    user_private_key: Secret,
}

impl ServerSetup {
    /// Two new keys, named after the server.
    ///
    /// # Errors
    /// Reports a system without a random source.
    pub fn generate(server: &str) -> Result<Self, EngineError> {
        let user = ServerKey::generate(&format!("hoista@{server}"))?;
        let host = ServerKey::generate(&format!("root@{server}"))?;
        Ok(Self {
            authorized_key: user.public_line(),
            host_public_key: host.public_line(),
            host_private_key: host.private_openssh()?,
            user_private_key: user.private_openssh()?,
        })
    }

    /// All of it as one vault entry, kept while the server is being made so a
    /// retry makes it with the same keys.
    #[must_use]
    pub fn to_secret(&self) -> Secret {
        Secret::new(
            serde_json::json!({
                "authorized_key": self.authorized_key,
                "host_public_key": self.host_public_key,
                "host_private_key": self.host_private_key.expose(),
                "user_private_key": self.user_private_key.expose(),
            })
            .to_string(),
        )
    }

    /// Reads an entry written by [`Self::to_secret`].
    ///
    /// # Errors
    /// Reports an entry in another shape.
    pub fn from_secret(secret: &Secret) -> Result<Self, EngineError> {
        let value: serde_json::Value =
            serde_json::from_str(secret.expose()).map_err(|_| EngineError::Store)?;
        let text = |key: &str| field(&value, key);
        Ok(Self {
            authorized_key: text("authorized_key")?,
            host_public_key: text("host_public_key")?,
            host_private_key: Secret::new(text("host_private_key")?),
            user_private_key: Secret::new(text("user_private_key")?),
        })
    }

    /// What Hoista keeps once the server is made: only what it needs to sign in.
    #[must_use]
    pub fn kept(&self) -> Secret {
        ServerAccess {
            user: SERVER_USER.to_owned(),
            private_key: Secret::new(self.user_private_key.expose().to_owned()),
            host_key: self.host_public_key.clone(),
            replace_host_key: true,
            agents_full_access: false,
        }
        .to_secret()
    }
}

/// What Hoista keeps to reach one server it made.
pub struct ServerAccess {
    /// Who to sign in as.
    pub user: String,
    /// The key to sign in with, in OpenSSH's format.
    pub private_key: Secret,
    /// The server's host key, `ssh-ed25519 …`: a server that shows another is
    /// refused.
    pub host_key: String,
    /// The host key is still the one in the server's `user_data`, which a
    /// Read token can fetch, so Hoista replaces it on the next connection.
    pub replace_host_key: bool,
    /// The person lets agents with Full access run commands here without
    /// asking, except those on the always-ask list. Off until they turn it on.
    pub agents_full_access: bool,
}

impl ServerAccess {
    /// As one vault entry.
    #[must_use]
    pub fn to_secret(&self) -> Secret {
        Secret::new(
            serde_json::json!({
                "user": self.user,
                "private_key": self.private_key.expose(),
                "host_key": self.host_key,
                "replace_host_key": self.replace_host_key,
                "agents_full_access": self.agents_full_access,
            })
            .to_string(),
        )
    }

    /// Reads a vault entry written from [`ServerSetup::kept`].
    ///
    /// # Errors
    /// Reports an entry in another shape.
    pub fn from_kept(kept: &Secret) -> Result<Self, EngineError> {
        let value: serde_json::Value =
            serde_json::from_str(kept.expose()).map_err(|_| EngineError::Store)?;
        Ok(Self {
            user: field(&value, "user")?,
            private_key: Secret::new(field(&value, "private_key")?),
            host_key: field(&value, "host_key")?,
            replace_host_key: value["replace_host_key"].as_bool().unwrap_or(false),
            agents_full_access: value["agents_full_access"].as_bool().unwrap_or(false),
        })
    }
}

fn field(value: &serde_json::Value, key: &str) -> Result<String, EngineError> {
    value[key]
        .as_str()
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
        .ok_or(EngineError::Store)
}

/// Where the vault keeps the way into the server with this Hoista resource id.
#[must_use]
pub fn vault_reference(resource_id: &str) -> String {
    format!("server/{resource_id}")
}

/// Where the vault keeps a server's keys while it's being made on `account_id`
/// under `name`.
#[must_use]
pub fn pending_reference(account_id: &str, name: &str) -> String {
    format!("server-pending/{account_id}/{name}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_survive_the_vault() {
        let setup = ServerSetup::generate("web-1").unwrap();
        let again = ServerSetup::from_secret(&setup.to_secret()).unwrap();
        assert_eq!(again.authorized_key, setup.authorized_key);
        assert_eq!(
            again.host_private_key.expose(),
            setup.host_private_key.expose()
        );
        let kept = ServerAccess::from_kept(&again.kept()).unwrap();
        assert_eq!(kept.user, SERVER_USER);
        assert_eq!(kept.host_key, setup.host_public_key);
        assert!(kept.replace_host_key);
        assert!(!kept.agents_full_access);
        assert!(kept.private_key.expose().contains("OPENSSH PRIVATE KEY"));
        assert_ne!(kept.private_key.expose(), setup.host_private_key.expose());
        assert!(setup.authorized_key.ends_with(" hoista@web-1"));
    }
}
