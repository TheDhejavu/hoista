//! Moving Hoista to another computer or a server: everything it knows,
//! connected accounts' tokens included, in one encrypted transfer. Only the
//! person can make one or restore one.

pub mod payload;

use crate::data::DataService;
use crate::engine::Actor;
use crate::{
    data::{
        backup::validate_backup,
        transfer::payload::{
            TransferCredential, TransferPayload, TransferRestoreRecord, TransferRestoreStatus,
        },
    },
    domain::{Secret, model::AccountStatus},
    ports::catalog::EngineError,
};
use std::{
    collections::{HashMap, HashSet},
    time::Duration,
};

const COLLECT_TIMEOUT: Duration = Duration::from_secs(30);
const RESTORE_TIMEOUT: Duration = Duration::from_secs(120);

fn owner(actor: &Actor) -> Result<(), EngineError> {
    if matches!(actor, Actor::Person) {
        Ok(())
    } else {
        Err(EngineError::Invalid(
            "Transfer recovery is available only to the local owner.",
        ))
    }
}

fn vault_key(account_id: &str) -> String {
    format!("account/{account_id}")
}

impl<'a> DataService<'a> {
    /// Gathers Hoista's records and the connected accounts' tokens, in memory
    /// that's wiped after use. What comes back must only ever be encrypted,
    /// never sent as JSON or logged.
    ///
    /// # Errors
    /// Fails for an agent, a token that can't be read (or that a server's
    /// credential keeper holds), records that don't check out, or when it
    /// takes longer than 30 seconds.
    pub async fn collect_transfer(self, actor: &Actor) -> Result<TransferPayload, EngineError> {
        if !matches!(actor, Actor::Person) {
            return Err(EngineError::Invalid(
                "Only the local owner can transfer credentials.",
            ));
        }
        tokio::time::timeout(COLLECT_TIMEOUT, self.gather_transfer())
            .await
            .map_err(|_| {
                EngineError::Invalid("Transfer collection timed out. Wait for active work to finish or unlock the native keychain, then try again.")
            })?
    }

    async fn gather_transfer(self) -> Result<TransferPayload, EngineError> {
        let _mutation = self.engine.mutation.lock().await;
        let mut payload = self.engine.repository.transfer_snapshot().await?;
        // Accounts sharing one sign-in carry it once, under the account holding it.
        let mut holders: Vec<String> = Vec::new();
        for account in &payload.catalog.accounts {
            if account.status != AccountStatus::Connected {
                continue;
            }
            let holder = account.credential.as_ref().unwrap_or(&account.id);
            if !holders.contains(holder) {
                holders.push(holder.clone());
            }
        }
        for holder in holders {
            let token = self
                .engine
                .vault
                .read(&vault_key(&holder))
                .await
                .map_err(|_| {
                    EngineError::Invalid(
                        "Unlock the native keychain before exporting connected account credentials.",
                    )
                })?;
            // On a server the tokens stay with its credential keeper.
            if token.handle_key().is_some() {
                return Err(EngineError::Conflict(
                    "Provider tokens on a server stay with its credential keeper and can't be exported. Move from your computer instead, or reconnect the accounts where you need them.",
                ));
            }
            payload.credentials.push(TransferCredential {
                account_id: holder,
                token,
            });
        }
        payload.validate()?;
        Ok(payload)
    }

    /// Restores a transfer into an empty Hoista. The keychain entries it will
    /// write are recorded first; then each token is written and read back;
    /// then the records go in. Nothing changes at any provider, projects come
    /// back watched, and agents' access never comes with them.
    ///
    /// # Errors
    /// Fails for an agent, a Hoista that isn't empty, a keychain entry that's
    /// already there or can't be written, or when it takes too long. The
    /// restore's record stays, so `transfer_recovery` can show where it got to.
    pub async fn restore_transfer(
        self,
        actor: &Actor,
        mut payload: TransferPayload,
    ) -> Result<TransferRestoreRecord, EngineError> {
        owner(actor)?;
        payload.validate()?;
        let _mutation = self.engine.mutation.lock().await;
        payload.catalog = validate_backup(payload.catalog)?;
        give_accounts_new_ids(&mut payload)?;
        let record = TransferRestoreRecord {
            id: uuid::Uuid::new_v4().to_string(),
            created_at: chrono::Utc::now().to_rfc3339(),
            credential_references: payload
                .credentials
                .iter()
                .map(|credential| vault_key(&credential.account_id))
                .collect(),
        };
        self.engine
            .repository
            .prepare_transfer_restore(&record)
            .await?;
        // A keychain write can still finish after this gives up, so every
        // entry recorded above is kept: nothing here deletes a token.
        tokio::time::timeout(RESTORE_TIMEOUT, self.install_transfer(payload, &record))
            .await
            .map_err(|_| {
                EngineError::Conflict("Restore timed out. Check hoist backups recovery; native writes may finish later and their references remain retained.")
            })??;
        Ok(record)
    }

    async fn install_transfer(
        self,
        mut payload: TransferPayload,
        record: &TransferRestoreRecord,
    ) -> Result<(), EngineError> {
        for credential in std::mem::take(&mut payload.credentials) {
            let reference = vault_key(&credential.account_id);
            match self.engine.vault.read(&reference).await {
                Err(EngineError::NotFound) => {}
                Ok(_) => {
                    return Err(EngineError::Conflict(
                        "A reserved native entry already exists; it was not overwritten. Inspect transfer recovery.",
                    ));
                }
                Err(_) => {
                    return Err(EngineError::Invalid(
                        "Unlock the native keychain, then inspect transfer recovery before retrying.",
                    ));
                }
            }
            self.engine
                .vault
                .write(
                    &reference,
                    Secret::new(credential.token.expose().to_owned()),
                )
                .await?;
            let readback = self.engine.vault.read(&reference).await?;
            // On a server the keeper holds the token and answers with a handle.
            if readback.handle_key().is_none() && readback.expose() != credential.token.expose() {
                return Err(EngineError::Conflict(
                    "Credential verification failed. Inspect transfer recovery; existing entries were retained.",
                ));
            }
            let account = payload
                .catalog
                .accounts
                .iter_mut()
                .find(|account| account.id == credential.account_id)
                .ok_or(EngineError::Store)?;
            account.status = AccountStatus::Connected;
        }
        // Accounts sharing a sign-in are connected once the sign-in is.
        let connected: HashSet<String> = payload
            .catalog
            .accounts
            .iter()
            .filter(|account| {
                account.status == AccountStatus::Connected && account.credential.is_none()
            })
            .map(|account| account.id.clone())
            .collect();
        for account in &mut payload.catalog.accounts {
            if account
                .credential
                .as_ref()
                .is_some_and(|holder| connected.contains(holder))
            {
                account.status = AccountStatus::Connected;
            }
        }
        self.engine
            .repository
            .install_transfer(&payload, record)
            .await
    }

    /// Every restore and where it got to. Reads no token.
    ///
    /// # Errors
    /// Fails for an agent, or when state can't be read.
    pub async fn transfer_recovery(
        self,
        actor: &Actor,
    ) -> Result<Vec<TransferRestoreStatus>, EngineError> {
        owner(actor)?;
        self.engine.repository.transfer_restores().await
    }

    /// Gives up on a restore that didn't finish. Installs nothing and deletes
    /// no keychain entry.
    ///
    /// # Errors
    /// Fails for an agent, an id that isn't a restore's, or a restore that
    /// isn't pending.
    pub async fn abandon_transfer(self, actor: &Actor, id: &str) -> Result<(), EngineError> {
        owner(actor)?;
        if uuid::Uuid::parse_str(id).is_err() {
            return Err(EngineError::Invalid(
                "Use the restore ID shown by transfer recovery.",
            ));
        }
        let _mutation = self.engine.mutation.lock().await;
        self.engine.repository.abandon_transfer_restore(id).await
    }
}

/// Gives every account a new id here, and points resources, shared sign-ins
/// and tokens at the new ids.
fn give_accounts_new_ids(payload: &mut TransferPayload) -> Result<(), EngineError> {
    let mut ids = HashMap::new();
    for account in &mut payload.catalog.accounts {
        let fresh = uuid::Uuid::new_v4().to_string();
        ids.insert(account.id.clone(), fresh.clone());
        account.id = fresh;
    }
    let new_id = |old: &str| ids.get(old).cloned().ok_or(EngineError::Store);
    for account in &mut payload.catalog.accounts {
        if let Some(holder) = &account.credential {
            account.credential = Some(new_id(holder)?);
        }
    }
    for resource in &mut payload.catalog.resources {
        resource.account_id = new_id(&resource.account_id)?;
    }
    for credential in &mut payload.credentials {
        credential.account_id = new_id(&credential.account_id)?;
    }
    Ok(())
}
