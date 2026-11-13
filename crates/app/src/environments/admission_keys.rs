//! The key that fingerprints the environment values in a deploy request, so
//! a retried request can be recognised without keeping the values. It's an
//! HMAC key kept in the system keychain; the database keeps only where it is
//! and a hash of it, never the values or plain hashes of them.

use crate::domain::{Secret, model::valid_env_name};
use crate::ports::{
    catalog::{CatalogRepository, EngineError, EnvironmentAdmissionKey},
    vault::TokenVault,
};
use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};
use std::time::Duration;

/// The key: 96 lowercase hex characters.
const KEY_CHARS: usize = 96;
const LOAD_TIMEOUT: Duration = Duration::from_secs(10);
const INSPECT_TIMEOUT: Duration = Duration::from_secs(3);
const MAX_METADATA_BYTES: usize = 16 * 1024;
const MAX_VALUES: usize = 64;
const MAX_VALUE_BYTES: usize = 16 * 1024;
const MAX_TOTAL_BYTES: usize = 64 * 1024;
/// Named in every fingerprint, so a change to how they're made changes them all.
const FINGERPRINT_FORMAT: &[u8] = b"hoista-environment-admission-v1\0";

fn unavailable() -> EngineError {
    EngineError::Provider {
        evidence: None,
        code: "keychain.admission_unavailable".into(),
        message: "The environment admission key is unavailable or changed. Run hoist doctor environment-key, then unlock or restore the original native keychain entry and retry with the same request key; do not clear local state to retry an uncertain operation.".into(),
    }
}

/// What the database keeps to recognise the key: its SHA-256.
fn commitment(key: &Secret) -> String {
    format!("{:x}", Sha256::digest(key.expose().as_bytes()))
}

fn valid_key(key: &Secret) -> bool {
    key.expose().len() == KEY_CHARS
        && key
            .expose()
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// The key at `record`'s reference, when it's still the one recorded.
async fn verified(
    vault: &dyn TokenVault,
    record: &EnvironmentAdmissionKey,
) -> Result<Secret, EngineError> {
    let key = vault
        .read(&record.reference)
        .await
        .map_err(|_| unavailable())?;
    if !valid_key(&key) || commitment(&key) != record.commitment {
        return Err(unavailable());
    }
    Ok(key)
}

/// The key, made the first time it's needed. Call only once the request
/// carrying the values is allowed. A key that's gone missing is never made
/// again: that would make earlier requests unrecognisable. A setup that's
/// cut short, or loses a race to another, stays recorded so it can be
/// looked at.
///
/// # Errors
/// Fails when the keychain can't be used or the key has changed, after too
/// many unfinished setups, or after ten seconds. Never writes the key to a
/// file and never replaces one.
pub async fn load(
    repository: &dyn CatalogRepository,
    vault: &dyn TokenVault,
) -> Result<Secret, EngineError> {
    tokio::time::timeout(LOAD_TIMEOUT, load_or_create(repository, vault))
        .await
        .map_err(|_| unavailable())?
}

async fn load_or_create(
    repository: &dyn CatalogRepository,
    vault: &dyn TokenVault,
) -> Result<Secret, EngineError> {
    if let Some(record) = repository.environment_admission_key().await? {
        return verified(vault, &record).await;
    }
    let reference = format!("environment-admission:{}", uuid::Uuid::new_v4());
    repository
        .reserve_environment_admission_key(&reference)
        .await?;
    let mut bytes = zeroize::Zeroizing::new([0u8; KEY_CHARS / 2]);
    ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), bytes.as_mut())
        .map_err(|_| unavailable())?;
    let key = Secret::new(bytes.iter().map(|byte| format!("{byte:02x}")).collect());
    let candidate = EnvironmentAdmissionKey {
        reference,
        commitment: commitment(&key),
    };
    vault
        .write(&candidate.reference, key)
        .await
        .map_err(|_| unavailable())?;
    let _readback = verified(vault, &candidate).await?;
    let winner = repository
        .activate_environment_admission_key(&candidate)
        .await?;
    verified(vault, &winner).await
}

/// HMAC-SHA256 of `metadata` and the values, sorted by name. Every part is
/// prefixed with its length, so moving bytes between parts, or an empty
/// value, always changes the result, and the order given doesn't matter.
/// `metadata` must cover the actor, request key, target, reviewed revision
/// and how the request was shown. Values go into the HMAC one at a time,
/// never joined into one buffer.
///
/// # Errors
/// Fails for a bad key, metadata over 16 KiB, no values, more than 64,
/// a bad or repeated name, or values over the size limits.
pub fn fingerprint(
    key: &Secret,
    metadata: &[u8],
    values: &[(String, Secret)],
) -> Result<String, EngineError> {
    if !valid_key(key)
        || metadata.len() > MAX_METADATA_BYTES
        || values.is_empty()
        || values.len() > MAX_VALUES
    {
        return Err(EngineError::Invalid("Invalid environment admission input."));
    }
    let mut ordered: Vec<_> = values.iter().collect();
    ordered.sort_by(|a, b| a.0.cmp(&b.0));
    let mut total = 0usize;
    for (index, (name, value)) in ordered.iter().enumerate() {
        total = total
            .checked_add(value.expose().len())
            .ok_or(EngineError::Store)?;
        if !valid_env_name(name)
            || value.expose().len() > MAX_VALUE_BYTES
            || total > MAX_TOTAL_BYTES
            || value.expose().contains('\0')
            || index > 0 && ordered[index - 1].0 == *name
        {
            return Err(EngineError::Invalid("Invalid environment admission input."));
        }
    }
    let mut mac =
        Hmac::<Sha256>::new_from_slice(key.expose().as_bytes()).map_err(|_| unavailable())?;
    mac.update(FINGERPRINT_FORMAT);
    mac.update(&(metadata.len() as u64).to_be_bytes());
    mac.update(metadata);
    mac.update(&(ordered.len() as u64).to_be_bytes());
    for (name, value) in ordered {
        mac.update(&(name.len() as u64).to_be_bytes());
        mac.update(name.as_bytes());
        mac.update(&(value.expose().len() as u64).to_be_bytes());
        mac.update(value.expose().as_bytes());
    }
    Ok(format!("{:x}", mac.finalize().into_bytes()))
}

/// Where the key stands, as read from the keychain just now.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    serde::Serialize,
    serde::Deserialize,
    schemars::JsonSchema,
    ts_rs::TS,
)]
#[serde(rename_all = "snake_case")]
pub enum EnvironmentKeyState {
    /// No key has been made yet.
    NotInitialized,
    /// The key is there and is the one recorded.
    Available,
    /// The keychain entry is gone.
    Missing,
    /// The keychain entry holds something other than the key recorded.
    Changed,
    /// The keychain couldn't be read, or didn't answer in time.
    Unavailable,
}

/// A report for the person on the key, to help get it back. Holds no key,
/// fingerprint or hash.
#[derive(Debug, serde::Serialize, serde::Deserialize, schemars::JsonSchema, ts_rs::TS)]
pub struct EnvironmentKeyRecovery {
    /// Where the key stood when checked. It can change after.
    pub state: EnvironmentKeyState,
    /// The keychain entry holding the key, to check or restore; null before setup.
    pub active_reference: Option<String>,
    /// Keychain entries of setups that didn't finish, up to sixteen.
    pub pending_references: Vec<String>,
    /// How many unfinished setups the person retired. Their entries are kept.
    pub retired_attempts: u32,
    /// Pass this to `--retry-setup`, so it only acts on what this report
    /// showed. Grants nothing on its own.
    pub review_token: String,
    /// Whether `--retry-setup` can be used: there are unfinished setups and no key.
    pub retry_setup_allowed: bool,
    /// What to do next.
    pub next_step: String,
}

/// Checks the key against the keychain, giving up after three seconds.
/// Never makes or changes a key. The caller makes sure it's the person.
///
/// # Errors
/// Fails when the key's records can't be read or don't add up. Keychain
/// problems show up in the report's state instead.
pub async fn inspect(
    repository: &dyn CatalogRepository,
    vault: &dyn TokenVault,
) -> Result<EnvironmentKeyRecovery, EngineError> {
    let snapshot = repository.environment_key_snapshot().await?;
    let state = match &snapshot.active {
        None => EnvironmentKeyState::NotInitialized,
        Some(record) => {
            match tokio::time::timeout(INSPECT_TIMEOUT, vault.read(&record.reference)).await {
                Ok(Ok(key)) if valid_key(&key) && commitment(&key) == record.commitment => {
                    EnvironmentKeyState::Available
                }
                Ok(Ok(_)) => EnvironmentKeyState::Changed,
                Ok(Err(EngineError::NotFound)) => EnvironmentKeyState::Missing,
                _ => EnvironmentKeyState::Unavailable,
            }
        }
    };
    let retry_setup_allowed = snapshot.active.is_none() && !snapshot.pending.is_empty();
    Ok(EnvironmentKeyRecovery {
        review_token: snapshot.review_token()?,
        next_step: next_step(state, retry_setup_allowed).into(),
        state,
        active_reference: snapshot.active.map(|key| key.reference),
        pending_references: snapshot.pending,
        retired_attempts: snapshot.retired,
        retry_setup_allowed,
    })
}

fn next_step(state: EnvironmentKeyState, retry_setup_allowed: bool) -> &'static str {
    match state {
        EnvironmentKeyState::NotInitialized if retry_setup_allowed => {
            "Unlock the native keychain, review the pending references, then run hoist doctor environment-key --retry-setup TOKEN with this report's token. Credentials remain retained and no deployment is retried."
        }
        EnvironmentKeyState::NotInitialized => {
            "The next keyed environment submission can initialize a native keychain key. No setup repair is needed."
        }
        EnvironmentKeyState::Available => {
            "The original key is available. Look up an uncertain deploy with hoist deploys request KEY before deciding whether to retry."
        }
        EnvironmentKeyState::Missing | EnvironmentKeyState::Changed => {
            "Restore the original credential at active_reference. Retry-setup cannot replace an active key. You can inspect accepted requests without that key using hoist deploys request KEY; do not clear local state or resubmit blindly."
        }
        EnvironmentKeyState::Unavailable => {
            "Unlock the native keychain and check access to active_reference, then inspect again. No key was changed; an unavailable check does not establish that the entry is missing."
        }
    }
}
