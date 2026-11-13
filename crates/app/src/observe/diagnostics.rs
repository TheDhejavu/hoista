//! Account checks: read-only, time-limited, and never showing a provider's raw error.

use crate::domain::leaks::LeakScanner;
use crate::engine::Actor;
use crate::observe::ObserveService;
use crate::{
    Provider,
    domain::model::*,
    ports::catalog::{EngineError, Page},
};
use futures_util::{StreamExt, stream};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use ts_rs::TS;

/// How long one page of account checks may take in all.
const PAGE_TIMEOUT: Duration = Duration::from_secs(25);
/// How long one account's check may take.
const CHECK_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_SCOPES: usize = 100;
const MAX_SCOPE_BYTES: usize = 256;

/// Result of checking an account. Passing doesn't prove every write permission.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum AccountDiagnosticStatus {
    /// The token check and read checks passed.
    Verified,
    /// No saved credential exists.
    CredentialMissing,
    /// The provider refused the token as expired or revoked.
    Expired,
    /// The provider refused a permission the check needs.
    MissingScopes,
    /// The keychain, the adapter or the network couldn't finish the check.
    Unavailable,
    /// This account's check timed out; whether the token works is unknown.
    TimedOut,
    /// The account is disconnected; no token was read.
    Disconnected,
    /// Not checked: the overall time ran out, or the account changed during the check.
    NotChecked,
}

/// One account's check result. Holds no tokens, provider responses or connection strings.
#[derive(Debug, Serialize, Deserialize, JsonSchema, TS)]
pub struct AccountDiagnostic {
    /// Account id.
    pub account_id: String,
    /// The provider.
    pub provider: Provider,
    /// The account's label in Hoista.
    pub label: String,
    /// Result of this check; the saved account status is not changed.
    pub status: AccountDiagnosticStatus,
    /// Permissions the check saw; not a full list of what the token can write.
    pub verified_scopes: Vec<String>,
    /// RFC 3339 time the check finished; null when it didn't run or the account is
    /// disconnected.
    pub checked_at: Option<String>,
    /// What to do next. Never the provider's own error text.
    pub message: String,
}

fn result(
    account: &Account,
    status: AccountDiagnosticStatus,
    scopes: Vec<String>,
) -> AccountDiagnostic {
    let message = match status {
        AccountDiagnosticStatus::Verified => {
            "Credential verification succeeded. Reported permissions cover the adapter's probes, not every provider operation."
        }
        AccountDiagnosticStatus::CredentialMissing => {
            "The saved credential is missing. Reconnect this account through the local connection form."
        }
        AccountDiagnosticStatus::Expired => {
            "The provider rejected this credential. Reconnect the account with a current token."
        }
        AccountDiagnosticStatus::MissingScopes => {
            "The provider denied a verification permission. Review required scopes and reconnect the account."
        }
        AccountDiagnosticStatus::Unavailable => {
            "Verification is unavailable. Check keychain access and network connectivity, then retry."
        }
        AccountDiagnosticStatus::TimedOut => {
            "Verification timed out. Credential validity is unknown; check connectivity and retry."
        }
        AccountDiagnosticStatus::Disconnected => {
            "This account is disconnected. No credential or provider request was made."
        }
        AccountDiagnosticStatus::NotChecked => {
            "No current result is available within this batch. Retry this page; account metadata may have changed."
        }
    };
    AccountDiagnostic {
        account_id: account.id.clone(),
        provider: account.provider,
        label: account.label.clone(),
        status,
        verified_scopes: scopes,
        checked_at: (!matches!(
            status,
            AccountDiagnosticStatus::Disconnected | AccountDiagnosticStatus::NotChecked
        ))
        .then(crate::engine::now),
        message: message.into(),
    }
}

fn failure(error: &EngineError) -> AccountDiagnosticStatus {
    match error {
        EngineError::NotFound => AccountDiagnosticStatus::CredentialMissing,
        EngineError::Provider { code, .. } if code == "provider.token_expired" => {
            AccountDiagnosticStatus::Expired
        }
        EngineError::Provider { code, .. } if code == "provider.token_scope" => {
            AccountDiagnosticStatus::MissingScopes
        }
        _ => AccountDiagnosticStatus::Unavailable,
    }
}

impl<'a> ObserveService<'a> {
    /// Checks one page of accounts: at most four at once, ten seconds each, 25 seconds in
    /// all. Doesn't change saved account state or hold the deploy lock.
    ///
    /// # Errors
    /// Rejects missing permission, a check already running and unreadable account state.
    pub async fn diagnose_accounts(
        self,
        actor: &Actor,
        cursor: Option<&str>,
    ) -> Result<Page<AccountDiagnostic>, EngineError> {
        self.engine.authorize(actor, "apps:read").await?;
        let _guard = self.engine.diagnostics.try_lock().map_err(|_| {
            EngineError::Conflict(
                "Account diagnostics are already running. Retry when they finish.",
            )
        })?;
        let page = self.engine.repository.accounts(cursor).await?;
        let deadline = tokio::time::Instant::now() + PAGE_TIMEOUT;
        let mut pending = stream::iter(page.items.clone())
            .map(|account: Account| async move {
                if account.status == AccountStatus::Disconnected {
                    return result(&account, AccountDiagnosticStatus::Disconnected, vec![]);
                }
                let check = async {
                    let token = self.engine.credential(&account).await?;
                    let scopes = self
                        .engine
                        .adapter(account.provider)?
                        .verify(&token, &account.external_id)
                        .await?;
                    let scanner = LeakScanner::new().map_err(|_| EngineError::Store)?;
                    if scopes.len() > MAX_SCOPES
                        || scopes.iter().any(|scope| {
                            scope.len() > MAX_SCOPE_BYTES
                                || scope.chars().any(char::is_control)
                                || !scanner
                                    .scan(
                                        "permission",
                                        scope.as_bytes(),
                                        std::slice::from_ref(&token),
                                    )
                                    .is_empty()
                        })
                    {
                        return Err(EngineError::Invalid(
                            "Verification returned invalid permission metadata.",
                        ));
                    }
                    Ok(scopes)
                };
                let mut diagnostic = match tokio::time::timeout(CHECK_TIMEOUT, check).await {
                    Ok(Ok(scopes)) => result(&account, AccountDiagnosticStatus::Verified, scopes),
                    Ok(Err(error)) => result(&account, failure(&error), vec![]),
                    Err(_) => result(&account, AccountDiagnosticStatus::TimedOut, vec![]),
                };
                // A disconnect or reconnect during the check makes its result stale.
                if self
                    .engine
                    .repository
                    .account(&account.id)
                    .await
                    .ok()
                    .flatten()
                    .as_ref()
                    != Some(&account)
                {
                    diagnostic = result(&account, AccountDiagnosticStatus::NotChecked, vec![]);
                }
                diagnostic
            })
            .buffer_unordered(4);
        let mut completed = std::collections::BTreeMap::new();
        while let Ok(Some(diagnostic)) = tokio::time::timeout_at(deadline, pending.next()).await {
            completed.insert(diagnostic.account_id.clone(), diagnostic);
        }
        drop(pending);
        let items = page
            .items
            .iter()
            .map(|account| {
                completed.remove(&account.id).unwrap_or_else(|| {
                    result(
                        account,
                        if account.status == AccountStatus::Disconnected {
                            AccountDiagnosticStatus::Disconnected
                        } else {
                            AccountDiagnosticStatus::NotChecked
                        },
                        vec![],
                    )
                })
            })
            .collect();
        self.engine.authorize(actor, "apps:read").await?;
        Ok(Page {
            items,
            next_cursor: page.next_cursor,
        })
    }
}
