//! What Hoista keeps while an agent on another app (a chat app or a cloud
//! agent) connects to a remote with OAuth. Never a token itself.

use serde::{Deserialize, Serialize};

/// An app that has registered to connect. Public: it has no secret, and
/// proves itself with PKCE instead.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OAuthClient {
    /// Made by Hoista when the app registers.
    pub id: String,
    /// The name the person sees when asked to allow it.
    pub name: String,
    /// The only addresses Hoista sends the person back to, matched exactly.
    pub redirect_uris: Vec<String>,
}

/// A connection the person hasn't finished allowing yet. Only a hash of the
/// one-time code is kept.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OAuthPending {
    /// What the app asked to be allowed to do. If the person changes the
    /// agent's access before it finishes, the app has to start again.
    pub requested_scopes: Vec<String>,
    /// This request's id.
    pub id: String,
    /// The app asking.
    pub client_id: String,
    /// The agent made for this app, with access of its own.
    pub agent_id: String,
    /// Where to send the person back to; one of the app's registered addresses.
    pub redirect_uri: String,
    /// The app's own value, handed back to it untouched.
    pub state: String,
    /// The PKCE challenge (S256).
    pub challenge: String,
    /// The MCP address the token will be for.
    pub resource: String,
    /// SHA-256 of the one-time code.
    pub code_hash: String,
    /// The approval the person answers, once.
    pub approval_id: String,
    /// When the request lapses (RFC 3339).
    pub expires_at: String,
}

/// A connection the person allowed. Removing the agent ends it too.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OAuthGrant {
    /// The agent it belongs to.
    pub agent_id: String,
    /// The app holding the token.
    pub client_id: String,
    /// The MCP address the token is for.
    pub resource: String,
    /// Which signing key was current when it was granted; changing the key ends older grants.
    pub generation: String,
    /// When it stops working, however it's used (RFC 3339).
    pub expires_at: String,
}
