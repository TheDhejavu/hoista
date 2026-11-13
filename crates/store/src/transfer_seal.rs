//! Transfers sealed with `transfer_crypto` and kept in files by `transfer_file`.

use crate::{
    application::ports::{
        catalog::EngineError,
        transfer::{SavedTransfer, TransferSeal},
    },
    domain::Secret,
    transfer_crypto::{MAX_ENVELOPE, TransferCipher, TransferCryptoError},
    transfer_file::{self, TransferFileError},
};
use std::path::{Path, PathBuf};
use zeroize::Zeroizing;

/// Seals with a passphrase-derived key and saves to local files.
#[derive(Default, Clone)]
pub struct LocalTransferSeal {
    cipher: TransferCipher,
}

fn crypto_error(error: TransferCryptoError) -> EngineError {
    EngineError::Provider {
        evidence: None,
        code: "transfer.crypto".into(),
        message: error.to_string(),
    }
}

fn file_error(error: TransferFileError) -> EngineError {
    EngineError::Provider {
        evidence: None,
        code: "transfer.file".into(),
        message: error.to_string(),
    }
}

#[async_trait::async_trait]
impl TransferSeal for LocalTransferSeal {
    fn max_sealed(&self) -> usize {
        MAX_ENVELOPE
    }

    async fn seal_to_file(
        &self,
        path: &Path,
        passphrase: Secret,
        plaintext: Zeroizing<Vec<u8>>,
    ) -> Result<SavedTransfer, EngineError> {
        let encrypted = self
            .cipher
            .seal(passphrase, plaintext)
            .await
            .map_err(crypto_error)?;
        let path: PathBuf = path.to_path_buf();
        tokio::task::spawn_blocking(move || match transfer_file::save(&path, &encrypted) {
            Ok(receipt) => Ok(SavedTransfer {
                path: receipt.path,
                bytes: receipt.bytes,
                sha256: receipt.sha256,
                durable: true,
            }),
            Err(TransferFileError::DurabilityUncertain(receipt)) => Ok(SavedTransfer {
                path: receipt.path,
                bytes: receipt.bytes,
                sha256: receipt.sha256,
                durable: false,
            }),
            Err(error) => Err(file_error(error)),
        })
        .await
        .map_err(|_| EngineError::Store)?
    }

    async fn open_file(
        &self,
        path: &Path,
        passphrase: Secret,
    ) -> Result<Zeroizing<Vec<u8>>, EngineError> {
        let path = path.to_path_buf();
        let bytes = tokio::task::spawn_blocking(move || transfer_file::read(&path))
            .await
            .map_err(|_| EngineError::Store)?
            .map_err(file_error)?;
        self.open(bytes, passphrase).await
    }

    async fn open(
        &self,
        sealed: Vec<u8>,
        passphrase: Secret,
    ) -> Result<Zeroizing<Vec<u8>>, EngineError> {
        self.cipher
            .open(passphrase, sealed)
            .await
            .map_err(crypto_error)
    }
}
