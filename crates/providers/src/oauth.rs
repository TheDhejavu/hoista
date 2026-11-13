//! The OAuth token exchange for provider sign-in. Hoista is a public client, so it
//! sends only its client id and the PKCE verifier, never a secret.
//!
//! Client ids come from `HOISTA_<PROVIDER>_CLIENT_ID` at build time; a provider without
//! one is connected with a pasted token instead.

use super::http::ProviderHttp;
use crate::{
    application::ports::{
        catalog::EngineError,
        providers::{OAuthGrant, OAuthTokens},
    },
    domain::Secret,
};

const MAX_SCOPES: usize = 100;
/// An access token's lifetime when the provider doesn't say, and the range
/// a stated one is kept within, in seconds.
const DEFAULT_LIFETIME: i64 = 60 * 60;
const LIFETIME: std::ops::RangeInclusive<i64> = 60..=24 * 60 * 60;
const MAX_SCOPE_BYTES: usize = 100;

/// Trades a code or refresh token at `token_url`, as the public client `client_id`.
///
/// # Errors
/// Fails when the provider refuses, can't be reached, or answers without a token.
pub(super) async fn public_token(
    http: &ProviderHttp,
    token_url: &str,
    client_id: &str,
    grant: OAuthGrant<'_>,
) -> Result<OAuthTokens, EngineError> {
    let form: Vec<(&str, &str)> = match grant {
        OAuthGrant::Code {
            code,
            verifier,
            redirect_uri,
        } => vec![
            ("grant_type", "authorization_code"),
            ("code", code),
            ("code_verifier", verifier),
            ("redirect_uri", redirect_uri),
            ("client_id", client_id),
        ],
        OAuthGrant::Refresh(refresh) => vec![
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh),
            ("client_id", client_id),
        ],
    };
    let value = http.oauth_form(token_url, &form).await?;
    let access = value["access_token"]
        .as_str()
        .filter(|token| !token.is_empty())
        .ok_or_else(super::answers::invalid)?;
    Ok(OAuthTokens {
        access: Secret::new(access.to_owned()),
        refresh: value["refresh_token"]
            .as_str()
            .filter(|token| !token.is_empty())
            .map(|token| Secret::new(token.to_owned())),
        expires_in: value["expires_in"]
            .as_i64()
            .unwrap_or(DEFAULT_LIFETIME)
            .clamp(*LIFETIME.start(), *LIFETIME.end()),
        granted: value["scope"]
            .as_str()
            .map(|scope| {
                scope
                    .split([' ', ','])
                    .filter(|scope| {
                        !scope.is_empty()
                            && scope.len() <= MAX_SCOPE_BYTES
                            && scope.bytes().all(|b| b.is_ascii_graphic())
                    })
                    .take(MAX_SCOPES)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default(),
    })
}

/// The provider's sign-in isn't set up in this build.
pub(super) fn not_set_up() -> EngineError {
    EngineError::Conflict(
        "This provider connects with a token; Hoista can't sign in on its page yet.",
    )
}
