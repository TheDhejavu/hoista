//! The HTTP side shared by Hoista on a server and MCP over HTTP on a
//! computer: OAuth metadata, CORS preflight, and turning requests into
//! dispatcher requests and its replies back into responses.

use super::remote::{REVIEW_HEADER, RemoteRequest, RemoteResponse};
use axum::{
    Json,
    body::{Body, Bytes},
    extract::Request,
    http::{HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};

const MAX_QUERY_BYTES: usize = 8192;

/// The scopes an agent can ask for.
pub const SCOPES: [&str; 3] = ["apps:read", "apps:write", "apps:create"];

/// OAuth protected-resource metadata for the MCP endpoint at `origin`.
pub fn resource_metadata(origin: &str) -> Response {
    Json(serde_json::json!({
        "resource": format!("{origin}/mcp"),
        "authorization_servers": [origin],
        "scopes_supported": SCOPES,
    }))
    .into_response()
}

/// OAuth authorization-server metadata for `origin`.
pub fn server_metadata(origin: &str) -> Response {
    Json(serde_json::json!({
        "issuer": origin,
        "authorization_endpoint": format!("{origin}/oauth/authorize"),
        "token_endpoint": format!("{origin}/oauth/token"),
        "registration_endpoint": format!("{origin}/oauth/register"),
        "response_types_supported": ["code"],
        "grant_types_supported": ["authorization_code"],
        "code_challenge_methods_supported": ["S256"],
        "token_endpoint_auth_methods_supported": ["none"],
        "scopes_supported": SCOPES,
    }))
    .into_response()
}

/// Answers a browser's CORS preflight for the OAuth and MCP routes.
pub async fn preflight() -> Response {
    let mut response = StatusCode::NO_CONTENT.into_response();
    let headers = response.headers_mut();
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_METHODS,
        HeaderValue::from_static("GET, POST, DELETE"),
    );
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_HEADERS,
        HeaderValue::from_static("Authorization, Content-Type, MCP-Protocol-Version"),
    );
    headers.insert(
        header::ACCESS_CONTROL_MAX_AGE,
        HeaderValue::from_static("600"),
    );
    response
}

/// Reads a request, up to a size limit, keeping only the headers the dispatcher needs.
///
/// # Errors
/// The response to send instead, when the request is too large or not UTF-8.
pub async fn read(request: Request) -> Result<RemoteRequest, Response> {
    let (head, body) = request.into_parts();
    if head
        .uri
        .query()
        .is_some_and(|query| query.len() > MAX_QUERY_BYTES)
    {
        return Err((StatusCode::URI_TOO_LONG, "Request URL too large").into_response());
    }
    let Ok(bytes) = axum::body::to_bytes(body, crate::MAX_REQUEST_BYTES).await else {
        return Err((StatusCode::PAYLOAD_TOO_LARGE, "Request too large").into_response());
    };
    let Ok(body) = String::from_utf8(bytes.to_vec()) else {
        return Err((StatusCode::BAD_REQUEST, "Send UTF-8").into_response());
    };
    let headers = [
        "authorization",
        "origin",
        "content-type",
        "mcp-protocol-version",
        "accept",
    ]
    .iter()
    .filter_map(|name| {
        head.headers
            .get(*name)
            .and_then(|value| value.to_str().ok())
            .map(|value| ((*name).to_owned(), value.to_owned()))
    })
    .collect();
    Ok(RemoteRequest {
        id: uuid::Uuid::new_v4().to_string(),
        method: head.method.to_string(),
        path: head.uri.path_and_query().map_or_else(
            || head.uri.path().to_owned(),
            |path| path.as_str().to_owned(),
        ),
        headers,
        body,
    })
}

/// The HTTP response for a dispatcher's reply. The review header is for
/// this computer only and never goes out.
#[must_use]
pub fn respond(mut reply: RemoteResponse) -> Response {
    reply.headers.remove(REVIEW_HEADER);
    let mut response = Response::new(Body::from(Bytes::from(reply.body)));
    *response.status_mut() = StatusCode::from_u16(reply.status).unwrap_or(StatusCode::BAD_GATEWAY);
    for (name, value) in reply.headers {
        if let (Ok(name), Ok(value)) = (
            header::HeaderName::try_from(name),
            HeaderValue::try_from(value),
        ) {
            response.headers_mut().insert(name, value);
        }
    }
    response
}
