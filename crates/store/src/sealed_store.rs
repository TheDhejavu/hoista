//! Credential store for servers without a keychain: one file sealed with
//! XChaCha20-Poly1305 under `HOISTA_SEALED_KEY`.
//!
//! The key must stay outside the state folder, so the file alone reveals
//! nothing. Writes replace the file whole, so a crash leaves one version.

use crate::{
    application::{CredentialError, CredentialStore},
    domain::Secret,
};
use base64::Engine as _;
use chacha20poly1305::{
    XChaCha20Poly1305, XNonce,
    aead::{Aead, KeyInit, OsRng, rand_core::RngCore},
};
use std::{collections::BTreeMap, path::PathBuf, sync::Mutex};
use zeroize::Zeroizing;

/// Marks the file format, so another file is never taken for this one.
const MAGIC: &[u8; 8] = b"HOISTAS1";
const KEY_BYTES: usize = 32;
const NONCE_BYTES: usize = 24;
/// Where the sealed entries start: after the magic and the nonce.
const HEADER_BYTES: usize = MAGIC.len() + NONCE_BYTES;

/// Entries sealed in one file under a key held elsewhere.
pub struct SealedStore {
    path: PathBuf,
    cipher: XChaCha20Poly1305,
    /// One reader or writer at a time within this process.
    lock: Mutex<()>,
}

impl SealedStore {
    /// Opens (or will create) `path`, sealed with a 32-byte key given as
    /// standard base64.
    ///
    /// # Errors
    /// Rejects a key that isn't 32 bytes of base64.
    pub fn new(path: PathBuf, key_base64: &str) -> Result<Self, CredentialError> {
        let key = Zeroizing::new(
            base64::engine::general_purpose::STANDARD
                .decode(key_base64.trim())
                .map_err(|_| CredentialError::Unavailable)?,
        );
        if key.len() != KEY_BYTES {
            return Err(CredentialError::Unavailable);
        }
        let cipher =
            XChaCha20Poly1305::new_from_slice(&key).map_err(|_| CredentialError::Unavailable)?;
        Ok(Self {
            path,
            cipher,
            lock: Mutex::new(()),
        })
    }

    /// A fresh key, base64, for the person to save as the provider secret.
    #[must_use]
    pub fn new_key() -> String {
        let mut key = Zeroizing::new([0u8; KEY_BYTES]);
        OsRng.fill_bytes(key.as_mut());
        base64::engine::general_purpose::STANDARD.encode(key.as_ref())
    }

    fn read(&self) -> Result<BTreeMap<String, String>, CredentialError> {
        let bytes = match std::fs::read(&self.path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(BTreeMap::new());
            }
            Err(_) => return Err(CredentialError::Unavailable),
        };
        if bytes.len() < HEADER_BYTES || &bytes[..MAGIC.len()] != MAGIC {
            return Err(CredentialError::Verification);
        }
        let nonce = XNonce::from_slice(&bytes[MAGIC.len()..HEADER_BYTES]);
        let plain = Zeroizing::new(
            self.cipher
                .decrypt(nonce, &bytes[HEADER_BYTES..])
                .map_err(|_| CredentialError::Verification)?,
        );
        serde_json::from_slice(&plain).map_err(|_| CredentialError::Verification)
    }

    fn write(&self, entries: &BTreeMap<String, String>) -> Result<(), CredentialError> {
        let plain =
            Zeroizing::new(serde_json::to_vec(entries).map_err(|_| CredentialError::Unavailable)?);
        let mut nonce = [0u8; NONCE_BYTES];
        OsRng.fill_bytes(&mut nonce);
        let sealed = self
            .cipher
            .encrypt(XNonce::from_slice(&nonce), plain.as_slice())
            .map_err(|_| CredentialError::Unavailable)?;
        let mut out = Vec::with_capacity(HEADER_BYTES + sealed.len());
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&nonce);
        out.extend_from_slice(&sealed);
        let next = self.path.with_extension("next");
        {
            use std::io::Write as _;
            let mut options = std::fs::OpenOptions::new();
            options.write(true).create(true).truncate(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt as _;
                options.mode(0o600);
            }
            let mut file = options
                .open(&next)
                .map_err(|_| CredentialError::Unavailable)?;
            file.write_all(&out)
                .and_then(|()| file.sync_all())
                .map_err(|_| CredentialError::Unavailable)?;
        }
        std::fs::rename(&next, &self.path).map_err(|_| CredentialError::Unavailable)
    }
}

impl CredentialStore for SealedStore {
    fn get(&self, reference: &str) -> Result<Secret, CredentialError> {
        let _one = self.lock.lock().map_err(|_| CredentialError::Unavailable)?;
        self.read()?
            .remove(reference)
            .map(Secret::new)
            .ok_or(CredentialError::Missing)
    }

    fn set(&self, reference: &str, secret: &Secret) -> Result<(), CredentialError> {
        let _one = self.lock.lock().map_err(|_| CredentialError::Unavailable)?;
        let mut entries = self.read()?;
        entries.insert(reference.to_owned(), secret.expose().to_owned());
        self.write(&entries)
    }

    fn delete(&self, reference: &str) -> Result<(), CredentialError> {
        let _one = self.lock.lock().map_err(|_| CredentialError::Unavailable)?;
        let mut entries = self.read()?;
        if entries.remove(reference).is_some() {
            self.write(&entries)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entries_are_sealed_and_need_the_key() {
        let folder = tempfile::tempdir().expect("temp");
        let path = folder.path().join("vault.sealed");
        let key = SealedStore::new_key();
        let store = SealedStore::new(path.clone(), &key).unwrap();
        assert!(matches!(store.get("a"), Err(CredentialError::Missing)));
        store.set("a", &Secret::new("token-value".into())).unwrap();
        store.set("b", &Secret::new("other".into())).unwrap();
        assert_eq!(store.get("a").unwrap().expose(), "token-value");
        let bytes = std::fs::read(&path).unwrap();
        assert!(!String::from_utf8_lossy(&bytes).contains("token-value"));
        // Another key can't open it, and says so rather than "missing".
        let other = SealedStore::new(path.clone(), &SealedStore::new_key()).unwrap();
        assert!(matches!(other.get("a"), Err(CredentialError::Verification)));
        store.delete("a").unwrap();
        assert!(matches!(store.get("a"), Err(CredentialError::Missing)));
        assert_eq!(store.get("b").unwrap().expose(), "other");
        assert!(SealedStore::new(path, "short").is_err());
    }
}
