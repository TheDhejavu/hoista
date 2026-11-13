//! Past Worker logs from Cloudflare's Workers Observability, a page at a
//! time. A cursor only works for the resource, range and search it came from.

use super::{Cloudflare, invalid};
use crate::{
    application::ports::catalog::EngineError,
    domain::{
        Secret,
        leaks::LeakScanner,
        logs::{LogEntry, LogHistory, LogHistoryQuery, LogLevel},
    },
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use reqwest::Method;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

const PAGE_SIZE: usize = 100;
const MAX_CURSOR_BYTES: usize = 2048;
const MAX_ID_BYTES: usize = 512;
const MAX_MESSAGE_BYTES: usize = 8192;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Cursor {
    query: String,
    id: String,
}

fn cursor_error() -> EngineError {
    EngineError::Invalid(
        "Reuse a valid log history cursor with the same resource, range and search.",
    )
}

fn fingerprint(account: &str, script: &str, from: i64, to: i64, query: &str) -> String {
    format!(
        "{:x}",
        Sha256::digest(
            json!([account, script, from, to, query])
                .to_string()
                .as_bytes()
        )
    )
}

fn offset(cursor: Option<&str>, fingerprint: &str) -> Result<Option<String>, EngineError> {
    let Some(raw) = cursor else { return Ok(None) };
    if raw.len() > MAX_CURSOR_BYTES {
        return Err(cursor_error());
    }
    let bytes = URL_SAFE_NO_PAD.decode(raw).map_err(|_| cursor_error())?;
    let cursor: Cursor = serde_json::from_slice(&bytes).map_err(|_| cursor_error())?;
    if cursor.query != fingerprint || cursor.id.is_empty() || cursor.id.len() > MAX_ID_BYTES {
        return Err(cursor_error());
    }
    Ok(Some(cursor.id))
}

pub(super) async fn read(
    adapter: &Cloudflare,
    token: &Secret,
    account: &str,
    script: &str,
    query: &LogHistoryQuery,
) -> Result<LogHistory, EngineError> {
    let (from, to) = query.milliseconds().map_err(EngineError::Invalid)?;
    let fingerprint = fingerprint(account, script, from, to, &query.query);
    let offset = offset(query.cursor.as_deref(), &fingerprint)?;
    let mut body = json!({
        "queryId": "hoista-history",
        "timeframe": {"from": from, "to": to},
        "view": "events",
        "limit": 100,
        "dry": true,
        "ignoreSeries": true,
        "parameters": {
            "datasets": ["cloudflare-workers"],
            "filterCombination": "and",
            "filters": [
                {
                    "key": "$workers.scriptName",
                    "type": "string",
                    "operation": "eq",
                    "value": script,
                },
            ],
        },
    });
    if !query.query.is_empty() {
        body["parameters"]["needle"] = json!({"value":query.query,"isRegex":false});
    }
    if let Some(offset) = &offset {
        body["offset"] = json!(offset);
        body["offsetDirection"] = json!("next");
    }
    let value = adapter
        .call(
            token,
            Method::POST,
            &[
                "accounts",
                account,
                "workers",
                "observability",
                "telemetry",
                "query",
            ],
            &[],
            Some(&body),
        )
        .await?;
    decode(
        &value,
        token,
        script,
        from,
        to,
        &fingerprint,
        offset.as_deref(),
    )
}

fn decode(
    value: &Value,
    token: &Secret,
    script: &str,
    from: i64,
    to: i64,
    fingerprint: &str,
    offset: Option<&str>,
) -> Result<LogHistory, EngineError> {
    if value.pointer("/run/status").and_then(Value::as_str) != Some("COMPLETED") {
        return Err(EngineError::Provider {
            evidence: None,
            code: "logs.query_incomplete".into(),
            message: "The provider did not complete the stored-log query. Retry a narrower range."
                .into(),
        });
    }
    let rows = value
        .pointer("/events/events")
        .and_then(Value::as_array)
        .ok_or_else(invalid)?;
    if rows.len() > PAGE_SIZE {
        return Err(invalid());
    }
    let sampling = value
        .pointer("/statistics/abr_level")
        .filter(|v| !v.is_null())
        .map(|v| {
            v.as_f64()
                .filter(|v| v.is_finite() && *v >= 1.0)
                .ok_or_else(invalid)
        })
        .transpose()?;
    let scanner = LeakScanner::new().map_err(|_| invalid())?;
    let observed_at = chrono::Utc::now().to_rfc3339();
    let mut items = Vec::with_capacity(rows.len());
    let mut ids = std::collections::HashSet::new();
    let mut last = None;
    for row in rows {
        let id = row
            .pointer("/$metadata/id")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty() && id.len() <= MAX_ID_BYTES)
            .ok_or_else(invalid)?;
        let millis = row
            .get("timestamp")
            .and_then(Value::as_i64)
            .ok_or_else(invalid)?;
        if row.pointer("/$workers/scriptName").and_then(Value::as_str) != Some(script)
            || row.get("dataset").and_then(Value::as_str) != Some("cloudflare-workers")
            || millis < from
            || millis > to
            || Some(id) == offset
            || !ids.insert(id)
        {
            return Err(invalid());
        }
        let timestamp = chrono::DateTime::from_timestamp_millis(millis)
            .ok_or_else(invalid)?
            .to_rfc3339();
        let mut level = match row.pointer("/$metadata/level").and_then(Value::as_str) {
            Some("debug") => LogLevel::Debug,
            Some("warn" | "warning") => LogLevel::Warn,
            Some("error" | "fatal") => LogLevel::Error,
            None | Some("info" | "log") => LogLevel::Info,
            _ => return Err(invalid()),
        };
        // An invocation's metadata may hold URLs, headers and IPs; only its outcome is shown.
        let (source, mut message) =
            if row.pointer("/$metadata/type").and_then(Value::as_str) == Some("cf-worker-event") {
                (
                    "invocation",
                    json!(
                        row.pointer("/$workers/outcome")
                            .and_then(Value::as_str)
                            .unwrap_or("Stored invocation")
                    ),
                )
            } else {
                let source = row.get("source").ok_or_else(invalid)?;
                let message = if source.is_string() {
                    source.clone()
                } else {
                    source
                        .get("message")
                        .cloned()
                        .or_else(|| row.pointer("/$metadata/message").cloned())
                        .unwrap_or(json!("Stored event"))
                };
                ("stored", message)
            };
        if source == "invocation"
            && row
                .pointer("/$workers/outcome")
                .and_then(Value::as_str)
                .is_some_and(|outcome| outcome != "ok")
        {
            level = LogLevel::Error;
        }
        super::logs::sanitize(&mut message);
        let message = scanner.redact(
            &message
                .as_str()
                .map_or_else(|| message.to_string(), str::to_owned),
            std::slice::from_ref(token),
        );
        if message.len() > MAX_MESSAGE_BYTES {
            return Err(EngineError::Provider {
                evidence: None,
                code: "logs.limit".into(),
                message:
                    "A stored log record exceeds the eight KiB display limit. Narrow the query."
                        .into(),
            });
        }
        items.push(LogEntry {
            timestamp: Some(timestamp),
            observed_at: observed_at.clone(),
            level,
            source: source.into(),
            message,
        });
        last = Some(id);
    }
    let next_cursor = if rows.len() == 100 {
        let cursor = Cursor {
            query: fingerprint.into(),
            id: last.ok_or_else(invalid)?.into(),
        };
        Some(URL_SAFE_NO_PAD.encode(serde_json::to_vec(&cursor).map_err(|_| invalid())?))
    } else {
        None
    };
    Ok(LogHistory {
        items,
        next_cursor,
        query_sampling_level: sampling,
        coverage_complete: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn event(id: usize) -> Value {
        json!({
            "$metadata": {"id": format!("event-{id}"), "type": "cf-worker-log", "level": "info"},
            "dataset": "cloudflare-workers",
            "timestamp": 1000,
            "$workers": {
                "scriptName": "script",
                "event": {
                    "request": {
                        "headers": {
                            "Authorization": "must-not-appear",
                        },
                    },
                },
            },
            "source": {
                "message": ["provider-token", {"secret": "hidden", "safe": "hello"}],
                "request": {"url": "must-not-appear"},
            },
        })
    }

    fn response(rows: Vec<Value>) -> Value {
        json!({"run":{"status":"COMPLETED"},"events":{"events":rows},"statistics":{"abr_level":2}})
    }

    #[test]
    fn history_rejects_incomplete_foreign_duplicate_and_oversized_observations() {
        let token = Secret::new("provider-token".into());
        let parse = |value: &Value| decode(value, &token, "script", 0, 2000, "query", None);
        let page = parse(&response(vec![event(1)])).unwrap();
        let text = serde_json::to_string(&page).unwrap();
        for secret in ["provider-token", "hidden", "must-not-appear"] {
            assert!(!text.contains(secret));
        }
        assert!(text.contains("hello"));
        assert!(!page.coverage_complete);
        assert_eq!(page.query_sampling_level, Some(2.0));
        for (pointer, value) in [
            ("/run/status", json!("STARTED")),
            ("/events/events/0/$workers/scriptName", json!("other")),
            ("/events/events/0/timestamp", json!(3000)),
            ("/events/events/0/$metadata/id", json!("")),
            ("/events/events/0/source/message", json!("x".repeat(8193))),
            ("/statistics/abr_level", json!(0)),
        ] {
            let mut bad = response(vec![event(1)]);
            *bad.pointer_mut(pointer).unwrap() = value;
            assert!(parse(&bad).is_err(), "{pointer}");
        }
        assert!(parse(&response(vec![event(1), event(1)])).is_err());
        assert!(parse(&response((0..101).map(event).collect())).is_err());
        let mut invocation = event(1);
        invocation["$metadata"]["type"] = json!("cf-worker-event");
        invocation["$workers"]["outcome"] = json!("ok");
        let page = parse(&response(vec![invocation])).unwrap();
        assert_eq!(page.items[0].message, "ok");
        let mut failed = event(2);
        failed["$metadata"]["type"] = json!("cf-worker-event");
        failed["$workers"]["outcome"] = json!("exception");
        assert_eq!(
            parse(&response(vec![failed])).unwrap().items[0].level,
            LogLevel::Error
        );
    }

    #[tokio::test]
    async fn history_queries_paginate_with_bound_cursors_and_literal_filters() {
        use crate::http::ProviderHttp;
        use axum::{Json, Router, routing::post};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let router = Router::new().route(
            "/accounts/account/workers/observability/telemetry/query",
            post(|Json(body): Json<Value>| async move {
                assert_eq!(body["timeframe"], json!({"from":0,"to":2000}));
                assert_eq!(
                    body["parameters"]["filters"],
                    json!([
                        {
                            "key": "$workers.scriptName",
                            "type": "string",
                            "operation": "eq",
                            "value": "script",
                        },
                    ])
                );
                assert_eq!(
                    body["parameters"]["needle"],
                    json!({"value":"hello.*","isRegex":false})
                );
                assert_eq!(body["limit"], 100);
                assert_eq!(body["dry"], true);
                let rows = if body.get("offset").is_some() {
                    assert_eq!(body["offset"], "event-99");
                    assert_eq!(body["offsetDirection"], "next");
                    vec![event(100)]
                } else {
                    (0..100).map(event).collect()
                };
                Json(json!({"success":true,"result":response(rows)}))
            }),
        );
        let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
        let server = tokio::spawn(async move {
            axum::serve(listener, router)
                .with_graceful_shutdown(async {
                    stopped.await.unwrap();
                })
                .await
                .unwrap();
        });
        let adapter = Cloudflare::new(ProviderHttp::local_fixture(
            url::Url::parse(&format!("http://{address}/")).unwrap(),
        ));
        let token = Secret::new("provider-token".into());
        let mut query = LogHistoryQuery {
            from: "1970-01-01T00:00:00Z".into(),
            to: "1970-01-01T00:00:02Z".into(),
            query: "hello.*".into(),
            cursor: None,
        };
        let first = read(&adapter, &token, "account", "script", &query)
            .await
            .unwrap();
        assert_eq!(first.items.len(), 100);
        query.cursor = first.next_cursor;
        assert!(query.cursor.is_some());
        let second = read(&adapter, &token, "account", "script", &query)
            .await
            .unwrap();
        assert_eq!(second.items.len(), 1);
        assert!(second.next_cursor.is_none());
        assert!(
            read(&adapter, &token, "other", "script", &query)
                .await
                .is_err()
        );
        assert!(
            read(&adapter, &token, "account", "other", &query)
                .await
                .is_err()
        );
        query.to = "1970-01-01T00:00:03Z".into();
        assert!(
            read(&adapter, &token, "account", "script", &query)
                .await
                .is_err()
        );
        query.cursor = Some("invalid".into());
        assert!(
            read(&adapter, &token, "account", "script", &query)
                .await
                .is_err()
        );
        stop.send(()).unwrap();
        server.await.unwrap();
    }
}
