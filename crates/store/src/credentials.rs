//! Credentials in the native system keychain, with no fallback to disk.
//!
//! On macOS entries go through `/usr/bin/security` so that updating or
//! rebuilding Hoista doesn't make the keychain prompt again.

use crate::{
    application::{CredentialError, CredentialStore, ports::catalog::EngineError},
    domain::Secret,
};
use std::{
    collections::{BTreeMap, HashMap},
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

/// Keychain calls allowed at once; more wait their turn.
const MAX_KEYCHAIN_CALLS: usize = 4;
/// How long reads pause after the keychain first refuses, doubling with each
/// refusal in a row up to [`LONGEST_PAUSE`].
const FIRST_PAUSE: Duration = Duration::from_secs(30);
const LONGEST_PAUSE: Duration = Duration::from_secs(5 * 60);

/// Uses macOS Keychain (through `security`), Windows Credential Manager or
/// Linux Secret Service.
#[derive(Clone, Copy, Debug)]
pub struct SystemKeychain;

/// How values travel through `security` on macOS.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
mod codec {
    /// Marks values Hoista wrote, so hex output from `security` is never
    /// mistaken for a value that happens to look like hex.
    pub(super) const PREFIX: &str = "hoista1:";

    pub(super) fn hex(bytes: &[u8]) -> String {
        use std::fmt::Write;
        bytes
            .iter()
            .fold(String::with_capacity(bytes.len() * 2), |mut out, byte| {
                let _ = write!(out, "{byte:02x}");
                out
            })
    }

    fn unhex(text: &str) -> Option<Vec<u8>> {
        if !text.len().is_multiple_of(2) {
            return None;
        }
        (0..text.len())
            .step_by(2)
            .map(|at| u8::from_str_radix(text.get(at..at + 2)?, 16).ok())
            .collect()
    }

    /// `security -w` prints the value as text, or as hex when it isn't
    /// plain ASCII. Either way it starts with the prefix once decoded.
    pub(super) fn decode(printed: &str) -> Option<String> {
        let printed = printed.trim_end_matches('\n');
        if let Some(value) = printed.strip_prefix(PREFIX) {
            return Some(value.to_owned());
        }
        let bytes = unhex(printed)?;
        String::from_utf8(bytes)
            .ok()?
            .strip_prefix(PREFIX)
            .map(str::to_owned)
    }

    #[cfg(test)]
    mod tests {
        use super::{PREFIX, decode, hex};

        #[test]
        fn values_come_back_as_text_or_hex() {
            assert_eq!(
                decode("hoista1:{\"a\":\"b\"}\n").as_deref(),
                Some("{\"a\":\"b\"}")
            );
            let printed = hex(format!("{PREFIX}café").as_bytes());
            assert_eq!(decode(&printed).as_deref(), Some("café"));
            assert_eq!(decode("deadbeef"), None);
            assert_eq!(decode("abc"), None);
        }
    }
}

/// Values are kept in pieces: the value under `<entry>.part-<generation>-<n>`
/// (each piece short enough for one `security -i` command line), and the
/// entry itself holding only which pieces make the value and their digest.
/// New pieces are written and read back before the entry points at them, so a
/// failed or cut-short write never damages the value already there, and a
/// reader never sees a half-written one.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
mod pieces {
    use super::CredentialError;
    use sha2::{Digest, Sha256};

    /// Longest piece kept in one entry, in bytes; about 2 KB once
    /// hex-encoded with the command around it.
    pub(super) const PIECE: usize = 1000;
    /// Starts an entry that points at pieces; no value Hoista keeps starts so.
    const MARK: &str = "\u{1}parts:";
    /// Characters of a random id that tell one write's pieces from the next.
    pub(super) const GENERATION_CHARS: usize = 8;

    /// One keychain entry per call.
    pub(super) trait Entries {
        fn get(&mut self, account: &str) -> Result<String, CredentialError>;

        fn set(&mut self, account: &str, text: &str) -> Result<(), CredentialError>;

        fn delete(&mut self, account: &str) -> Result<(), CredentialError>;
    }

    fn digest(text: &str) -> String {
        super::codec::hex(&Sha256::digest(text.as_bytes()))
    }

    fn name(reference: &str, generation: &str, index: usize) -> String {
        format!("{reference}.part-{generation}-{index}")
    }

    /// `(generation, count, digest)` when the entry points at pieces.
    fn pointer(text: &str) -> Option<(String, usize, String)> {
        let mut fields = text.strip_prefix(MARK)?.splitn(3, ':');
        let generation = fields.next()?.to_owned();
        let count = fields.next()?.parse().ok()?;
        let digest = fields.next()?.to_owned();
        Some((generation, count, digest))
    }

    /// Splits at character boundaries into pieces of at most `PIECE` bytes.
    fn pieces(text: &str) -> Vec<&str> {
        let mut out = Vec::new();
        let mut rest = text;
        while !rest.is_empty() {
            let mut end = rest.len().min(PIECE);
            while !rest.is_char_boundary(end) {
                end -= 1;
            }
            out.push(&rest[..end]);
            rest = &rest[end..];
        }
        out
    }

    pub(super) fn read(
        entries: &mut impl Entries,
        reference: &str,
    ) -> Result<String, CredentialError> {
        let head = entries.get(reference)?;
        let (generation, count, expected) = pointer(&head).ok_or(CredentialError::Verification)?;
        let mut value = String::new();
        for index in 0..count {
            value.push_str(&entries.get(&name(reference, &generation, index))?);
        }
        if digest(&value) == expected {
            Ok(value)
        } else {
            Err(CredentialError::Verification)
        }
    }

    pub(super) fn write(
        entries: &mut impl Entries,
        reference: &str,
        text: &str,
        generation: &str,
    ) -> Result<(), CredentialError> {
        let before = match entries.get(reference) {
            Ok(head) => pointer(&head),
            Err(CredentialError::Missing) => None,
            Err(error) => return Err(error),
        };
        let pieces = pieces(text);
        for (index, piece) in pieces.iter().enumerate() {
            if let Err(error) = entries.set(&name(reference, generation, index), piece) {
                // Nothing points at these yet; the value already there is untouched.
                for written in 0..=index {
                    let _ = entries.delete(&name(reference, generation, written));
                }
                return Err(error);
            }
        }
        entries.set(
            reference,
            &format!("{MARK}{generation}:{}:{}", pieces.len(), digest(text)),
        )?;
        // The old pieces are no longer pointed at.
        if let Some((old, count, _)) = before
            && old != generation
        {
            for index in 0..count {
                let _ = entries.delete(&name(reference, &old, index));
            }
        }
        Ok(())
    }

    pub(super) fn remove(
        entries: &mut impl Entries,
        reference: &str,
    ) -> Result<(), CredentialError> {
        let before = match entries.get(reference) {
            Ok(head) => pointer(&head),
            Err(CredentialError::Missing) => return Ok(()),
            Err(error) => return Err(error),
        };
        entries.delete(reference)?;
        if let Some((generation, count, _)) = before {
            for index in 0..count {
                let _ = entries.delete(&name(reference, &generation, index));
            }
        }
        Ok(())
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::collections::BTreeMap;

        #[derive(Default)]
        struct Memory(BTreeMap<String, String>);
        impl Entries for Memory {
            fn get(&mut self, account: &str) -> Result<String, CredentialError> {
                self.0.get(account).cloned().ok_or(CredentialError::Missing)
            }

            fn set(&mut self, account: &str, text: &str) -> Result<(), CredentialError> {
                assert!(
                    text.len() <= PIECE + 200,
                    "one entry stays under the line limit"
                );
                self.0.insert(account.into(), text.into());
                Ok(())
            }

            fn delete(&mut self, account: &str) -> Result<(), CredentialError> {
                self.0.remove(account);
                Ok(())
            }
        }

        #[test]
        fn long_values_are_split_and_put_back_together() {
            let mut entries = Memory::default();
            write(&mut entries, "store/engine", "short", "a").unwrap();
            assert_eq!(read(&mut entries, "store/engine").unwrap(), "short");
            assert_eq!(entries.0.len(), 2);
            let long: String = "é{token}".repeat(900);
            write(&mut entries, "store/engine", &long, "b").unwrap();
            assert_eq!(read(&mut entries, "store/engine").unwrap(), long);
            let first = entries.0.len();
            assert!(first > 2);
            // A new long value replaces the old pieces.
            let other: String = "x".repeat(4000);
            write(&mut entries, "store/engine", &other, "c").unwrap();
            assert_eq!(read(&mut entries, "store/engine").unwrap(), other);
            assert!(entries.0.keys().all(|key| !key.contains("part-b-")));
            // A changed piece is caught.
            let piece = entries
                .0
                .keys()
                .find(|key| key.contains("part-c-0"))
                .unwrap()
                .clone();
            entries.0.insert(piece, "tampered".into());
            assert!(read(&mut entries, "store/engine").is_err());
            remove(&mut entries, "store/engine").unwrap();
            assert!(entries.0.is_empty());
            // An entry that doesn't point at pieces wasn't written by Hoista.
            entries.0.insert("store/engine".into(), "plain".into());
            assert!(read(&mut entries, "store/engine").is_err());
        }

        /// What's already in people's keychains: this exact layout must keep
        /// reading, whatever the code around it is renamed to.
        #[test]
        fn the_stored_layout_never_changes() {
            let mut entries = Memory::default();
            let value = "{\"account/1\":\"token\"}";
            entries.0.insert(
                "store/engine".into(),
                format!("\u{1}parts:g1:1:{}", digest(value)),
            );
            entries
                .0
                .insert("store/engine.part-g1-0".into(), value.into());
            assert_eq!(read(&mut entries, "store/engine").unwrap(), value);
        }
    }
}

#[cfg(target_os = "macos")]
mod store {
    use super::codec::{PREFIX, decode, hex};
    use super::{CredentialError, Secret};
    use std::process::{Command, Stdio};

    /// Keychain Access shows entries under this name.
    const SERVICE: &str = "Hoista";
    /// `security` exits with this when the entry doesn't exist.
    const NOT_FOUND: i32 = 44;
    const MAX_REFERENCE_BYTES: usize = 200;

    fn security(args: &[&str]) -> Result<std::process::Output, CredentialError> {
        Command::new("/usr/bin/security")
            .args(args)
            .stdin(Stdio::null())
            .output()
            .map_err(|_| CredentialError::Unavailable)
    }

    fn missing_or_unavailable(output: &std::process::Output) -> CredentialError {
        if output.status.code() == Some(NOT_FOUND) {
            CredentialError::Missing
        } else {
            CredentialError::Unavailable
        }
    }

    fn get_entry(reference: &str) -> Result<Secret, CredentialError> {
        let output = security(&[
            "find-generic-password",
            "-s",
            SERVICE,
            "-a",
            reference,
            "-w",
        ])?;
        if !output.status.success() {
            return Err(missing_or_unavailable(&output));
        }
        let printed = String::from_utf8(output.stdout).map_err(|_| CredentialError::Unavailable)?;
        // Present but not something Hoista wrote whole: say so, not "locked".
        decode(&printed)
            .map(Secret::new)
            .ok_or(CredentialError::Verification)
    }

    /// Entry names are Hoista's own (`store/engine`, `agent/<uuid>`); anything
    /// else could change the command `security` reads.
    fn plain(reference: &str) -> bool {
        !reference.is_empty()
            && reference.len() <= MAX_REFERENCE_BYTES
            && reference
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"/_.:-".contains(&byte))
    }

    fn set_entry(reference: &str, secret: &Secret) -> Result<(), CredentialError> {
        use std::io::Write as _;
        if !plain(reference) {
            return Err(CredentialError::Unavailable);
        }
        // The value goes to `security` on its input, never in its arguments,
        // so it never shows in a list of running programs. Hex so any value
        // survives the command line `security -i` reads.
        let value = hex(format!("{PREFIX}{}", secret.expose()).as_bytes());
        let mut child = Command::new("/usr/bin/security")
            .arg("-i")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|_| CredentialError::Unavailable)?;
        let line = format!(
            "add-generic-password -U -s \"{SERVICE}\" -a \"{reference}\" -l \"{SERVICE}\" -X \"{value}\"\n"
        );
        let written = child
            .stdin
            .take()
            .ok_or(CredentialError::Unavailable)
            .and_then(|mut input| {
                input
                    .write_all(line.as_bytes())
                    .map_err(|_| CredentialError::Unavailable)
            });
        drop(line);
        let status = child.wait().map_err(|_| CredentialError::Unavailable)?;
        written?;
        // `security -i` doesn't report a failed command in its exit status,
        // so read the entry back.
        if status.success() && get_entry(reference)?.expose() == secret.expose() {
            Ok(())
        } else {
            Err(CredentialError::Unavailable)
        }
    }

    fn delete_entry(reference: &str) -> Result<(), CredentialError> {
        let output = security(&["delete-generic-password", "-s", SERVICE, "-a", reference])?;
        if output.status.success() || output.status.code() == Some(NOT_FOUND) {
            Ok(())
        } else {
            Err(CredentialError::Unavailable)
        }
    }

    struct Security;

    impl super::pieces::Entries for Security {
        fn get(&mut self, account: &str) -> Result<String, CredentialError> {
            get_entry(account).map(|secret| secret.expose().to_owned())
        }

        fn set(&mut self, account: &str, text: &str) -> Result<(), CredentialError> {
            set_entry(account, &Secret::new(text.to_owned()))
        }

        fn delete(&mut self, account: &str) -> Result<(), CredentialError> {
            delete_entry(account)
        }
    }

    pub(super) fn get(reference: &str) -> Result<Secret, CredentialError> {
        super::pieces::read(&mut Security, reference).map(Secret::new)
    }

    pub(super) fn set(reference: &str, secret: &Secret) -> Result<(), CredentialError> {
        let generation = uuid::Uuid::new_v4().simple().to_string();
        let generation = &generation[..super::pieces::GENERATION_CHARS];
        super::pieces::write(&mut Security, reference, secret.expose(), generation)
    }

    pub(super) fn delete(reference: &str) -> Result<(), CredentialError> {
        super::pieces::remove(&mut Security, reference)
    }
}

#[cfg(not(target_os = "macos"))]
mod store {
    use super::{CredentialError, Secret};

    /// The service name entries are kept under.
    const SERVICE: &str = "ai.hoista";

    fn entry(reference: &str) -> Result<keyring::Entry, CredentialError> {
        keyring::Entry::new(SERVICE, reference).map_err(|_| CredentialError::Unavailable)
    }

    fn failure(error: keyring::Error) -> CredentialError {
        match error {
            keyring::Error::NoEntry => CredentialError::Missing,
            _ => CredentialError::Unavailable,
        }
    }

    pub(super) fn get(reference: &str) -> Result<Secret, CredentialError> {
        entry(reference)?
            .get_password()
            .map(Secret::new)
            .map_err(failure)
    }

    pub(super) fn set(reference: &str, secret: &Secret) -> Result<(), CredentialError> {
        entry(reference)?
            .set_password(secret.expose())
            .map_err(failure)
    }

    pub(super) fn delete(reference: &str) -> Result<(), CredentialError> {
        match entry(reference)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(failure(error)),
        }
    }
}

impl CredentialStore for SystemKeychain {
    fn get(&self, reference: &str) -> Result<Secret, CredentialError> {
        store::get(reference)
    }

    fn set(&self, reference: &str, secret: &Secret) -> Result<(), CredentialError> {
        store::set(reference, secret)
    }

    fn delete(&self, reference: &str) -> Result<(), CredentialError> {
        store::delete(reference)
    }
}

/// Behaves like a machine with no keychain: every call reports it unavailable.
/// Selected by setting `HOISTA_NO_KEYCHAIN`, which the tests use so a saved
/// credential on the machine running them can't change what they check.
#[derive(Clone, Copy, Debug)]
struct NoKeychain;

impl CredentialStore for NoKeychain {
    fn get(&self, _: &str) -> Result<Secret, CredentialError> {
        Err(CredentialError::Unavailable)
    }

    fn set(&self, _: &str, _: &Secret) -> Result<(), CredentialError> {
        Err(CredentialError::Unavailable)
    }

    fn delete(&self, _: &str) -> Result<(), CredentialError> {
        Err(CredentialError::Unavailable)
    }
}

/// Provider tokens kept in the system keychain, for the engine to use.
///
/// Everything one program keeps lives in a single keychain entry (a JSON map
/// from reference to value), not one entry per credential, so each run makes
/// one keychain call to load. Values are never written to a file instead.
///
/// The engine and the coding-tool bridge are separate programs, so each uses
/// its own entry and only touches its own. Development builds use entries of
/// their own, so a release never inherits what a development build stored.
///
/// The entry is read once per run and kept in memory (zeroized when dropped).
/// Writes and deletes re-read it first, so changes made by another run of the
/// same program are kept, then write it back.
#[derive(Clone)]
pub struct NativeVault {
    /// Keychain account name of this program's single entry.
    entry: String,
    store: Arc<dyn CredentialStore + Send + Sync>,
    /// Development builds only: where each keychain call is noted.
    trace: Option<PathBuf>,
    permits: Arc<tokio::sync::Semaphore>,
    /// The entry's contents once read this run.
    cache: Arc<tokio::sync::Mutex<Option<Bundle>>>,
    /// After the keychain refuses a read, background reads wait instead of
    /// asking again at once, so one refusal never turns into a prompt storm.
    pause: Arc<std::sync::Mutex<Pause>>,
}

#[derive(Default)]
struct Pause {
    until: Option<Instant>,
    refusals: u32,
}

impl Pause {
    fn refused(&mut self) {
        let wait = FIRST_PAUSE
            .saturating_mul(1 << self.refusals.min(4))
            .min(LONGEST_PAUSE);
        self.refusals = self.refusals.saturating_add(1);
        self.until = Some(Instant::now() + wait);
    }
}

type Bundle = HashMap<String, Secret>;

impl Default for NativeVault {
    fn default() -> Self {
        Self::named("engine")
    }
}

impl NativeVault {
    /// A vault in the single entry for `name` (the engine, or connections
    /// kept by the coding-tool bridge).
    #[must_use]
    pub fn named(name: &str) -> Self {
        if std::env::var_os("HOISTA_NO_KEYCHAIN").is_some() {
            return Self::in_store(name, Arc::new(NoKeychain));
        }
        Self::in_store(name, Arc::new(SystemKeychain))
    }

    /// The same vault over another credential store (tests use a fake one).
    #[must_use]
    pub fn in_store(name: &str, store: Arc<dyn CredentialStore + Send + Sync>) -> Self {
        let build = if cfg!(debug_assertions) { "-dev" } else { "" };
        Self {
            entry: format!("store/{name}{build}"),
            store,
            trace: None,
            pause: Arc::default(),
            permits: Arc::new(tokio::sync::Semaphore::new(MAX_KEYCHAIN_CALLS)),
            cache: Arc::default(),
        }
    }

    /// Development builds note every keychain call (program, call,
    /// entry name and outcome, never a value) in `keychain.log` in this
    /// folder, to find what makes macOS ask for a password.
    #[must_use]
    pub fn traced(mut self, folder: &Path) -> Self {
        if cfg!(debug_assertions) {
            self.trace = Some(folder.join("keychain.log"));
        }
        self
    }

    fn note<T>(&self, call: &str, entry: &str, outcome: &Result<T, CredentialError>) {
        let Some(path) = &self.trace else { return };
        let program = std::env::current_exe()
            .ok()
            .and_then(|path| {
                path.file_name()
                    .map(|name| name.to_string_lossy().into_owned())
            })
            .unwrap_or_default();
        let result = match outcome {
            Ok(_) => "ok",
            Err(CredentialError::Missing) => "missing",
            Err(CredentialError::Unavailable) => "denied or unavailable",
            Err(CredentialError::Verification) => "unreadable",
        };
        let line = format!(
            "{} {program}[{}] {call} {entry}: {result}\n",
            chrono::Utc::now().format("%H:%M:%S%.3f"),
            std::process::id()
        );
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
        {
            let _ = std::io::Write::write_all(&mut file, line.as_bytes());
        }
    }

    async fn blocking<T: Send + 'static>(
        &self,
        work: impl FnOnce(&dyn CredentialStore) -> Result<T, CredentialError> + Send + 'static,
    ) -> Result<T, CredentialError> {
        let permit = self
            .permits
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| CredentialError::Unavailable)?;
        let store = self.store.clone();
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            work(store.as_ref())
        })
        .await
        .map_err(|_| CredentialError::Unavailable)?
    }

    /// Reads this program's entry from the keychain; missing is empty.
    async fn load(&self) -> Result<Bundle, CredentialError> {
        let entry = self.entry.clone();
        let outcome = self.blocking(move |store| store.get(&entry)).await;
        self.note("read", &self.entry, &outcome);
        let stored = match outcome {
            Ok(stored) => stored,
            Err(CredentialError::Missing) => return Ok(Bundle::new()),
            Err(error) => return Err(error),
        };
        let parsed: HashMap<String, String> =
            serde_json::from_str(stored.expose()).map_err(|_| CredentialError::Verification)?;
        Ok(parsed
            .into_iter()
            .map(|(key, value)| (key, Secret::new(value)))
            .collect())
    }

    /// Reads the entry unless the keychain refused recently. A refusal pauses
    /// reads; a successful read ends the pause.
    async fn load_unless_paused(&self) -> Result<Bundle, CredentialError> {
        let paused = self
            .pause
            .lock()
            .ok()
            .and_then(|pause| pause.until)
            .is_some_and(|until| Instant::now() < until);
        if paused {
            return Err(CredentialError::Unavailable);
        }
        let outcome = self.load().await;
        if let Ok(mut pause) = self.pause.lock() {
            match &outcome {
                Err(CredentialError::Unavailable) => pause.refused(),
                _ => *pause = Pause::default(),
            }
        }
        outcome
    }

    async fn save(&self, bundle: &Bundle) -> Result<(), CredentialError> {
        let plain: BTreeMap<&str, &str> = bundle
            .iter()
            .map(|(key, value)| (key.as_str(), value.expose()))
            .collect();
        let text =
            Secret::new(serde_json::to_string(&plain).map_err(|_| CredentialError::Verification)?);
        drop(plain);
        let entry = self.entry.clone();
        let outcome = self.blocking(move |store| store.set(&entry, &text)).await;
        self.note("write", &self.entry, &outcome);
        outcome
    }
}

fn copy(secret: &Secret) -> Secret {
    Secret::new(secret.expose().to_owned())
}

fn vault_error_for(error: CredentialError) -> EngineError {
    if matches!(error, CredentialError::Verification) {
        return EngineError::Provider {
            evidence: None,
            code: "keychain.unreadable".into(),
            message: "Hoista's entry in your keychain is there but can't be read; it may be damaged. Nothing was changed.".into(),
        };
    }
    vault_error()
}

fn vault_error() -> EngineError {
    EngineError::Provider {
        evidence: None,
        code: "keychain.unavailable".into(),
        message:
            "Unlock the system keychain and reconnect the provider account if its token is missing."
                .into(),
    }
}

#[async_trait::async_trait]
impl crate::application::ports::vault::TokenVault for NativeVault {
    async fn read(&self, reference: &str) -> Result<Secret, EngineError> {
        let mut cache = self.cache.lock().await;
        if cache.is_none() {
            *cache = Some(self.load_unless_paused().await.map_err(vault_error_for)?);
        }
        let bundle = cache.as_mut().ok_or_else(vault_error)?;
        if let Some(secret) = bundle.get(reference) {
            return Ok(copy(secret));
        }
        Err(EngineError::NotFound)
    }

    async fn write(&self, reference: &str, value: Secret) -> Result<(), EngineError> {
        let mut cache = self.cache.lock().await;
        let mut next = match cache.take() {
            Some(bundle) => bundle,
            None => self.load().await.map_err(vault_error_for)?,
        };
        let previous = next.insert(reference.to_owned(), value);
        if self.save(&next).await.is_err() {
            // Keep what the keychain still holds, not the value that failed.
            match previous {
                Some(old) => next.insert(reference.to_owned(), old),
                None => next.remove(reference),
            };
            *cache = Some(next);
            return Err(vault_error());
        }
        *cache = Some(next);
        Ok(())
    }

    async fn delete(&self, reference: &str) -> Result<(), EngineError> {
        let mut cache = self.cache.lock().await;
        let mut next = match cache.take() {
            Some(bundle) => bundle,
            None => self.load().await.map_err(vault_error_for)?,
        };
        if let Some(old) = next.remove(reference)
            && self.save(&next).await.is_err()
        {
            next.insert(reference.to_owned(), old);
            *cache = Some(next);
            return Err(vault_error());
        }
        *cache = Some(next);
        Ok(())
    }
}
