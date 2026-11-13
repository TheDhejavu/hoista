//! What's inside an encrypted transfer. It's decrypted before it's parsed,
//! and checked in full before anything is written to the keychain or the
//! database.

use crate::domain::{
    Profile, Secret,
    backup::{Backup, MAX_ACCOUNTS, MAX_PROJECTS},
    map_layout::MapLayout,
};
use crate::{data::backup::validate_backup, ports::catalog::EngineError};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashSet},
    io::Write,
};
use zeroize::Zeroizing;

/// The transfer format this build writes and reads. Separate from the
/// encryption's own version.
pub const TRANSFER_VERSION: u32 = 1;
/// The largest transfer accepted, however it arrives.
pub const MAX_TRANSFER_PAYLOAD: usize = 16 * 1024 * 1024;
/// The most activity a transfer carries; older entries are left out.
pub const MAX_ACTIVITY: usize = 10_000;

const MAX_ID_BYTES: usize = 128;
const MAX_SUMMARY_BYTES: usize = 2048;
const MAX_TOKEN_BYTES: usize = 64 * 1024;

/// The person's name and initials, checked as in first-run setup.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransferProfile {
    /// The name.
    pub name: String,
    /// The initials.
    pub initials: String,
}

/// One past activity entry. Bringing it over grants nothing and restarts
/// nothing.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransferEvent {
    /// Its number on the other computer. Only used to keep the order.
    pub source_id: i64,
    /// When it happened (RFC 3339).
    pub at: String,
    /// Who did it. Their access doesn't come with it.
    pub actor: String,
    /// The project it was about, which may since have been removed.
    pub project_id: Option<String>,
    /// What kind of entry it is, such as `deploy.finished`.
    pub kind: String,
    /// What happened, as it was recorded.
    pub summary: String,
}

/// An account's token. It only ever travels inside the encrypted transfer,
/// and deliberately has no `Debug`.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransferCredential {
    /// The account's id. Its token goes into a new keychain entry here.
    pub account_id: String,
    /// The token, in memory that's wiped after use. Never sent from the API.
    #[serde(
        serialize_with = "serialize_secret",
        deserialize_with = "deserialize_secret"
    )]
    pub token: Secret,
}

fn serialize_secret<S: serde::Serializer>(
    value: &Secret,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(value.expose())
}

/// Reads a secret written by `serialize_secret`.
///
/// # Errors
/// Fails when the value isn't a string.
pub fn deserialize_secret<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Secret, D::Error> {
    String::deserialize(deserializer).map(Secret::new)
}

/// Everything a transfer carries. Agent grants, project secrets, admission
/// keys and anything that would run on restore are never in it.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransferPayload {
    /// The format, [`TRANSFER_VERSION`].
    pub version: u32,
    /// When it was made (RFC 3339).
    pub created_at: String,
    /// Accounts, projects, resources, links, custom domains and settings.
    pub catalog: Backup,
    /// The person's profile, when they've set one.
    pub profile: Option<TransferProfile>,
    /// Each project's map layout.
    pub map_layouts: BTreeMap<String, MapLayout>,
    /// Unassigned resources the person chose to ignore.
    pub ignored_resources: Vec<String>,
    /// Up to [`MAX_ACTIVITY`] activity entries, oldest first.
    pub activity: Vec<TransferEvent>,
    /// Whether older activity was left out.
    pub activity_truncated: bool,
    /// Tokens for the accounts in `catalog`.
    pub credentials: Vec<TransferCredential>,
}

fn invalid() -> EngineError {
    EngineError::Invalid("The transfer payload has an unsupported format or invalid relationships.")
}

/// Not empty, not too long, and no control characters.
fn plain_text(value: &str, max_bytes: usize) -> bool {
    !value.is_empty() && value.len() <= max_bytes && !value.chars().any(char::is_control)
}

impl TransferPayload {
    /// Parses decrypted bytes, checking everything before returning.
    ///
    /// # Errors
    /// Fails for too much data, invalid JSON, unknown fields or versions, or
    /// anything [`Self::validate`] refuses.
    pub fn decode(bytes: &[u8]) -> Result<Self, EngineError> {
        if bytes.len() > MAX_TRANSFER_PAYLOAD {
            return Err(invalid());
        }
        let payload: Self = serde_json::from_slice(bytes).map_err(|_| invalid())?;
        payload.validate()?;
        Ok(payload)
    }

    /// Writes the payload as JSON straight into memory that's wiped when
    /// dropped, stopping at the size limit.
    ///
    /// # Errors
    /// Fails for a payload [`Self::validate`] refuses, or one over 16 MiB.
    pub fn encode(&self) -> Result<Zeroizing<Vec<u8>>, EngineError> {
        self.validate()?;
        let mut writer = PlaintextWriter(Zeroizing::new(Vec::new()));
        serde_json::to_writer(&mut writer, self).map_err(|_| invalid())?;
        Ok(writer.0)
    }

    /// Checks the payload hangs together. Changes nothing and calls no provider.
    ///
    /// # Errors
    /// Fails for a wrong version or date, too many entries, bad or repeated
    /// ids, references to things that aren't there, bad tokens or layouts,
    /// and activity out of order.
    pub fn validate(&self) -> Result<(), EngineError> {
        if self.version != TRANSFER_VERSION
            || chrono::DateTime::parse_from_rfc3339(&self.created_at).is_err()
            || self.credentials.len() > MAX_ACCOUNTS
            || self.activity.len() > MAX_ACTIVITY
        {
            return Err(invalid());
        }
        validate_backup(self.catalog.clone())?;
        let catalog = &self.catalog;
        for ids in [
            catalog
                .accounts
                .iter()
                .map(|v| v.id.as_str())
                .collect::<Vec<_>>(),
            catalog.projects.iter().map(|v| v.id.as_str()).collect(),
            catalog.resources.iter().map(|v| v.id.as_str()).collect(),
            catalog.links.iter().map(|v| v.id.as_str()).collect(),
            catalog
                .custom_domains
                .iter()
                .map(|v| v.id.as_str())
                .collect(),
        ] {
            let mut unique = HashSet::new();
            if ids
                .into_iter()
                .any(|id| !plain_text(id, MAX_ID_BYTES) || !unique.insert(id))
            {
                return Err(invalid());
            }
        }
        let mut hostnames = HashSet::new();
        if catalog
            .custom_domains
            .iter()
            .any(|v| !hostnames.insert(v.hostname.as_str()))
        {
            return Err(invalid());
        }
        if let Some(profile) = &self.profile {
            Profile::new(profile.name.clone(), profile.initials.clone()).map_err(|_| invalid())?;
        }
        let accounts: HashSet<_> = catalog.accounts.iter().map(|a| a.id.as_str()).collect();
        let projects: HashSet<_> = catalog.projects.iter().map(|p| p.id.as_str()).collect();
        let mut credentials = HashSet::new();
        for credential in &self.credentials {
            let token = credential.token.expose();
            if !accounts.contains(credential.account_id.as_str())
                || !credentials.insert(&credential.account_id)
                || token.is_empty()
                || token.len() > MAX_TOKEN_BYTES
                || token.contains('\0')
            {
                return Err(invalid());
            }
        }
        if self.map_layouts.len() > MAX_PROJECTS
            || self
                .map_layouts
                .iter()
                .any(|(id, layout)| !projects.contains(id.as_str()) || !layout.valid())
        {
            return Err(invalid());
        }
        let unassigned: HashSet<_> = catalog
            .resources
            .iter()
            .filter(|r| r.project_id.is_none())
            .map(|r| r.id.as_str())
            .collect();
        let ignored: HashSet<_> = self.ignored_resources.iter().map(String::as_str).collect();
        if ignored.len() != self.ignored_resources.len() || !ignored.is_subset(&unassigned) {
            return Err(invalid());
        }
        let mut previous = 0;
        for event in &self.activity {
            if event.source_id <= previous
                || chrono::DateTime::parse_from_rfc3339(&event.at).is_err()
                || !plain_text(&event.actor, MAX_ID_BYTES)
                || !plain_text(&event.kind, MAX_ID_BYTES)
                || !plain_text(&event.summary, MAX_SUMMARY_BYTES)
                || event
                    .project_id
                    .as_ref()
                    .is_some_and(|id| !plain_text(id, MAX_ID_BYTES))
            {
                return Err(invalid());
            }
            previous = event.source_id;
        }
        Ok(())
    }
}

/// Collects the JSON, refusing to grow past the payload limit.
struct PlaintextWriter(Zeroizing<Vec<u8>>);

impl Write for PlaintextWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if bytes.len() > MAX_TRANSFER_PAYLOAD.saturating_sub(self.0.len()) {
            return Err(std::io::Error::other(
                "Transfer payload exceeds its size limit.",
            ));
        }
        self.0
            .try_reserve(bytes.len())
            .map_err(|_| std::io::Error::other("Cannot allocate transfer payload."))?;
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// The keychain entries one restore made, so an abandoned restore can be
/// cleaned up. No tokens.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, schemars::JsonSchema, ts_rs::TS)]
#[serde(deny_unknown_fields)]
pub struct TransferRestoreRecord {
    /// The restore's id. Never reused.
    pub id: String,
    /// When it started.
    pub created_at: String,
    /// The keychain entries it writes, chosen before any is written.
    pub credential_references: Vec<String>,
}

/// Where a restore got to. Abandoned ones are kept until the person removes
/// them.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema, ts_rs::TS)]
pub struct TransferRestoreStatus {
    /// The restore and its keychain entries.
    pub record: TransferRestoreRecord,
    /// `staging`, `installed` or `abandoned`.
    pub status: String,
}
