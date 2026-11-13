//! The accounts a token can see, for the person to pick from before
//! connecting. Saves nothing.

use crate::accounts::AccountService;
use crate::domain::leaks::LeakScanner;
use crate::{
    Provider,
    domain::{Secret, model::*},
    ports::catalog::EngineError,
};
use std::{collections::HashSet, time::Duration};

const MAX_TOKEN_BYTES: usize = 16 * 1024;
const LOOKUP_TIMEOUT: Duration = Duration::from_secs(25);
const MAX_CHOICES: usize = 1_000;
const MAX_ID_BYTES: usize = 256;
const MAX_NAME_BYTES: usize = 1024;

impl<'a> AccountService<'a> {
    /// The accounts `token` can see. The token isn't kept and nothing is saved.
    ///
    /// # Errors
    /// Fails for a token that's empty, too long or has control characters, a
    /// lookup that fails or takes too long, or an answer that looks wrong:
    /// too many accounts, repeated or blank ids, or the token echoed back.
    pub async fn account_choices(
        self,
        provider: Provider,
        token: Secret,
    ) -> Result<Vec<AccountChoice>, EngineError> {
        if token.expose().is_empty()
            || token.expose().len() > MAX_TOKEN_BYTES
            || token.expose().chars().any(char::is_control)
        {
            return Err(EngineError::Invalid(
                "Provide one token of at most 16 KiB without control characters.",
            ));
        }
        let choices = tokio::time::timeout(
            LOOKUP_TIMEOUT,
            self.engine.adapter(provider)?.account_choices(&token),
        )
        .await
        .map_err(|_| lookup_failed())?
        .map_err(|_| lookup_failed())?;
        if choices.len() > MAX_CHOICES {
            return Err(lookup_failed());
        }
        let scanner = LeakScanner::new().map_err(|_| EngineError::Store)?;
        let mut ids = HashSet::new();
        for choice in &choices {
            if choice.external_id.trim().is_empty()
                || choice.external_id.len() > MAX_ID_BYTES
                || choice.name.trim().is_empty()
                || choice.name.len() > MAX_NAME_BYTES
                || !ids.insert(&choice.external_id)
            {
                return Err(lookup_failed());
            }
            for text in [&choice.external_id, &choice.name] {
                if text.chars().any(char::is_control)
                    || !scanner
                        .scan(
                            "account selection",
                            text.as_bytes(),
                            std::slice::from_ref(&token),
                        )
                        .is_empty()
                {
                    return Err(lookup_failed());
                }
            }
        }
        Ok(choices)
    }
}

fn lookup_failed() -> EngineError {
    EngineError::Provider {
        evidence: None,
        code: "account.lookup_unavailable".into(),
        message: "Could not list accounts with this token. Check account-list permissions or connect using an explicit account ID.".into(),
    }
}
