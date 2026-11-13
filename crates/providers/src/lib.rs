//! The provider adapters, and the HTTP client that only reaches their own APIs.
//!
//! Every provider Hoista connects to is one entry in `ENTRIES`, declared in its
//! own adapter file: where its API is, how it lists accounts, and how to
//! build its adapter. Nothing else in Hoista needs a line per provider.

// The adapters name the use cases and core types the way the rest of Hoista
// does: `crate::application::…` and `crate::domain::…`.
use hoista_app as application;
use hoista_app::is_pinned_image;
use hoista_domain as domain;
use hoista_domain::Provider;

use crate::{
    application::ports::{catalog::EngineError, providers::ProviderAdapter},
    domain::{Secret, model::Capabilities},
};
use std::sync::Arc;

pub mod http;
pub mod network_history;

// One module per provider.
pub mod cloudflare;
pub mod fly;
/// GitHub, where projects' code lives.
pub mod github;
pub mod modal;
pub mod neon;
mod railway;
mod railway_catalog;
mod railway_live;
pub mod render;
pub mod resend;
pub mod runpod;
pub mod supabase;
mod supabase_catalog;
pub mod vercel;

// What the adapters share.
mod accounts;
mod answers;
mod api_catalog;
mod cli;
mod files;
mod manifest;
mod metrics;
mod oauth;
mod units;

#[cfg(test)]
mod conformance;

pub use railway::Railway;

/// Where a provider's API is. Only these API hosts ever receive the provider's token.
pub struct ProviderApi {
    /// Base of every ordinary request.
    pub base: &'static str,
    /// Every host the adapter may call, including `base`'s. HTTPS only.
    pub hosts: &'static [&'static str],
    /// The provider explains refusals in words meant for its users, so they're
    /// passed on (with the token redacted) instead of a generic sentence.
    pub explains_refusals: bool,
    /// How the provider's token goes on a request.
    pub sign_in: SignIn,
}

/// The headers that sign one request in: name and value.
pub type SignInHeaders = Vec<(&'static str, String)>;

/// How a provider's token is attached to a request. Used both where Hoista holds
/// the token itself and by the credential keeper on a server, so a provider signs
/// in the same way everywhere.
pub enum SignIn {
    /// `Authorization: Bearer <token>`.
    Bearer,
    /// Headers the provider's own module builds from the stored token.
    Headers(fn(&Secret) -> Result<SignInHeaders, EngineError>),
}

impl SignIn {
    /// The headers that sign a request in with `token`.
    ///
    /// # Errors
    /// Reports a stored token the provider's module can't use.
    pub fn headers(&self, token: &Secret) -> Result<SignInHeaders, EngineError> {
        match self {
            Self::Bearer => Ok(vec![(
                "authorization",
                format!("Bearer {}", token.expose()),
            )]),
            Self::Headers(build) => build(token),
        }
    }
}

/// A provider's own command-line tool, for what its API doesn't do.
pub struct ProviderCli {
    /// The program, found where `hoist tools configure` says.
    pub program: &'static str,
    /// The environment variable it reads the token from.
    pub token_env: &'static str,
    /// The rest of its environment: no prompts, update checks or telemetry.
    pub env: &'static [(&'static str, &'static str)],
    /// The only commands (first arguments) the credential keeper runs on a
    /// server: the ones the adapter uses.
    pub keeper_commands: &'static [&'static str],
}

impl ProviderCli {
    /// Sets up `command` to run with `token`: a clean environment holding only
    /// what the tool needs, and its output discarded since it may hold secrets.
    pub fn prepare(&self, command: &mut tokio::process::Command, token: &Secret) {
        command
            .env_clear()
            .kill_on_drop(true)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        hoista_system::process::inherit_environment(command);
        command.env(self.token_env, token.expose());
        for (name, value) in self.env {
            command.env(name, value);
        }
    }
}

/// One provider: its API, how it lists accounts, and its adapter.
pub(crate) struct ProviderEntry {
    pub provider: Provider,
    /// Its mark as a small SVG, for pages Hoista serves.
    pub logo: &'static str,
    /// What its adapter can do, declared once; the adapter reports these.
    pub capabilities: Capabilities,
    /// Its command-line tool, for providers whose adapter runs one.
    pub cli: Option<ProviderCli>,
    pub api: ProviderApi,
    pub accounts: accounts::AccountListing,
    pub adapter: fn(http::ProviderHttp) -> Arc<dyn ProviderAdapter>,
    /// A fake of the provider for the conformance suite every adapter must pass.
    #[cfg(test)]
    pub fake: conformance::Fake,
}

/// Every provider Hoista connects to, in the order of [`Provider::ALL`].
pub(crate) const ENTRIES: &[ProviderEntry] = &[
    cloudflare::ENTRY,
    fly::ENTRY,
    railway::ENTRY,
    vercel::ENTRY,
    neon::ENTRY,
    supabase::ENTRY,
    render::ENTRY,
    resend::ENTRY,
    runpod::ENTRY,
    modal::ENTRY,
    github::ENTRY,
];

/// A provider's mark as a small SVG, for pages Hoista serves.
#[must_use]
pub fn logo(provider: Provider) -> &'static str {
    entry(provider).logo
}

/// A provider's entry. Every provider has one; a test checks it.
pub(crate) fn entry(provider: Provider) -> &'static ProviderEntry {
    ENTRIES
        .iter()
        .find(|entry| entry.provider == provider)
        .unwrap_or(&ENTRIES[0])
}

/// Every provider Hoista connects to and what its adapter can do, in the order
/// Hoista suggests them.
#[must_use]
pub fn provider_capabilities() -> Vec<(Provider, Capabilities)> {
    ENTRIES
        .iter()
        .map(|entry| (entry.provider, entry.capabilities.clone()))
        .collect()
}

/// Every API host Hoista calls, provider by provider.
#[must_use]
pub fn api_hosts() -> Vec<&'static str> {
    ENTRIES
        .iter()
        .flat_map(|entry| entry.api.hosts.iter().copied())
        .collect()
}

/// The provider whose API answers at the domain `name`.
pub(crate) fn entry_for_domain(name: &str) -> Option<&'static ProviderEntry> {
    ENTRIES.iter().find(|entry| entry.api.hosts.contains(&name))
}

/// How `provider`'s token goes on a request.
#[must_use]
pub fn sign_in(provider: Provider) -> &'static SignIn {
    &entry(provider).api.sign_in
}

/// How to sign a request in at the API host `name`, when a provider owns it.
#[must_use]
pub fn sign_in_for_host(name: &str) -> Option<&'static SignIn> {
    entry_for_domain(name).map(|entry| &entry.api.sign_in)
}

/// Whether `provider` runs containers, which are built from a Dockerfile.
#[must_use]
pub fn runs_containers(provider: Provider) -> bool {
    entry(provider).capabilities.containers
}

/// `provider`'s command-line tool, when the credential keeper may run
/// `command` (the first argument) with it on a server.
#[must_use]
pub fn keeper_cli(provider: Provider, command: &str) -> Option<&'static ProviderCli> {
    entry(provider)
        .cli
        .as_ref()
        .filter(|cli| cli.keeper_commands.contains(&command))
}

/// True for a host name in [`api_hosts`].
#[must_use]
pub fn allowed_host(name: &str) -> bool {
    entry_for_domain(name).is_some()
}

/// One adapter per provider.
#[must_use]
pub fn adapters(http: &http::ProviderHttp) -> Vec<Arc<dyn ProviderAdapter>> {
    ENTRIES
        .iter()
        .map(|entry| (entry.adapter)(http.clone()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_provider_has_one_provider_entry_in_order() {
        assert_eq!(
            ENTRIES
                .iter()
                .map(|entry| entry.provider)
                .collect::<Vec<_>>(),
            Provider::ALL.to_vec()
        );
        for entry in ENTRIES {
            let base = reqwest::Url::parse(entry.api.base).unwrap();
            assert_eq!(base.scheme(), "https");
            assert!(entry.api.hosts.contains(&base.host_str().unwrap()));
        }
    }
}
