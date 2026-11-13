//! The credential keeper, as provider calls see it. On a server, provider
//! tokens stay with a separate keeper process; requests carry a handle naming
//! the token, and the keeper adds the token and makes the call.

use crate::{Provider, domain::Secret, ports::catalog::EngineError};
use std::path::Path;

/// A provider response as the keeper returned it.
pub struct KeptResponse {
    /// The HTTP status code.
    pub status: u16,
    /// The response headers the keeper passes back.
    pub headers: Vec<(String, String)>,
    /// The response body.
    pub body: Vec<u8>,
}

/// Makes provider calls with a token the caller never sees.
#[async_trait::async_trait]
pub trait CredentialKeeper: Send + Sync {
    /// Sends a provider request; the keeper signs it in with the token named by
    /// `key`. `http2` sends it over HTTP/2, as gRPC needs.
    ///
    /// # Errors
    /// Returns an error when the keeper can't be reached, refuses the request,
    /// or doesn't hold a token for `key`.
    async fn http(
        &self,
        key: &str,
        method: &str,
        url: &str,
        headers: Vec<(String, String)>,
        body: &[u8],
        http2: bool,
    ) -> Result<KeptResponse, EngineError>;

    /// Runs the provider's own command-line tool with the token named by `key`,
    /// and says whether it succeeded.
    ///
    /// # Errors
    /// Returns an error when the keeper can't be reached or refuses the command.
    async fn provider_cli(
        &self,
        provider: Provider,
        key: &str,
        args: &[&str],
        dir: Option<&Path>,
        stdin: Option<&Secret>,
    ) -> Result<bool, EngineError>;
}
