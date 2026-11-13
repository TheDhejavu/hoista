//! What Hoista does: connecting accounts, importing, planning and deploying,
//! environments, logs, agents, webhooks and backups, and the person's
//! profile. Written against traits (providers, store, vault) that the other
//! crates implement; it never uses a database, HTTP library or provider API
//! directly.

/// Hoista's core types and rules, from the `hoista-domain` crate.
use crate::domain::{Profile, Secret};
pub use hoista_domain as domain;
pub use hoista_domain::{Provider, UnknownProvider};
use std::future::Future;

/// A SHA-256 written out in hex.
const SHA256_HEX_CHARS: usize = 64;

/// Version of Hoista, as published in its manifest.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The published `hoistd` image this release puts on a server, pinned by
/// digest (`registry/name@sha256:…`) and signed. Set by the release build;
/// absent in development builds, which build a server from a repository.
pub const RELEASE_IMAGE: Option<&str> = option_env!("HOISTA_RELEASE_IMAGE");

/// Whether `image` names one exact image: `registry/name@sha256:` and 64 hex digits.
#[must_use]
pub fn is_pinned_image(image: &str) -> bool {
    image.split_once("@sha256:").is_some_and(|(name, digest)| {
        !name.is_empty()
            && name.contains('/')
            && name
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"-_./:".contains(&b))
            && digest.len() == SHA256_HEX_CHARS
            && digest
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    })
}

/// How many events one page of the activity list holds.
pub const EVENT_PAGE_SIZE: usize = 50;

/// Why saved state couldn't be used. Never carries what's in the database or a token.
#[derive(Debug, thiserror::Error)]
pub enum StateError {
    /// The database couldn't be read or written.
    #[error("Could not access local state. Run hoist doctor and try again.")]
    Unavailable,
    /// What's saved breaks Hoista's own rules.
    #[error("Local state is invalid. Restore a backup before continuing.")]
    Invalid,
}

/// One entry in the activity log.
#[derive(Debug, Clone)]
pub struct Event {
    /// The project it's about. Kept after the project is removed.
    pub project_id: Option<String>,
    /// The record the event is about, such as a deployment or approval id.
    /// Null when it isn't about one record.
    pub subject_id: Option<String>,
    /// Its number. Later entries have higher numbers.
    pub id: i64,
    /// RFC 3339 UTC timestamp.
    pub at: String,
    /// The person or agent who did it.
    pub actor: String,
    /// What kind of entry it is, such as `deploy.finished`.
    pub kind: String,
    /// What happened, in plain words, without any values given.
    pub summary: String,
}

/// Where the profile and the activity log are saved.
pub trait StateStore: Clone + Send + Sync + 'static {
    /// Reads the profile, or None before first-run setup.
    ///
    /// # Errors
    /// Returns an error when state cannot be read or validated.
    fn profile(&self) -> impl Future<Output = Result<Option<Profile>, StateError>> + Send;

    /// Atomically saves the profile and appends its activity event.
    ///
    /// # Errors
    /// Returns an error if either effect cannot be committed.
    fn save_profile(
        &self,
        profile: &Profile,
    ) -> impl Future<Output = Result<(), StateError>> + Send;

    /// Reads at most 100 events strictly after the supplied cursor.
    ///
    /// # Errors
    /// Returns an error if state cannot be read.
    fn events_after(
        &self,
        cursor: i64,
    ) -> impl Future<Output = Result<Vec<Event>, StateError>> + Send;

    /// The newest committed event cursor, or 0 when there are no events, so a
    /// client can follow only what happens from now on.
    ///
    /// # Errors
    /// Returns an error if state cannot be read.
    fn latest_event_id(&self) -> impl Future<Output = Result<i64, StateError>> + Send;

    /// Reads at most `EVENT_PAGE_SIZE` events strictly before `before` (or the newest when
    /// None), newest first, optionally only one actor's or one project's.
    ///
    /// # Errors
    /// Returns an error if state cannot be read.
    fn events_before(
        &self,
        before: Option<i64>,
        actor: Option<&str>,
        project_id: Option<&str>,
        kind: Option<&str>,
    ) -> impl Future<Output = Result<Vec<Event>, StateError>> + Send;
}

/// Credential access failure without sensitive operating-system error details.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum CredentialError {
    /// No entry exists at the requested reference.
    #[error("Credential was not found. Connect the account again.")]
    Missing,
    /// The system keychain is locked, unavailable or denied access.
    #[error("Cannot access the system keychain. Unlock it and allow Hoista access.")]
    Unavailable,
    /// A keychain write could not be read back unchanged.
    #[error("Keychain verification failed. Check system credential storage.")]
    Verification,
}

/// The system's credential store. Calls block, so run them off the async runtime.
pub trait CredentialStore {
    /// Reads the entry named `reference`.
    ///
    /// # Errors
    /// Reports missing entries and denied or unavailable keychain access.
    fn get(&self, reference: &str) -> Result<Secret, CredentialError>;

    /// Writes a value directly to the OS credential store.
    ///
    /// # Errors
    /// Reports denied or unavailable keychain access.
    fn set(&self, reference: &str, secret: &Secret) -> Result<(), CredentialError>;

    /// Removes an entry; removing a missing entry succeeds.
    ///
    /// # Errors
    /// Reports denied or unavailable keychain access.
    fn delete(&self, reference: &str) -> Result<(), CredentialError>;
}

/// Checks write/read/delete access using a unique temporary keychain entry.
///
/// # Errors
/// Reports unavailable access, changed values or failed cleanup. No account entries are touched.
pub fn check_keychain(store: &impl CredentialStore) -> Result<(), CredentialError> {
    let reference = format!("doctor/{}", uuid::Uuid::new_v4());
    let probe = Secret::new(uuid::Uuid::new_v4().to_string());
    store.set(&reference, &probe)?;
    let result = store.get(&reference).and_then(|value| {
        if value.expose() == probe.expose() {
            Ok(())
        } else {
            Err(CredentialError::Verification)
        }
    });
    store.delete(&reference)?;
    result
}

pub mod engine;

/// What the use cases need from outside, as traits.
pub mod ports;

/// Provider accounts.
pub mod accounts;
/// Agents and their access.
pub mod agents;
/// Backups, database exports and transfers.
pub mod data;
/// Plans and deploys.
pub mod deploys;
/// Environments, variables, secrets and domains.
pub mod environments;
/// Logs, diagnostics and the operation journal.
pub mod observe;
/// Projects and their resources.
pub mod projects;
/// Hoista's own settings.
pub(crate) mod settings;
/// Webhooks.
pub mod webhooks;
