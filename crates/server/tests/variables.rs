//! A provider takes variables one way or another: on the service, or as secrets.
//! Hoista uses whichever it has, so a Worker's secrets can be set like any
//! other variable, without the provider's dashboard.

use hoista_server::{
    Provider,
    application::ports::{
        catalog::EngineError,
        providers::{ProviderAdapter, RemoteResource, put_variable, remove_variable},
    },
    domain::{Secret, model::*},
};
use std::sync::Mutex;

/// A provider that only has secrets, like a Cloudflare Worker.
#[derive(Default)]
struct SecretsOnly {
    calls: Mutex<Vec<String>>,
}

#[async_trait::async_trait]
impl ProviderAdapter for SecretsOnly {
    fn provider(&self) -> Provider {
        Provider::Cloudflare
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities::default()
    }

    async fn verify(&self, _: &Secret, _: &str) -> Result<Vec<String>, EngineError> {
        Ok(vec![])
    }

    async fn discover(&self, _: &Secret, _: &str) -> Result<Vec<RemoteResource>, EngineError> {
        Ok(vec![])
    }

    async fn set_secret(
        &self,
        _: &Secret,
        _: &str,
        reference: &str,
        name: &str,
        _: &Secret,
    ) -> Result<(), EngineError> {
        self.calls
            .lock()
            .expect("fixture")
            .push(format!("set {reference} {name}"));
        Ok(())
    }

    async fn delete_secret(
        &self,
        _: &Secret,
        _: &str,
        reference: &str,
        name: &str,
    ) -> Result<(), EngineError> {
        self.calls
            .lock()
            .expect("fixture")
            .push(format!("delete {reference} {name}"));
        Ok(())
    }
}

/// A provider with neither.
struct Neither;

#[async_trait::async_trait]
impl ProviderAdapter for Neither {
    fn provider(&self) -> Provider {
        Provider::Neon
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities::default()
    }

    async fn verify(&self, _: &Secret, _: &str) -> Result<Vec<String>, EngineError> {
        Ok(vec![])
    }

    async fn discover(&self, _: &Secret, _: &str) -> Result<Vec<RemoteResource>, EngineError> {
        Ok(vec![])
    }
}

#[tokio::test]
async fn a_provider_with_only_secrets_takes_variables_as_secrets() {
    let provider = SecretsOnly::default();
    let token = Secret::new("fixture-token".into());
    let value = Secret::new("value".into());
    put_variable(
        &provider,
        &token,
        "acct",
        "worker:edge",
        "API_TOKEN",
        &value,
    )
    .await
    .expect("set as a secret");
    remove_variable(&provider, &token, "acct", "worker:edge", "API_TOKEN")
        .await
        .expect("deleted as a secret");
    assert_eq!(
        *provider.calls.lock().expect("fixture"),
        ["set worker:edge API_TOKEN", "delete worker:edge API_TOKEN"]
    );
}

#[tokio::test]
async fn a_provider_with_neither_says_so() {
    let token = Secret::new("fixture-token".into());
    let value = Secret::new("value".into());
    let error = put_variable(&Neither, &token, "acct", "x", "A", &value)
        .await
        .expect_err("unsupported");
    assert!(hoista_server::application::ports::providers::is_unsupported(&error));
}
