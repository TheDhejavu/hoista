//! Encrypting and decrypting a transfer with a passphrase: Argon2id turns the
//! passphrase into a key, and XChaCha20-Poly1305 seals the contents. Writes
//! nothing; the caller checks what comes out before using it.
//!
//! A sealed transfer is the header, then the encrypted contents, then the
//! authentication tag. The header is the magic, a random salt and a random
//! nonce, and the tag covers it too, so none of it can be changed unnoticed.

use crate::domain::Secret;
use argon2::{Algorithm, Argon2, Block, Params, Version};
use chacha20poly1305::{
    XChaCha20Poly1305, XNonce,
    aead::{AeadInPlace, KeyInit, OsRng, rand_core::RngCore},
};
use std::{ops::Range, sync::Arc};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use zeroize::Zeroizing;

/// The format and its version, in the last byte.
const MAGIC: &[u8; 8] = b"HOISTA\0\x01";
const SALT: Range<usize> = 8..24;
const NONCE: Range<usize> = 24..48;
const HEADER: usize = NONCE.end;
const TAG: usize = 16;
const KEY_BYTES: usize = 32;

// Argon2id settings. They're fixed rather than read from the file, so a
// crafted file can't ask for weaker or endless work.
const ARGON2_MEMORY_KIB: u32 = 64 * 1024;
const ARGON2_PASSES: u32 = 3;
const ARGON2_LANES: u32 = 4;

/// Passphrase length in bytes, as typed: no trimming or normalizing.
const PASSPHRASE_BYTES: std::ops::RangeInclusive<usize> = 12..=1024;

/// The most a transfer can hold before it's sealed.
pub const MAX_PAYLOAD: usize = 16 * 1024 * 1024;
/// The most a sealed transfer can be, header and tag included.
pub const MAX_ENVELOPE: usize = HEADER + MAX_PAYLOAD + TAG;

/// A sealed transfer. Only [`TransferCipher`] makes these, so anything holding
/// one knows it's encrypted.
#[derive(Clone)]
pub struct EncryptedTransfer(Vec<u8>);

impl std::fmt::Debug for EncryptedTransfer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EncryptedTransfer")
            .field("bytes", &self.0.len())
            .finish()
    }
}

impl EncryptedTransfer {
    /// The sealed bytes.
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// The sealed bytes, to send or save.
    pub fn into_bytes(self) -> Vec<u8> {
        self.0
    }
}

/// Why a transfer couldn't be sealed or opened. The messages never include
/// the passphrase, the key, the contents or the crypto library's own errors.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum TransferCryptoError {
    /// Not a Hoista transfer, or a version this build can't read. Caught
    /// before any key work.
    #[error("Unsupported or malformed Hoista transfer.")]
    Format,
    /// Over the size limit. Caught before any key work.
    #[error("The Hoista transfer exceeds the 16 MiB payload limit.")]
    TooLarge,
    /// The passphrase is too short or too long.
    #[error("Use a passphrase between 12 and 1024 UTF-8 bytes.")]
    Passphrase,
    /// The wrong passphrase, or a changed file. There's no telling which.
    #[error("Cannot unlock the transfer. Check its passphrase and file integrity.")]
    Authentication,
    /// Another transfer is being sealed or opened. Requests don't queue.
    #[error("Another transfer encryption operation is running. Try again after it finishes.")]
    Busy,
    /// Memory, randomness or the worker thread failed. Nothing unencrypted
    /// is ever produced instead.
    #[error("Transfer encryption is unavailable. No unencrypted transfer was produced.")]
    Unavailable,
}

/// Seals and opens transfers, one at a time. Clones share the one slot.
///
/// The key work takes a lot of memory, so a second request is refused rather
/// than run alongside. A caller that gives up doesn't free the slot until
/// the work it started has finished.
#[derive(Clone)]
pub struct TransferCipher {
    permits: Arc<Semaphore>,
}

impl Default for TransferCipher {
    fn default() -> Self {
        Self {
            permits: Arc::new(Semaphore::new(1)),
        }
    }
}

impl TransferCipher {
    fn permit(&self) -> Result<OwnedSemaphorePermit, TransferCryptoError> {
        self.permits
            .clone()
            .try_acquire_owned()
            .map_err(|_| TransferCryptoError::Busy)
    }

    /// Seals `plaintext` with a fresh random salt and nonce.
    ///
    /// # Errors
    /// Fails for a passphrase of the wrong length, contents over the limit,
    /// another transfer in progress, or a failure in the crypto itself.
    pub async fn seal(
        &self,
        passphrase: Secret,
        plaintext: Zeroizing<Vec<u8>>,
    ) -> Result<EncryptedTransfer, TransferCryptoError> {
        check_passphrase(&passphrase)?;
        if plaintext.len() > MAX_PAYLOAD {
            return Err(TransferCryptoError::TooLarge);
        }
        let permit = self.permit()?;
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            let mut header = [0u8; HEADER];
            header[..MAGIC.len()].copy_from_slice(MAGIC);
            OsRng
                .try_fill_bytes(&mut header[MAGIC.len()..])
                .map_err(|_| TransferCryptoError::Unavailable)?;
            seal_with_header(&passphrase, plaintext, &header).map(EncryptedTransfer)
        })
        .await
        .map_err(|_| TransferCryptoError::Unavailable)?
    }

    /// Opens a sealed transfer, checking all of it before returning anything.
    /// What comes out still has to be checked before it's used.
    ///
    /// # Errors
    /// Fails for a passphrase of the wrong length, a file that isn't a
    /// transfer or is too large (both before any key work), another transfer
    /// in progress, or [`TransferCryptoError::Authentication`] for a wrong
    /// passphrase or a changed file.
    pub async fn open(
        &self,
        passphrase: Secret,
        envelope: Vec<u8>,
    ) -> Result<Zeroizing<Vec<u8>>, TransferCryptoError> {
        check_passphrase(&passphrase)?;
        check_envelope(&envelope)?;
        let permit = self.permit()?;
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            open_envelope(&passphrase, &envelope)
        })
        .await
        .map_err(|_| TransferCryptoError::Unavailable)?
    }
}

fn check_passphrase(passphrase: &Secret) -> Result<(), TransferCryptoError> {
    if PASSPHRASE_BYTES.contains(&passphrase.expose().len()) {
        Ok(())
    } else {
        Err(TransferCryptoError::Passphrase)
    }
}

fn check_envelope(bytes: &[u8]) -> Result<(), TransferCryptoError> {
    if bytes.len() > MAX_ENVELOPE {
        return Err(TransferCryptoError::TooLarge);
    }
    if bytes.len() < HEADER + TAG || &bytes[..MAGIC.len()] != MAGIC {
        return Err(TransferCryptoError::Format);
    }
    Ok(())
}

fn derive_key(
    passphrase: &Secret,
    salt: &[u8],
) -> Result<Zeroizing<[u8; KEY_BYTES]>, TransferCryptoError> {
    let params = Params::new(
        ARGON2_MEMORY_KIB,
        ARGON2_PASSES,
        ARGON2_LANES,
        Some(KEY_BYTES),
    )
    .map_err(|_| TransferCryptoError::Unavailable)?;
    // Argon2's working memory, reserved up front so running out fails cleanly
    // and wiped afterwards.
    let mut blocks = Zeroizing::new(Vec::new());
    blocks
        .try_reserve_exact(params.block_count())
        .map_err(|_| TransferCryptoError::Unavailable)?;
    blocks.resize(params.block_count(), Block::new());
    let mut key = Zeroizing::new([0; KEY_BYTES]);
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
        .hash_password_into_with_memory(
            passphrase.expose().as_bytes(),
            salt,
            &mut *key,
            &mut *blocks,
        )
        .map_err(|_| TransferCryptoError::Unavailable)?;
    Ok(key)
}

fn seal_with_header(
    passphrase: &Secret,
    mut plaintext: Zeroizing<Vec<u8>>,
    header: &[u8; HEADER],
) -> Result<Vec<u8>, TransferCryptoError> {
    let key = derive_key(passphrase, &header[SALT])?;
    let cipher = XChaCha20Poly1305::new((&*key).into());
    let tag = cipher
        .encrypt_in_place_detached(XNonce::from_slice(&header[NONCE]), header, &mut plaintext)
        .map_err(|_| TransferCryptoError::Unavailable)?;
    let mut envelope = Vec::new();
    envelope
        .try_reserve_exact(HEADER + plaintext.len() + TAG)
        .map_err(|_| TransferCryptoError::Unavailable)?;
    envelope.extend_from_slice(header);
    envelope.extend_from_slice(&plaintext);
    envelope.extend_from_slice(&tag);
    Ok(envelope)
}

fn open_envelope(
    passphrase: &Secret,
    envelope: &[u8],
) -> Result<Zeroizing<Vec<u8>>, TransferCryptoError> {
    let (header, rest) = envelope.split_at(HEADER);
    let (sealed, tag) = rest.split_at(rest.len() - TAG);
    let key = derive_key(passphrase, &header[SALT])?;
    let cipher = XChaCha20Poly1305::new((&*key).into());
    let mut plaintext = Zeroizing::new(Vec::new());
    plaintext
        .try_reserve_exact(sealed.len())
        .map_err(|_| TransferCryptoError::Unavailable)?;
    plaintext.extend_from_slice(sealed);
    cipher
        .decrypt_in_place_detached(
            XNonce::from_slice(&header[NONCE]),
            header,
            &mut plaintext,
            tag.into(),
        )
        .map_err(|_| TransferCryptoError::Authentication)?;
    Ok(plaintext)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn passphrase() -> Secret {
        Secret::new("fixture passphrase 🔐".into())
    }

    #[tokio::test]
    async fn transfer_roundtrip_randomizes_and_authenticates_every_section() {
        let cipher = TransferCipher::default();
        let payload = b"provider-token-fixture\0exact bytes\n";
        let first = cipher
            .seal(passphrase(), Zeroizing::new(payload.to_vec()))
            .await
            .unwrap()
            .into_bytes();
        let second = cipher
            .seal(passphrase(), Zeroizing::new(payload.to_vec()))
            .await
            .unwrap()
            .into_bytes();
        assert_ne!(first, second);
        assert_ne!(&first[SALT], &second[SALT]);
        assert_ne!(&first[NONCE], &second[NONCE]);
        assert!(!first.windows(payload.len()).any(|w| w == payload));
        assert_eq!(
            &*cipher.open(passphrase(), first.clone()).await.unwrap(),
            payload
        );
        assert_eq!(
            cipher
                .open(Secret::new("wrong passphrase".into()), first.clone())
                .await
                .unwrap_err(),
            TransferCryptoError::Authentication
        );
        // Salt, nonce, ciphertext and tag all participate in authentication.
        for index in [SALT.start, NONCE.start, HEADER, first.len() - 1] {
            let mut changed = first.clone();
            changed[index] ^= 1;
            assert_eq!(
                cipher.open(passphrase(), changed).await.unwrap_err(),
                TransferCryptoError::Authentication
            );
        }
        let mut appended = first.clone();
        appended.push(0);
        assert_eq!(
            cipher.open(passphrase(), appended).await.unwrap_err(),
            TransferCryptoError::Authentication
        );
        let mut truncated = first;
        truncated.pop();
        assert_eq!(
            cipher.open(passphrase(), truncated).await.unwrap_err(),
            TransferCryptoError::Authentication
        );
    }

    #[tokio::test]
    async fn transfer_rejects_unknown_versions_and_bounds_before_taking_capacity() {
        let cipher = TransferCipher::default();
        let _busy = cipher.permit().unwrap();
        assert_eq!(
            cipher
                .open(passphrase(), vec![0; HEADER + TAG])
                .await
                .unwrap_err(),
            TransferCryptoError::Format
        );
        assert_eq!(
            cipher
                .open(passphrase(), vec![0; MAX_ENVELOPE + 1])
                .await
                .unwrap_err(),
            TransferCryptoError::TooLarge
        );
        let mut header = vec![0; HEADER + TAG];
        header[..MAGIC.len()].copy_from_slice(MAGIC);
        header[MAGIC.len() - 1] = 2;
        assert_eq!(
            cipher.open(passphrase(), header).await.unwrap_err(),
            TransferCryptoError::Format
        );
        assert_eq!(
            cipher
                .seal(Secret::new("short".into()), Zeroizing::new(vec![]))
                .await
                .unwrap_err(),
            TransferCryptoError::Passphrase
        );
        assert_eq!(
            cipher
                .seal(Secret::new("x".repeat(1025)), Zeroizing::new(vec![]))
                .await
                .unwrap_err(),
            TransferCryptoError::Passphrase
        );
        assert_eq!(
            cipher
                .seal(passphrase(), Zeroizing::new(vec![0; MAX_PAYLOAD + 1]))
                .await
                .unwrap_err(),
            TransferCryptoError::TooLarge
        );
        assert_eq!(
            cipher
                .seal(passphrase(), Zeroizing::new(vec![]))
                .await
                .unwrap_err(),
            TransferCryptoError::Busy
        );
    }

    #[tokio::test]
    async fn cancelling_a_caller_retains_capacity_until_its_worker_finishes() {
        let cipher = TransferCipher::default();
        let worker_cipher = cipher.clone();
        let caller = tokio::spawn(async move {
            worker_cipher
                .seal(passphrase(), Zeroizing::new(vec![1]))
                .await
        });
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while cipher.permits.available_permits() != 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        caller.abort();
        assert_eq!(
            cipher
                .seal(passphrase(), Zeroizing::new(vec![]))
                .await
                .unwrap_err(),
            TransferCryptoError::Busy
        );
        tokio::time::timeout(std::time::Duration::from_secs(60), async {
            while cipher.permits.available_permits() == 0 {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        assert!(cipher.permit().is_ok());
    }

    #[test]
    fn envelope_matches_independent_argon2_and_libsodium_vector() {
        // Made separately with argon2-cffi 25.1.0 and PyNaCl 1.6.2 (Argon2id
        // v19, m=65536, t=3, p=4). The salt and nonce are fixed here only;
        // real transfers always get fresh random ones.
        let hex = "484f495354410001000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262719fbb7ec14398b7d97d7915bc173dd8dcfde2f6b72e86117afc8b2fb62a45e0eeb34f0d268639584cab147a63bd323374613e1";
        let expected = (0..hex.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
            .collect::<Vec<_>>();
        let header: [u8; HEADER] = expected[..HEADER].try_into().unwrap();
        let payload = b"provider-token-fixture\0exact bytes\n";
        assert_eq!(
            seal_with_header(&passphrase(), Zeroizing::new(payload.to_vec()), &header).unwrap(),
            expected
        );
        assert_eq!(&*open_envelope(&passphrase(), &expected).unwrap(), payload);
    }
}
