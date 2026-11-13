//! Checking on the environment admission key, and clearing setups that
//! didn't finish. Only the person can do either.

use crate::accounts::AccountService;
use crate::engine::Actor;
use crate::{
    environments::admission_keys::{self, EnvironmentKeyRecovery},
    ports::catalog::EngineError,
};

fn owner(actor: &Actor) -> Result<(), EngineError> {
    if matches!(actor, Actor::Person) {
        Ok(())
    } else {
        Err(EngineError::Invalid(
            "Environment-key recovery is available only to the local owner.",
        ))
    }
}

impl<'a> AccountService<'a> {
    /// Where the key setup stands. Changes nothing.
    ///
    /// # Errors
    /// Fails for anyone but the person, or when state can't be read.
    pub async fn environment_key_recovery(
        self,
        actor: &Actor,
    ) -> Result<EnvironmentKeyRecovery, EngineError> {
        owner(actor)?;
        admission_keys::inspect(self.engine.repository.as_ref(), self.engine.vault.as_ref()).await
    }

    /// Retires the unfinished setups the person reviewed, so a new one can
    /// start. Never replaces a working key, deletes a token or retries a deploy.
    ///
    /// # Errors
    /// Fails for anyone but the person, a review that's out of date, a key
    /// that already works, nothing to retire, or state that can't be read.
    pub async fn retry_environment_key_setup(
        self,
        actor: &Actor,
        review_token: &str,
    ) -> Result<EnvironmentKeyRecovery, EngineError> {
        owner(actor)?;
        let _mutation = self.engine.mutation.lock().await;
        self.engine
            .repository
            .retire_environment_key_setup(review_token)
            .await?;
        admission_keys::inspect(self.engine.repository.as_ref(), self.engine.vault.as_ref()).await
    }
}
