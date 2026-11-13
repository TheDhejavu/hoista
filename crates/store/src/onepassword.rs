//! Provider tokens kept in 1Password: Hoista stores only the `op://` reference and
//! reads the token with `op read` when a provider call needs it.
//!
//! The value is cached in memory only, briefly, so a rotated token is picked
//! up; it is never written to disk.

use crate::{
    application::ports::{catalog::EngineError, vault::TokenVault},
    domain::Secret,
};
use std::{
    collections::HashMap,
    future::Future,
    pin::Pin,
    sync::Arc,
    time::{Duration, Instant},
};

/// What every 1Password reference starts with.
pub const SCHEME: &str = "op://";
/// `vault/item/field`, or `vault/item/section/field`.
const PATH_PARTS: std::ops::RangeInclusive<usize> = 3..=4;
const MAX_REFERENCE_BYTES: usize = 512;
const MAX_VALUE_BYTES: usize = 64 * 1024;

/// A value read from 1Password is forgotten this long after it was read...
const HOLD_AT_MOST: Duration = Duration::from_secs(15 * 60);
/// ...or this long after it was last used, whichever comes first.
const HOLD_WHILE_IDLE: Duration = Duration::from_secs(5 * 60);
/// Long enough for a Touch ID prompt.
const WAIT: Duration = Duration::from_secs(60);

/// Whether `value` is a 1Password reference.
#[must_use]
pub fn is_reference(value: &str) -> bool {
    let Some(path) = value.strip_prefix(SCHEME) else {
        return false;
    };
    let parts: Vec<&str> = path.split('/').collect();
    PATH_PARTS.contains(&parts.len())
        && parts.iter().all(|part| !part.is_empty())
        && value.len() <= MAX_REFERENCE_BYTES
        && !value.chars().any(|c| c.is_control() || c == '\\')
}

/// Reading one reference.
pub type ReadReference = Pin<Box<dyn Future<Output = Result<Secret, EngineError>> + Send>>;
/// How references are read: the 1Password CLI, or a test's stand-in.
pub type Reader = Arc<dyn Fn(String) -> ReadReference + Send + Sync>;

struct Held {
    value: Secret,
    read_at: Instant,
    used_at: Instant,
}

/// A vault whose entries may be 1Password references, read through `op`.
pub struct ReferencedVault {
    inner: Arc<dyn TokenVault>,
    held: tokio::sync::Mutex<HashMap<String, Held>>,
    read: Reader,
}

impl ReferencedVault {
    /// References in `inner` are read with the 1Password CLI.
    #[must_use]
    pub fn new(inner: Arc<dyn TokenVault>) -> Self {
        Self::with_reader(inner, Arc::new(|reference| Box::pin(op_read(reference))))
    }

    /// With another way of reading a reference, for tests.
    #[must_use]
    pub fn with_reader(inner: Arc<dyn TokenVault>, read: Reader) -> Self {
        Self {
            inner,
            held: tokio::sync::Mutex::default(),
            read,
        }
    }

    async fn value_of(&self, reference: &str) -> Result<Secret, EngineError> {
        let now = Instant::now();
        let mut held = self.held.lock().await;
        held.retain(|_, item| {
            now.duration_since(item.read_at) < HOLD_AT_MOST
                && now.duration_since(item.used_at) < HOLD_WHILE_IDLE
        });
        if let Some(item) = held.get_mut(reference) {
            item.used_at = now;
            return Ok(Secret::new(item.value.expose().to_owned()));
        }
        drop(held);
        let value = (self.read)(reference.to_owned()).await?;
        let mut held = self.held.lock().await;
        held.insert(
            reference.to_owned(),
            Held {
                value: Secret::new(value.expose().to_owned()),
                read_at: Instant::now(),
                used_at: Instant::now(),
            },
        );
        Ok(value)
    }
}

#[async_trait::async_trait]
impl TokenVault for ReferencedVault {
    async fn read(&self, reference: &str) -> Result<Secret, EngineError> {
        let stored = self.inner.read(reference).await?;
        self.resolve(&stored).await
    }

    async fn write(&self, reference: &str, value: Secret) -> Result<(), EngineError> {
        self.held.lock().await.clear();
        self.inner.write(reference, value).await
    }

    async fn delete(&self, reference: &str) -> Result<(), EngineError> {
        self.held.lock().await.clear();
        self.inner.delete(reference).await
    }

    async fn check(&self) -> Result<(), EngineError> {
        self.inner.check().await
    }

    async fn resolve(&self, value: &Secret) -> Result<Secret, EngineError> {
        if value.expose().starts_with(SCHEME) {
            if !is_reference(value.expose()) {
                return Err(EngineError::Invalid(
                    "Use a 1Password reference like op://Vault/Item/field.",
                ));
            }
            return self.value_of(value.expose()).await;
        }
        Ok(Secret::new(value.expose().to_owned()))
    }
}

/// Reads one reference with the 1Password CLI. Only `PATH`, `HOME` and
/// 1Password's own settings reach it; the value comes back on stdout.
async fn op_read(reference: String) -> Result<Secret, EngineError> {
    let mut command = crate::tool_paths::command("op");
    command
        .args(["read", "--no-newline", "--", &reference])
        .env_clear()
        .kill_on_drop(true)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null());
    for name in [
        "PATH",
        "HOME",
        "USERPROFILE",
        "SYSTEMROOT",
        "TMPDIR",
        "OP_SERVICE_ACCOUNT_TOKEN",
        "OP_ACCOUNT",
        "OP_CONNECT_PROVIDER",
        "OP_CONNECT_TOKEN",
        "XDG_RUNTIME_DIR",
    ] {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
    let child = command.spawn().map_err(|_| EngineError::Provider {
        evidence: None,
        code: "onepassword.missing".into(),
        message: "Install the 1Password CLI (op) and turn on 1Password → Settings → Developer → Integrate with 1Password CLI, or use a token instead.".into(),
    })?;
    let output = tokio::time::timeout(WAIT, child.wait_with_output())
        .await
        .map_err(|_| EngineError::Provider {
            evidence: None,
            code: "onepassword.timeout".into(),
            message: "1Password didn't answer within a minute. Unlock it, then try again.".into(),
        })?
        .map_err(|_| EngineError::Store)?;
    let value = String::from_utf8(output.stdout).unwrap_or_default();
    if !output.status.success() || value.is_empty() || value.len() > MAX_VALUE_BYTES {
        return Err(EngineError::Provider {
            evidence: None,
            code: "onepassword.unreadable".into(),
            message: format!(
                "1Password couldn't read {reference}. Check the reference and that 1Password is unlocked."
            ),
        });
    }
    Ok(Secret::new(value))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[derive(Default)]
    struct Entries(std::sync::Mutex<HashMap<String, String>>);

    #[async_trait::async_trait]
    impl TokenVault for Entries {
        async fn read(&self, key: &str) -> Result<Secret, EngineError> {
            self.0
                .lock()
                .map_err(|_| EngineError::Store)?
                .get(key)
                .map(|value| Secret::new(value.clone()))
                .ok_or(EngineError::NotFound)
        }

        async fn write(&self, key: &str, value: Secret) -> Result<(), EngineError> {
            self.0
                .lock()
                .map_err(|_| EngineError::Store)?
                .insert(key.into(), value.expose().into());
            Ok(())
        }

        async fn delete(&self, key: &str) -> Result<(), EngineError> {
            self.0.lock().map_err(|_| EngineError::Store)?.remove(key);
            Ok(())
        }
    }

    #[test]
    fn references_look_like_op_urls() {
        assert!(is_reference("op://Private/Railway/token"));
        assert!(is_reference("op://Work/Fly.io/section/api token"));
        for not in [
            "op://Private/Railway",
            "op:///Railway/token",
            "op://a/b/c/d/e",
            "railway_abc123",
            "op://a/b/c\n",
        ] {
            assert!(!is_reference(not), "{not:?}");
        }
    }

    #[tokio::test]
    async fn only_the_reference_is_kept_and_its_value_is_read_when_needed() {
        let entries = Arc::new(Entries::default());
        let reads = Arc::new(AtomicUsize::new(0));
        let counter = reads.clone();
        let vault = ReferencedVault::with_reader(
            entries.clone(),
            Arc::new(move |reference: String| {
                counter.fetch_add(1, Ordering::SeqCst);
                Box::pin(async move {
                    if reference == "op://Private/Railway/token" {
                        Ok(Secret::new("railway-token-from-1password".into()))
                    } else {
                        Err(EngineError::Invalid("no such item"))
                    }
                })
            }),
        );
        vault
            .write(
                "account/a",
                Secret::new("op://Private/Railway/token".into()),
            )
            .await
            .expect("write");
        vault
            .write("account/b", Secret::new("plain-token".into()))
            .await
            .expect("write");
        // Stored: the reference only.
        assert_eq!(
            entries.read("account/a").await.expect("raw").expose(),
            "op://Private/Railway/token"
        );
        // Read: the value, from 1Password once, then held for a while.
        for _ in 0..3 {
            assert_eq!(
                vault.read("account/a").await.expect("read").expose(),
                "railway-token-from-1password"
            );
        }
        assert_eq!(reads.load(Ordering::SeqCst), 1);
        // A plain token is itself, and never goes to 1Password.
        assert_eq!(
            vault.read("account/b").await.expect("read").expose(),
            "plain-token"
        );
        assert_eq!(reads.load(Ordering::SeqCst), 1);
        // A change forgets what was held.
        vault
            .write("account/c", Secret::new("x".into()))
            .await
            .expect("write");
        vault.read("account/a").await.expect("read");
        assert_eq!(reads.load(Ordering::SeqCst), 2);
        // A reference that doesn't read is an error, not a token.
        assert!(
            vault
                .resolve(&Secret::new("op://Private/Missing/token".into()))
                .await
                .is_err()
        );
        assert!(
            vault
                .resolve(&Secret::new("op://not-a-reference".into()))
                .await
                .is_err()
        );
    }
}
