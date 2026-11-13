//! A Worker's usage for one day from Cloudflare's GraphQL analytics: requests,
//! errors and total CPU time. A CPU percentile never stands in for the total.

use super::{Cloudflare, invalid, worker};
use crate::{
    Provider,
    application::ports::catalog::EngineError,
    domain::{Secret, model::*},
};
use reqwest::Method;
use serde_json::{Value, json};

const MAX_FIELD_NAME_BYTES: usize = 128;
const MICROS_PER_SECOND: i64 = 1_000_000;

pub(super) async fn read(
    adapter: &Cloudflare,
    token: &Secret,
    account: &str,
    resource: &Resource,
    day: &str,
) -> Result<Usage, EngineError> {
    let date = chrono::NaiveDate::parse_from_str(day, "%Y-%m-%d")
        .map_err(|_| EngineError::Invalid("Use a UTC calendar date."))?;
    let tomorrow = date.succ_opt().ok_or_else(invalid)?;
    // Usage here is a Worker's requests and CPU; a domain or bucket has none.
    if !resource.provider_ref.starts_with("worker:") {
        return Err(crate::application::ports::providers::unsupported());
    }
    let script = worker(&resource.provider_ref)?;
    // No dimensions: one total for the Worker's day, not a time series cut off at a limit.
    let query = "query HoistaUsage($account:string,$start:string,$end:string,$script:string){viewer{accounts(filter:{accountTag:$account}){workersInvocationsAdaptive(limit:1,filter:{scriptName:$script,datetime_geq:$start,datetime_lt:$end}){sum{requests errors __typename}}}}}";
    let variables = json!({
        "account": account,
        "start": format!("{date}T00:00:00Z"),
        "end": format!("{tomorrow}T00:00:00Z"),
        "script": script,
    });
    let value = adapter
        .http
        .json(
            Provider::Cloudflare,
            token,
            Method::POST,
            &["graphql"],
            &[],
            Some(&json!({"query":query,"variables":variables})),
        )
        .await?;
    let counts = decode(&value, resource, day, false)?;
    let Some(sum) = value.pointer("/data/viewer/accounts/0/workersInvocationsAdaptive/0/sum")
    else {
        return Ok(counts);
    };
    // The sum's type name comes from the answer, and the schema says whether it has
    // `cpuTimeUs` in microseconds: Cloudflare's analytics schema changes.
    let kind = sum
        .get("__typename")
        .and_then(Value::as_str)
        .filter(|name| !name.is_empty() && name.len() <= MAX_FIELD_NAME_BYTES)
        .ok_or_else(invalid)?;
    let schema = adapter
        .http
        .json(
            Provider::Cloudflare,
            token,
            Method::POST,
            &["graphql"],
            &[],
            Some(&json!({
                "query": "query HoistaUsageSchema($kind:String!){__type(name:$kind){fields{name description}}}",
                "variables": {"kind": kind},
            })),
        )
        .await?;
    if !cpu_sum_available(&schema)? {
        return Ok(counts);
    }
    let query = query.replace("__typename", "cpuTimeUs");
    // Read the counts again with the CPU sum, so all three come from one answer.
    let value = adapter
        .http
        .json(
            Provider::Cloudflare,
            token,
            Method::POST,
            &["graphql"],
            &[],
            Some(&json!({"query":query,"variables":variables})),
        )
        .await?;
    decode(&value, resource, day, true)
}

fn check_errors(value: &Value) -> Result<(), EngineError> {
    // GraphQL can answer 200 with partial data and errors; partial usage is never kept.
    match value.get("errors") {
        None | Some(Value::Null) => {}
        Some(Value::Array(errors)) if errors.is_empty() => {}
        _ => {
            return Err(EngineError::Provider {
                evidence: None,
                code: "usage.query_failed".into(),
                message: "Cloudflare could not supply complete usage measurements. Check the token can read Analytics, then try again; nothing was recorded.".into(),
            });
        }
    }
    Ok(())
}

fn cpu_sum_available(value: &Value) -> Result<bool, EngineError> {
    check_errors(value)?;
    let kind = value.pointer("/data/__type").ok_or_else(invalid)?;
    if kind.is_null() {
        return Ok(false);
    }
    let fields = kind
        .get("fields")
        .and_then(Value::as_array)
        .ok_or_else(invalid)?;
    Ok(fields.iter().any(|field| {
        field.get("name").and_then(Value::as_str) == Some("cpuTimeUs")
            && field
                .get("description")
                .and_then(Value::as_str)
                .is_some_and(|text| text.to_ascii_lowercase().contains("microseconds"))
    }))
}

fn decode(
    value: &Value,
    resource: &Resource,
    day: &str,
    cpu_available: bool,
) -> Result<Usage, EngineError> {
    check_errors(value)?;
    let accounts = value
        .pointer("/data/viewer/accounts")
        .and_then(Value::as_array)
        .ok_or_else(invalid)?;
    if accounts.len() != 1 {
        return Err(invalid());
    }
    let rows = accounts[0]
        .get("workersInvocationsAdaptive")
        .and_then(Value::as_array)
        .ok_or_else(invalid)?;
    if rows.len() > 1 {
        return Err(invalid());
    }
    let (requests, errors, cpu_us) = if let Some(row) = rows.first() {
        let sum = row.get("sum").ok_or_else(invalid)?;
        let integer = |key| {
            sum.get(key)
                .and_then(Value::as_i64)
                .filter(|value| *value >= 0)
                .ok_or_else(invalid)
        };
        let requests = integer("requests")?;
        let errors = integer("errors")?;
        let cpu_us = cpu_available.then(|| integer("cpuTimeUs")).transpose()?;
        if errors > requests || (requests == 0 && cpu_us.is_some_and(|cpu| cpu != 0)) {
            return Err(invalid());
        }
        (requests, errors, cpu_us)
    } else {
        (0, 0, Some(0))
    };
    Ok(Usage {
        resource_id: resource.id.clone(),
        day: day.into(),
        requests: Some(requests),
        errors: Some(errors),
        compute_seconds: cpu_us.map(|cpu| cpu / MICROS_PER_SECOND),
        storage_bytes: None,
        egress_bytes: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn resource() -> Resource {
        Resource {
            id: "resource".into(),
            project_id: Some("project".into()),
            kind: ResourceKind::Web,
            provider: Provider::Cloudflare,
            account_id: "account".into(),
            provider_ref: "worker:script".into(),
            name: "script".into(),
            region: None,
            status: ResourceStatus::Live,
            mode: Mode::Watching,
            url: None,
            group: None,
        }
    }

    fn response(sum: Value) -> Value {
        json!({
            "data": {
                "viewer": {
                    "accounts": [
                        {
                            "workersInvocationsAdaptive": [
                                {
                                    "sum": sum,
                                    "quantiles": {
                                        "cpuTimeP50": 1,
                                    },
                                },
                            ],
                        },
                    ],
                },
            },
            "errors": null,
        })
    }

    #[test]
    fn usage_uses_cpu_sum_and_rejects_partial_invalid_and_truncated_data() {
        let resource = resource();
        let sum = json!({"requests":1_000_000,"errors":20,"cpuTimeUs":1_000_000_000});
        let value = response(sum.clone());
        let result = decode(&value, &resource, "2026-01-01", true).unwrap();
        assert_eq!(result.compute_seconds, Some(1000));
        let unavailable = decode(
            &response(json!({"requests":100,"errors":0})),
            &resource,
            "2026-01-01",
            false,
        )
        .unwrap();
        assert_eq!(unavailable.requests, Some(100));
        assert!(unavailable.compute_seconds.is_none());
        for description in ["CPU time - milliseconds", "CPU time", ""] {
            assert!(!cpu_sum_available(&json!({"data":{"__type":{"fields":[{"name":"cpuTimeUs","description":description}]}}})).unwrap());
        }
        assert!(!cpu_sum_available(&json!({"data":{"__type":null}})).unwrap());

        // Each of these is refused; the fixture's `cpuTimeP50` never stands in for
        // a missing CPU sum.
        for (path, bad) in [
            ("/errors", json!([{"message":"private-provider-error"}])),
            (
                "/data/viewer/accounts/0/workersInvocationsAdaptive/0/sum/cpuTimeUs",
                json!(null),
            ),
            (
                "/data/viewer/accounts/0/workersInvocationsAdaptive/0/sum/cpuTimeUs",
                json!(1.5),
            ),
            (
                "/data/viewer/accounts/0/workersInvocationsAdaptive/0/sum/cpuTimeUs",
                json!(u64::MAX),
            ),
            (
                "/data/viewer/accounts/0/workersInvocationsAdaptive/0/sum/requests",
                json!(-1),
            ),
            (
                "/data/viewer/accounts/0/workersInvocationsAdaptive/0/sum/errors",
                json!(1_000_001),
            ),
            (
                "/data/viewer/accounts/0/workersInvocationsAdaptive",
                json!([{"sum":sum},{"sum":sum}]),
            ),
            ("/data/viewer/accounts", json!([])),
        ] {
            let mut value = value.clone();
            *value.pointer_mut(path).unwrap() = bad;
            let error = decode(&value, &resource, "2026-01-01", true).unwrap_err();
            assert!(!error.to_string().contains("private-provider-error"));
        }
        let empty =
            json!({"data":{"viewer":{"accounts":[{"workersInvocationsAdaptive":[]}]}},"errors":[]});
        let zero = decode(&empty, &resource, "2026-01-01", true).unwrap();
        assert_eq!(zero.requests, Some(0));
    }

    #[tokio::test]
    async fn usage_requests_one_dimensionless_daily_sum() {
        use crate::http::ProviderHttp;
        use axum::{Json, Router, routing::post};
        for cpu_available in [true, false] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let router = Router::new().route("/graphql",post(move |Json(body):Json<Value>|async move {
            if body["query"].as_str().unwrap().contains("__type(") {
                    assert_eq!(body["variables"]["kind"], "ObservedSumType");
                    return Json(json!({
                        "data": {
                            "__type": {
                                "fields": [
                                    {
                                        "name": "cpuTimeUs",
                                        "description": if cpu_available {"Sum of CPU time in microseconds"} else {"Unavailable"},
                                    },
                                ],
                            },
                        },
                    }));
                }
                assert_eq!(body["variables"],json!({
                    "account": "account",
                    "script": "script",
                    "start": "2026-02-28T00:00:00Z",
                    "end": "2026-03-01T00:00:00Z",
                }));
            let query = body["query"].as_str().unwrap();
            if query.contains("__typename") {
                return Json(response(json!({"requests":100,"errors":1,"__typename":"ObservedSumType"})));
            }
            assert!(cpu_available);
            assert!(query.contains("cpuTimeUs"));
            assert!(!query.contains("quantiles"));
            assert!(!query.contains("dimensions"));
            Json(response(json!({"requests":100,"errors":1,"cpuTimeUs":3_000_000})))
        }));
            let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
            let server = tokio::spawn(async move {
                axum::serve(listener, router)
                    .with_graceful_shutdown(async { stopped.await.unwrap() })
                    .await
                    .unwrap()
            });
            let adapter = Cloudflare::new(ProviderHttp::local_fixture(
                url::Url::parse(&format!("http://{address}/")).unwrap(),
            ));
            let usage = read(
                &adapter,
                &Secret::new("fixture".into()),
                "account",
                &resource(),
                "2026-02-28",
            )
            .await
            .unwrap();
            assert_eq!(usage.compute_seconds, cpu_available.then_some(3));
            stop.send(()).unwrap();
            server.await.unwrap();
        }
    }
}
