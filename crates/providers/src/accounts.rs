//! Listing the accounts a token can see, before anything is saved.

use super::{answers::invalid, http::ProviderHttp};
use crate::{
    Provider,
    application::ports::catalog::EngineError,
    domain::{Secret, model::AccountChoice},
};
use reqwest::Method;
use serde_json::{Value, json};
use std::collections::HashSet;

const MAX_CHOICES: usize = 1000;
const MAX_PAGES: u64 = 100;
const MAX_CURSOR_BYTES: usize = 1024;

/// How a provider lists the accounts a token can reach, so one loop serves every
/// provider. Each adapter declares its own in its `ENTRY` entry.
pub(crate) struct AccountListing {
    /// The request for one page.
    pub fetch: Fetch,
    /// Where the rows are in each answer (empty: the answer itself).
    pub rows: &'static [&'static str],
    /// Where each account is inside its row (empty: the row itself).
    pub item: &'static [&'static str],
    /// The field naming each account.
    pub id: &'static str,
    /// How the answer says there's another page.
    pub pages: Pages,
}

/// One page's request.
pub(crate) enum Fetch {
    /// `GET` under the provider's API base, with fixed query pairs, then the page
    /// number and the cursor under these names when the provider pages that way.
    Get {
        path: &'static [&'static str],
        query: &'static [(&'static str, &'static str)],
        page: Option<&'static str>,
        cursor: Option<&'static str>,
    },
    /// A fixed GraphQL query, at `url` on one of the provider's API hosts or at
    /// its base, with the cursor in the variable `cursor` when it pages.
    Graphql {
        url: Option<&'static str>,
        query: &'static str,
        cursor: Option<&'static str>,
    },
}

/// How a provider says there's more.
pub(crate) enum Pages {
    /// Everything comes in one answer.
    One,
    /// A total page count at this path; pages are numbered from 1.
    Counted(&'static [&'static str]),
    /// `hasNextPage`-style flag and end cursor at these paths.
    Cursor {
        more: &'static [&'static str],
        end: &'static [&'static str],
    },
    /// A numeric next cursor at this path; null when there's no more.
    Next(&'static [&'static str]),
}

/// Follows a path of object keys; a missing key gives `Null`.
fn at<'a>(value: &'a Value, path: &[&str]) -> &'a Value {
    path.iter().fold(value, |value, key| &value[*key])
}

/// Every account the token can reach, following the provider's pages.
pub(super) async fn list(
    http: &ProviderHttp,
    provider: Provider,
    token: &Secret,
) -> Result<Vec<AccountChoice>, EngineError> {
    let listing = &super::entry(provider).accounts;
    let mut choices = Vec::new();
    let mut cursor = String::new();
    let mut cursors = HashSet::new();
    let mut identities = HashSet::new();
    for page in 1..=MAX_PAGES {
        let page_text = page.to_string();
        let value = match &listing.fetch {
            Fetch::Get {
                path,
                query,
                page: page_key,
                cursor: cursor_key,
            } => {
                let mut pairs: Vec<(&str, &str)> = query.to_vec();
                if let Some(key) = page_key {
                    pairs.push((key, &page_text));
                }
                if let Some(key) = cursor_key
                    && !cursor.is_empty()
                {
                    pairs.push((key, &cursor));
                }
                http.json(provider, token, Method::GET, path, &pairs, None)
                    .await?
            }
            Fetch::Graphql {
                url,
                query,
                cursor: variable,
            } => {
                let mut body = json!({ "query": query });
                if let Some(variable) = variable {
                    let mut variables = serde_json::Map::new();
                    variables.insert(
                        (*variable).to_owned(),
                        if cursor.is_empty() {
                            Value::Null
                        } else {
                            Value::String(cursor.clone())
                        },
                    );
                    body["variables"] = Value::Object(variables);
                }
                match url {
                    Some(url) => http.post_at(provider, token, url, &body, true).await?,
                    None => {
                        http.json(provider, token, Method::POST, &[], &[], Some(&body))
                            .await?
                    }
                }
            }
        };
        let rows = at(&value, listing.rows).as_array().ok_or_else(invalid)?;
        for row in rows {
            let row = at(row, listing.item);
            let id = row[listing.id].as_str().ok_or_else(invalid)?;
            let name = row["name"].as_str().ok_or_else(invalid)?;
            if !identities.insert(id.to_owned()) {
                return Err(invalid());
            }
            choices.push(AccountChoice {
                external_id: id.into(),
                name: name.into(),
            });
            if choices.len() > MAX_CHOICES {
                return Err(limit());
            }
        }
        let next = match &listing.pages {
            Pages::One => None,
            Pages::Counted(path) => {
                let pages = at(&value, path).as_u64().ok_or_else(invalid)?;
                (pages > 0 && page < pages).then(|| (page + 1).to_string())
            }
            Pages::Cursor { more, end } => match at(&value, more).as_bool() {
                Some(true) => Some(at(&value, end).as_str().ok_or_else(invalid)?.into()),
                Some(false) => None,
                None => return Err(invalid()),
            },
            Pages::Next(path) => {
                let parent = at(&value, &path[..path.len() - 1]);
                match parent.get(path[path.len() - 1]).ok_or_else(invalid)? {
                    Value::Null => None,
                    Value::String(s) if s.parse::<u64>().is_ok() => Some(s.clone()),
                    n if n.as_u64().is_some() => Some(n.to_string()),
                    _ => return Err(invalid()),
                }
            }
        };
        let Some(next) = next else {
            return Ok(choices);
        };
        if rows.is_empty()
            || next.is_empty()
            || next.len() > MAX_CURSOR_BYTES
            || !cursors.insert(next.clone())
        {
            return Err(invalid());
        }
        cursor = next;
    }
    Err(limit())
}

fn limit() -> EngineError {
    EngineError::Conflict(
        "Account lookup exceeded its limit. Connect using an explicit account ID.",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{Router, body::Bytes, extract::Request, response::IntoResponse};
    use http_body_util::BodyExt;
    use std::{
        collections::VecDeque,
        sync::{Arc, Mutex},
    };

    struct Reply {
        path: &'static str,
        body: Value,
        query: Option<&'static str>,
    }

    async fn run(
        provider: Provider,
        replies: Vec<Reply>,
    ) -> Result<Vec<AccountChoice>, EngineError> {
        let queue = Arc::new(Mutex::new(VecDeque::from(replies)));
        let remaining = queue.clone();
        let router = Router::new().fallback(move |request: Request| {
            let queue = queue.clone();
            async move {
                let reply = queue
                    .lock()
                    .unwrap()
                    .pop_front()
                    .expect("unexpected provider call");
                assert_eq!(request.uri().path_and_query().unwrap().as_str(), reply.path);
                assert_eq!(
                    request.headers()["authorization"],
                    "Bearer fixture-account-token"
                );
                assert_eq!(
                    request.method(),
                    if reply.query.is_some() { "POST" } else { "GET" }
                );
                let bytes: Bytes = request.into_body().collect().await.unwrap().to_bytes();
                if let Some(fragment) = reply.query {
                    let body: Value = serde_json::from_slice(&bytes).unwrap();
                    assert!(body["query"].as_str().unwrap().starts_with("query "));
                    assert!(body["query"].as_str().unwrap().contains(fragment));
                    if reply.path == "/graphql"
                        && reply.body["data"]["organizations"]["nodes"][0]["slug"] == "second"
                    {
                        assert_eq!(body["variables"]["after"], "cursor-one");
                    }
                } else {
                    assert!(bytes.is_empty());
                }
                axum::Json(reply.body).into_response()
            }
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let http = ProviderHttp::local_fixture(
            format!("http://{}/", listener.local_addr().unwrap())
                .parse()
                .unwrap(),
        );
        let shutdown = tokio_util::sync::CancellationToken::new();
        let stop = shutdown.clone();
        let server = tokio::spawn(async move {
            axum::serve(listener, router)
                .with_graceful_shutdown(stop.cancelled_owned())
                .await
                .unwrap();
        });
        let result = list(
            &http,
            provider,
            &Secret::new("fixture-account-token".into()),
        )
        .await;
        shutdown.cancel();
        server.await.unwrap();
        assert!(remaining.lock().unwrap().is_empty());
        result
    }

    #[tokio::test]
    async fn all_six_providers_list_account_identities_and_follow_pages() {
        for (provider, replies) in [
            (
                Provider::Cloudflare,
                vec![
                    Reply {
                        path: "/accounts?per_page=50&page=1",
                        body: json!({
                            "result": [{"id": "first", "name": "First"}],
                            "result_info": {"total_pages": 2},
                        }),
                        query: None,
                    },
                    Reply {
                        path: "/accounts?per_page=50&page=2",
                        body: json!({
                            "result": [{"id": "second", "name": "Second"}],
                            "result_info": {"total_pages": 2},
                        }),
                        query: None,
                    },
                ],
            ),
            (
                Provider::Fly,
                vec![
                    Reply {
                        path: "/graphql",
                        body: json!({
                            "data": {
                                "organizations": {
                                    "nodes": [
                                        {
                                            "slug": "first",
                                            "name": "First",
                                        },
                                    ],
                                    "pageInfo": {
                                        "hasNextPage": true,
                                        "endCursor": "cursor-one",
                                    },
                                },
                            },
                        }),
                        query: Some("organizations(first:100,after:$after)"),
                    },
                    Reply {
                        path: "/graphql",
                        body: json!({
                            "data": {
                                "organizations": {
                                    "nodes": [
                                        {
                                            "slug": "second",
                                            "name": "Second",
                                        },
                                    ],
                                    "pageInfo": {
                                        "hasNextPage": false,
                                        "endCursor": null,
                                    },
                                },
                            },
                        }),
                        query: Some("organizations(first:100,after:$after)"),
                    },
                ],
            ),
            (
                Provider::Vercel,
                vec![
                    Reply {
                        path: "/v2/teams?limit=100",
                        body: json!({"teams":[{"id":"first","name":"First"}],"pagination":{"next":123}}),
                        query: None,
                    },
                    Reply {
                        path: "/v2/teams?limit=100&until=123",
                        body: json!({
                            "teams": [{"id": "second", "name": "Second"}],
                            "pagination": {"next": null},
                        }),
                        query: None,
                    },
                ],
            ),
            (
                Provider::Neon,
                vec![Reply {
                    path: "/users/me/organizations",
                    body: json!({"organizations":[{"id":"first","name":"First"}]}),
                    query: None,
                }],
            ),
            (
                Provider::Supabase,
                vec![Reply {
                    path: "/organizations",
                    body: json!([{"id":"first","slug":"different-slug","name":"First"}]),
                    query: None,
                }],
            ),
            (
                Provider::Railway,
                vec![Reply {
                    path: "/",
                    body: json!({"data":{"me":{"workspaces":[{"id":"first","name":"First"}]}}}),
                    query: Some("me { workspaces { id name }"),
                }],
            ),
        ] {
            let choices = run(provider, replies).await.unwrap();
            assert_eq!(
                choices[0],
                AccountChoice {
                    external_id: "first".into(),
                    name: "First".into()
                }
            );
            assert_eq!(
                choices.len(),
                if matches!(
                    provider,
                    Provider::Cloudflare | Provider::Fly | Provider::Vercel
                ) {
                    2
                } else {
                    1
                }
            );
        }
    }

    #[tokio::test]
    async fn malformed_partial_duplicate_and_oversized_account_lists_fail() {
        for body in [
            json!({"teams":[],"pagination":{}}),
            json!({"teams":[],"pagination":{"next":10}}),
            json!({"teams":[{"id":"a","name":"A"},{"id":"a","name":"A"}],"pagination":{"next":null}}),
            json!({"teams":[{"id":"a"}],"pagination":{"next":null}}),
            json!({
                "teams": [],
                "pagination": {"next": null},
                "errors": [{"message": "fixture-account-token"}],
            }),
            json!({"teams":(0..1001).map(|i|json!({"id":i.to_string(),"name":"name"})).collect::<Vec<_>>(),"pagination":{"next":null}}),
        ] {
            let error = run(
                Provider::Vercel,
                vec![Reply {
                    path: "/v2/teams?limit=100",
                    body,
                    query: None,
                }],
            )
            .await
            .unwrap_err();
            assert!(!error.to_string().contains("fixture-account-token"));
        }
        let error = run(
            Provider::Vercel,
            vec![
                Reply {
                    path: "/v2/teams?limit=100",
                    body: json!({"teams":[{"id":"one","name":"One"}],"pagination":{"next":10}}),
                    query: None,
                },
                Reply {
                    path: "/v2/teams?limit=100&until=10",
                    body: json!({"teams":[{"id":"two","name":"Two"}],"pagination":{"next":"10"}}),
                    query: None,
                },
            ],
        )
        .await
        .unwrap_err();
        assert!(!error.to_string().contains("fixture-account-token"));
    }
}
