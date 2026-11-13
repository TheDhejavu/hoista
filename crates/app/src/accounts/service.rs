//! The accounts service: provider accounts: connecting, signing in, choosing what to import, rescanning and billing.

use crate::engine::*;
use crate::ports::catalog::EngineError;
use crate::{
    Provider,
    domain::{Secret, model::*},
};

const MAX_ACCOUNTS: usize = 1_000;
/// The key that seals a server's token file.
const SEALED_KEY_BYTES: usize = 32;

/// Provider accounts: connecting, signing in, choosing what to import, rescanning and billing.
#[derive(Clone, Copy)]
pub struct AccountService<'a> {
    pub(crate) engine: &'a Engine,
}

impl Engine {
    /// Provider accounts: connecting, signing in, choosing what to import, rescanning and billing.
    #[must_use]
    pub fn accounts(&self) -> AccountService<'_> {
        AccountService { engine: self }
    }
}

impl<'a> AccountService<'a> {
    /// Checks a token at the provider before saving it. Only the person's own connect form calls
    /// this.
    ///
    /// # Errors
    /// Reports invalid input, missing scopes, inaccessible keychain or state that can't be saved.
    pub async fn connect_account(
        self,
        provider: Provider,
        label: String,
        external_id: String,
        token: Secret,
    ) -> Result<Account, EngineError> {
        let _mutation = self.engine.mutation.lock().await;
        if label.trim().is_empty()
            || label.len() > 80
            || external_id.len() > 256
            || external_id.chars().any(char::is_control)
            || token.expose().is_empty()
        {
            return Err(EngineError::Invalid(
                "Provide a provider account, label and token.",
            ));
        }
        // A 1Password reference is checked with the value it points at, and
        // only the reference is kept: the token stays in 1Password.
        let usable = self.engine.vault.resolve(&token).await?;
        let scopes = self
            .engine
            .adapter(provider)?
            .verify(&usable, &external_id)
            .await?;
        drop(usable);
        let previous = self
            .engine
            .accounts()
            .all_accounts()
            .await?
            .into_iter()
            .find(|account| account.provider == provider && account.external_id == external_id);
        let previous_token = if let Some(account) = &previous {
            match self
                .engine
                .vault
                .read(&format!("account/{}", account.id))
                .await
            {
                Ok(token) => Some(token),
                Err(EngineError::NotFound) => None,
                Err(error) => return Err(error),
            }
        } else {
            None
        };
        let account = Account {
            last_discovery: None,
            id: previous
                .map(|account| account.id)
                .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
            provider,
            label: label.trim().into(),
            external_id,
            scopes,
            status: AccountStatus::Connected,
            connected_at: now(),
            credential: None,
            // A 1Password secret reference, read through 1Password when needed.
            in_1password: token.expose().starts_with("op://"),
        };
        let reference = format!("account/{}", account.id);
        self.engine.vault.write(&reference, token).await?;
        if let Err(error) = self.engine.repository.put_account(&account).await {
            if let Some(old_token) = previous_token {
                self.engine.vault.write(&reference, old_token).await?;
            } else {
                self.engine.vault.delete(&reference).await?;
            }
            return Err(error);
        }
        // The provider's command-line tool downloads now, not at an agent's first command.
        if let Ok(adapter) = self.engine.adapter(provider) {
            adapter.prepare_tools();
        }
        // If discovery fails, the checked connection stays saved.
        let _scan = self
            .engine
            .discover_locked(&Actor::Person, &account.id)
            .await;
        let saved = self
            .engine
            .repository
            .account(&account.id)
            .await?
            .ok_or(EngineError::NotFound)?;
        if saved.last_discovery.is_none() {
            // A failed discovery still saves a summary; none means saving it failed.
            return Err(EngineError::Store);
        }
        Ok(saved)
    }

    /// Reads credentials only for an account still enabled in the catalog.
    ///
    /// # Errors
    /// Reports disconnected accounts or unavailable keychain entries.
    pub async fn account_token(self, id: &str) -> Result<Secret, EngineError> {
        let account = self
            .engine
            .repository
            .account(id)
            .await?
            .ok_or(EngineError::NotFound)?;
        if account.status != AccountStatus::Connected {
            return Err(EngineError::Conflict(
                "Reconnect this account before using its provider.",
            ));
        }
        self.engine.credential(&account).await
    }

    /// Checks a saved token again at the provider, read-only. A disconnected account stays
    /// disconnected; a network error keeps the last known state.
    ///
    /// # Errors
    /// Reports a missing or disconnected account, keychain access, or a temporary provider failure.
    pub async fn check_account(self, id: &str) -> Result<Account, EngineError> {
        let _mutation = self.engine.mutation.lock().await;
        let mut account = self
            .engine
            .repository
            .account(id)
            .await?
            .ok_or(EngineError::NotFound)?;
        if account.status == AccountStatus::Disconnected {
            return Err(EngineError::Conflict(
                "Reconnect this account before checking its credential.",
            ));
        }
        let result = async {
            let token = self.engine.credential(&account).await?;
            self.engine
                .adapter(account.provider)?
                .verify(&token, &account.external_id)
                .await
        }
        .await;
        match result {
            Ok(scopes) => {
                account.scopes = scopes;
                account.status = AccountStatus::Connected;
            }
            Err(EngineError::NotFound) => {
                account.scopes.clear();
                account.status = AccountStatus::Expired;
            }
            Err(EngineError::Provider { code, .. })
                if code == "provider.token_expired" || code == "provider.token_scope" =>
            {
                account.scopes.clear();
                account.status = if code == "provider.token_expired" {
                    AccountStatus::Expired
                } else {
                    AccountStatus::MissingScopes
                };
            }
            Err(error) => return Err(error),
        }
        self.engine.repository.put_account(&account).await?;
        Ok(account)
    }

    /// Removes only the token, keeping the account's resources for when it reconnects.
    ///
    /// # Errors
    /// Reports an unknown account or a keychain or storage failure. No provider is contacted.
    pub async fn disconnect_account(self, id: &str) -> Result<(), EngineError> {
        let _mutation = self.engine.mutation.lock().await;
        let mut account = self
            .engine
            .repository
            .account(id)
            .await?
            .ok_or(EngineError::NotFound)?;
        // Persist the disabled state first: a failed keychain deletion must not leave
        // the account available for new operations. Retrying deletion is safe.
        account.status = AccountStatus::Disconnected;
        account.scopes.clear();
        self.engine.repository.put_account(&account).await?;
        // A sign-in shared by other workspaces stays until the last one goes.
        let holder = account.credential.clone().unwrap_or_else(|| id.to_owned());
        let shared = self
            .engine
            .accounts()
            .all_accounts()
            .await?
            .into_iter()
            .any(|other| {
                other.id != id
                    && other.status != AccountStatus::Disconnected
                    && other.credential.as_deref().unwrap_or(&other.id) == holder
            });
        if shared {
            return Ok(());
        }
        self.engine.vault.delete(&format!("account/{holder}")).await
    }

    /// Reads every saved account, up to 1,000.
    ///
    /// # Errors
    /// Reports a storage failure or more than 1,000 accounts.
    pub async fn all_accounts(self) -> Result<Vec<Account>, EngineError> {
        let mut all = Vec::new();
        let mut cursor = None;
        loop {
            let page = self.engine.repository.accounts(cursor.as_deref()).await?;
            all.extend(page.items);
            if all.len() > MAX_ACCOUNTS {
                return Err(EngineError::Conflict(
                    "This machine exceeds 1,000 connected accounts.",
                ));
            }
            cursor = page.next_cursor;
            if cursor.is_none() {
                return Ok(all);
            }
        }
    }

    /// Discovers resources without creating, deleting or changing anything on the provider.
    /// Existing managed resources retain their local management mode.
    ///
    /// # Errors
    /// Returns provider, credential or storage errors; a partial import is never reported complete.
    pub async fn discover(
        self,
        actor: &Actor,
        account_id: &str,
    ) -> Result<DiscoveryResult, EngineError> {
        let _mutation = self.engine.mutation.lock().await;
        self.engine.authorize(actor, "apps:read").await?;
        self.engine.discover_locked(actor, account_id).await
    }

    /// Reads a connected provider again for its current status and anything new,
    /// without a line in the activity feed. Run by the background monitor.
    ///
    /// # Errors
    /// Returns provider, credential or persistence errors; existing resources are kept.
    pub async fn refresh_found(self, account_id: &str) -> Result<(), EngineError> {
        self.engine.scan_unlocked(account_id).await.map(|_| ())
    }

    /// Connects to a Hoista already running on a server, set up elsewhere
    /// (with `hoist` on that server), by its address. This computer then
    /// signs in to it the same way as to one it set up.
    ///
    /// # Errors
    /// Rejects agents, an address that isn't `https://` and a host name (or
    /// `http://localhost` for one on this computer), or a remote already set up here.
    pub async fn connect_server(self, actor: &Actor, url: &str) -> Result<Settings, EngineError> {
        if !matches!(actor, Actor::Person) {
            return Err(EngineError::Conflict("Only the person can change this."));
        }
        let parsed = url::Url::parse(url.trim()).map_err(|_| {
            EngineError::Invalid("Give your remote's address, such as https://hoista.example.com.")
        })?;
        let host = parsed.host_str().unwrap_or_default().to_owned();
        // Plain HTTP only to this computer (a Hoista in Docker here), which
        // browsers treat as secure, so passkeys work there too.
        let loopback = matches!(host.as_str(), "localhost" | "127.0.0.1" | "[::1]");
        let scheme_fine = parsed.scheme() == "https" || (parsed.scheme() == "http" && loopback);
        if !scheme_fine
            || host.is_empty()
            || !parsed.username().is_empty()
            || parsed.password().is_some()
            || parsed.query().is_some()
            || !matches!(parsed.path(), "" | "/")
        {
            return Err(EngineError::Invalid(
                "Give your remote's address, such as https://hoista.example.com.",
            ));
        }
        let mut settings = self.engine.repository.settings().await?;
        if settings.server.is_some() {
            return Err(EngineError::Conflict(
                "A remote is already set up here. Remove it first to connect another.",
            ));
        }
        let scheme = parsed.scheme();
        let origin = match parsed.port() {
            Some(port) => format!("{scheme}://{host}:{port}"),
            None => format!("{scheme}://{host}"),
        };
        settings.server = Some(ServerRecord {
            provider: None,
            account_id: None,
            url: origin,
            reference: None,
            image: None,
            created_at: now(),
        });
        self.engine.repository.put_settings(&settings).await?;
        Ok(settings)
    }

    /// Stops showing the server this computer set up. It keeps running at
    /// the provider until the person deletes it there.
    ///
    /// # Errors
    /// Rejects agents and reports state that can't be saved.
    pub async fn forget_server(self, actor: &Actor) -> Result<Settings, EngineError> {
        if !matches!(actor, Actor::Person) {
            return Err(EngineError::Conflict("Only the person can change this."));
        }
        let mut settings = self.engine.repository.settings().await?;
        settings.server = None;
        self.engine.repository.put_settings(&settings).await?;
        Ok(settings)
    }

    /// Puts Hoista itself on the person's Railway: a provider project of its own
    /// built from `repo`, with a sealed-token key and a claim code set before
    /// its first start. Returns the address and the link that claims it (the
    /// code after `#`, so it never reaches a server log). The key stays only
    /// at the provider. Only the person can do this, after their owner check.
    ///
    /// # Errors
    /// Rejects agents, unknown or disconnected accounts and provider failures.
    pub async fn set_up_server(
        self,
        actor: &Actor,
        account_id: &str,
        repo: Option<&str>,
        branch: Option<&str>,
    ) -> Result<ServerSetup, EngineError> {
        use crate::ports::providers::ServerSource;
        use base64::Engine as _;
        use chacha20poly1305::aead::{OsRng, rand_core::RngCore};
        if !matches!(actor, Actor::Person) {
            return Err(EngineError::Conflict(
                "Only the person can put Hoista on a server.",
            ));
        }
        let account = self
            .engine
            .repository
            .account(account_id)
            .await?
            .ok_or(EngineError::NotFound)?;
        if account.status != AccountStatus::Connected {
            return Err(EngineError::Conflict(
                "This account isn't connected. Reconnect it in Hoista first.",
            ));
        }
        let adapter = self.engine.adapter(account.provider)?;
        if !adapter.capabilities().server {
            return Err(EngineError::Conflict(
                "Hoista can't run on this provider yet. Pick another account.",
            ));
        }
        let source = match (repo, &self.engine.server_image) {
            (Some(repo), _) => ServerSource::Repo {
                repo: repo.trim().to_owned(),
                branch: branch.map(str::to_owned),
            },
            (None, Some(image)) if crate::is_pinned_image(image) => {
                ServerSource::Image(image.clone())
            }
            (None, _) => {
                return Err(EngineError::Conflict(
                    "This build of Hoista has no published server image. Build one from your own copy of Hoista's repository instead.",
                ));
            }
        };
        self.engine
            .confirm_owner(&format!(
                "put Hoista on your {} account",
                account.provider.display_name()
            ))
            .await?;
        let mut key = zeroize::Zeroizing::new([0u8; SEALED_KEY_BYTES]);
        OsRng.fill_bytes(key.as_mut());
        let code = crate::agents::passkey::claim_code();
        // The provider adds HOISTA_PUBLIC_URL: only it knows the address it gives.
        let mut variables = std::collections::BTreeMap::from([
            (
                "HOISTA_SEALED_KEY".to_owned(),
                Secret::new(base64::engine::general_purpose::STANDARD.encode(key.as_ref())),
            ),
            ("HOISTA_CLAIM_CODE".to_owned(), Secret::new(code.clone())),
        ]);
        drop(key);
        let image = match &source {
            ServerSource::Image(image) => Some(image.clone()),
            ServerSource::Repo { .. } => None,
        };
        if let Some(image) = &image {
            // So the server's page can say which image it was started from.
            variables.insert("HOISTA_IMAGE".into(), Secret::new(image.clone()));
        }
        let token = self.engine.accounts().account_token(&account.id).await?;
        let (reference, host) = adapter
            .provider_new_hoista_server(&token, &account.external_id, &source, &variables)
            .await?;
        drop(variables);
        // The key stays at the provider; make sure the provider won't show it again.
        let key_hidden = adapter
            .provider_hide_server_key(&token, &account.external_id, &reference)
            .await
            .unwrap_or(false);
        drop(token);
        let steps = (!key_hidden).then(|| adapter.server_key_steps(&reference));
        let seal_page = steps.clone().flatten().and_then(|(_, page)| page);
        let seal_help = steps.map(|steps| {
            steps.map_or_else(
                || {
                    format!(
                        "Make HOISTA_SEALED_KEY a secret {} won't show again, if it offers that.",
                        account.provider.display_name()
                    )
                },
                |(help, _)| help,
            )
        });
        self.engine
            .repository
            .event(
                actor.id(),
                None,
                "server.created",
                &format!(
                    "Put Hoista on {} at {host}, from {}.",
                    account.provider.display_name(),
                    match &source {
                        ServerSource::Image(image) => format!("the published image {image}"),
                        ServerSource::Repo { repo, .. } => repo.clone(),
                    }
                ),
            )
            .await?;
        let mut settings = self.engine.repository.settings().await?;
        settings.server = Some(ServerRecord {
            provider: Some(account.provider),
            account_id: Some(account.id.clone()),
            url: format!("https://{host}"),
            reference: Some(reference.clone()),
            image,
            created_at: now(),
        });
        self.engine.repository.put_settings(&settings).await?;
        Ok(ServerSetup {
            url: format!("https://{host}"),
            claim_link: format!("https://{host}/#code={code}"),
            reference,
            key_hidden,
            seal_help,
            seal_page,
        })
    }

    /// The repositories a code account (GitHub) can reach, newest pushed first.
    ///
    /// # Errors
    /// Reports missing permission, an account that keeps no code, or
    /// provider failure.
    pub async fn repositories(
        self,
        actor: &Actor,
        account_id: &str,
    ) -> Result<Vec<crate::ports::providers::ProviderRepository>, EngineError> {
        self.engine.authorize(actor, "apps:read").await?;
        let account = self
            .engine
            .repository
            .account(account_id)
            .await?
            .ok_or(EngineError::NotFound)?;
        let token = self.engine.accounts().account_token(&account.id).await?;
        self.engine
            .adapter(account.provider)?
            .repositories(&token, &account.external_id)
            .await
    }

    /// A repository's branches, through the code account that reaches it.
    ///
    /// # Errors
    /// Reports missing permission, unknown accounts or repositories, or
    /// provider failure.
    pub async fn branches(
        self,
        actor: &Actor,
        account_id: &str,
        repository: &str,
    ) -> Result<Vec<String>, EngineError> {
        self.engine.authorize(actor, "apps:read").await?;
        let account = self
            .engine
            .repository
            .account(account_id)
            .await?
            .ok_or(EngineError::NotFound)?;
        let token = self.engine.accounts().account_token(&account.id).await?;
        self.engine
            .adapter(account.provider)?
            .branches(&token, &account.external_id, repository)
            .await
    }

    /// What each connected provider has actually billed so far this period.
    /// Accounts whose provider gives no bill, or whose read fails, are left out:
    /// Hoista shows no money figure it can't get from the provider.
    ///
    /// # Errors
    /// Reports missing permission or state that can't be saved.
    pub async fn bills(self, actor: &Actor) -> Result<Vec<AccountBill>, EngineError> {
        self.engine.authorize(actor, "apps:read").await?;
        let mut bills = Vec::new();
        for account in self.engine.accounts().all_accounts().await? {
            if account.status != AccountStatus::Connected {
                continue;
            }
            let Ok(adapter) = self.engine.adapter(account.provider) else {
                continue;
            };
            let Ok(token) = self.engine.accounts().account_token(&account.id).await else {
                continue;
            };
            if let Ok(Some(bill)) = adapter.provider_bill(&token, &account.external_id).await {
                bills.push(AccountBill {
                    account_id: account.id,
                    provider: account.provider,
                    label: account.label,
                    bill,
                });
            }
        }
        Ok(bills)
    }

    /// Every connected account's billing as its provider shares it: the money
    /// billed so far this period, plan usage where the provider counts usage but
    /// not money, or why there is neither. Nothing is estimated.
    ///
    /// # Errors
    /// Reports missing permission or state that can't be saved.
    pub async fn billing(self, actor: &Actor) -> Result<Vec<AccountBilling>, EngineError> {
        self.engine.authorize(actor, "apps:read").await?;
        let mut rows = Vec::new();
        for account in self.engine.accounts().all_accounts().await? {
            let mut row = AccountBilling {
                account_id: account.id.clone(),
                provider: account.provider,
                label: account.label.clone(),
                bill: None,
                plan_usage: vec![],
                note: None,
            };
            if account.status != AccountStatus::Connected {
                row.note = Some("The token needs replacing before Hoista can read billing.".into());
                rows.push(row);
                continue;
            }
            let (Ok(adapter), Ok(token)) = (
                self.engine.adapter(account.provider),
                self.engine.accounts().account_token(&account.id).await,
            ) else {
                rows.push(row);
                continue;
            };
            let bill = adapter.provider_bill(&token, &account.external_id).await;
            let usage = adapter
                .provider_plan_usage(&token, &account.external_id)
                .await;
            let name = account.provider.display_name();
            match &bill {
                Ok(Some(found)) => row.bill = Some(found.clone()),
                Err(error) if error.code() == "provider.token_scope" => {
                    row.note = Some(format!(
                        "The token can't read {name} billing. Give it billing read access to see it here."
                    ));
                }
                _ => {}
            }
            if let Ok(lines) = usage {
                row.plan_usage = lines;
            }
            if row.bill.is_none() && row.plan_usage.is_empty() && row.note.is_none() {
                row.note = Some(match bill {
                    Err(ref error) if error.code() == "provider.not_supported" => {
                        format!("{name} doesn't share billing through its API. See it on {name}.")
                    }
                    Ok(None) => format!("{name} reports no charges for this account yet."),
                    _ => format!("{name} didn't answer about billing just now."),
                });
            }
            rows.push(row);
        }
        Ok(rows)
    }
}
