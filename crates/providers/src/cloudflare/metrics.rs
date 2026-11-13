//! A Worker's and a domain's numbers over time, from Cloudflare's GraphQL
//! analytics: a Worker's requests, errors and CPU time, and a domain's
//! requests by status and bandwidth. The token needs Account Analytics (for
//! Workers) or Zone Analytics (for domains) read access.

use super::{Cloudflare, worker};
use crate::metrics::{by_status_class, chart, line, metrics, number, points, stamp, time_of};
use crate::{
    Provider,
    application::ports::{
        catalog::EngineError,
        providers::{MetricSummary as S, MetricUnit as U, ProviderChart, ProviderMetrics},
    },
    domain::Secret,
};
use chrono::{DateTime, Duration, Utc};
use reqwest::Method;
use serde_json::{Value, json};

/// Cloudflare's GraphQL time buckets: the finest one that keeps a range
/// within a few hundred points.
fn worker_bucket(from: DateTime<Utc>, to: DateTime<Utc>) -> &'static str {
    let span = to - from;
    if span <= Duration::hours(3) {
        "datetimeMinute"
    } else if span <= Duration::days(2) {
        "datetimeFifteenMinutes"
    } else {
        "datetimeHour"
    }
}

fn refused(value: &Value) -> Option<EngineError> {
    let errors = value["errors"]
        .as_array()
        .filter(|errors| !errors.is_empty())?;
    let text = errors
        .iter()
        .filter_map(|error| error["message"].as_str())
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase();
    Some(EngineError::Provider {
        evidence: None,
        code: if text.contains("auth") || text.contains("permission") {
            "provider.token_scope".into()
        } else {
            "provider.rejected".into()
        },
        message: if text.contains("auth") || text.contains("permission") {
            "Cloudflare needs analytics read access for this token to show metrics. Add \
             Account Analytics (and Zone Analytics for domains) and reconnect."
                .into()
        } else {
            "Cloudflare couldn't answer for this range. Try a shorter one.".into()
        },
    })
}

async fn ask(
    adapter: &Cloudflare,
    token: &Secret,
    query: &str,
    variables: Value,
) -> Result<Value, EngineError> {
    let value = adapter
        .http
        .json(
            Provider::Cloudflare,
            token,
            Method::POST,
            &["graphql"],
            &[],
            Some(&json!({"query": query, "variables": variables})),
        )
        .await?;
    match refused(&value) {
        Some(error) => Err(error),
        None => Ok(value),
    }
}

pub(super) async fn read(
    adapter: &Cloudflare,
    token: &Secret,
    account: &str,
    reference: &str,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> Result<ProviderMetrics, EngineError> {
    let (start, end) = (stamp(from), stamp(to));
    if let Some(zone) = reference.strip_prefix("zone:") {
        let daily = to - from > Duration::days(3);
        let (group, bucket) = if daily {
            ("httpRequests1dGroups", "date")
        } else {
            ("httpRequests1hGroups", "datetime")
        };
        // GraphQL refuses variables a query doesn't use, so each grouping
        // declares only its own.
        let (declared, filter, variables) = if daily {
            (
                "$startDay:Date,$endDay:Date",
                "date_geq:$startDay,date_leq:$endDay",
                json!({
                    "zone": zone,
                    "startDay": from.date_naive().to_string(),
                    "endDay": to.date_naive().to_string(),
                }),
            )
        } else {
            (
                "$start:Time,$end:Time",
                "datetime_geq:$start,datetime_lt:$end",
                json!({"zone": zone, "start": start, "end": end}),
            )
        };
        let query = format!(
            "query HoistaZoneMetrics($zone:string,{declared}){{viewer{{zones(filter:{{zoneTag:$zone}}){{{group}(limit:1000,filter:{{{filter}}},orderBy:[{bucket}_ASC]){{dimensions{{{bucket}}} sum{{requests cachedRequests bytes cachedBytes responseStatusMap{{edgeResponseStatus requests}}}}}}}}}}}}"
        );
        let value = ask(adapter, token, &query, variables).await?;
        let rows = value
            .pointer(&format!("/data/viewer/zones/0/{group}"))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        return Ok(metrics(
            from,
            to,
            zone_charts(&rows, bucket),
            &[],
            "Cloudflare",
        ));
    }
    let script = worker(reference)?;
    let bucket = worker_bucket(from, to);
    let query = format!(
        "query HoistaWorkerMetrics($account:string,$script:string,$start:Time,$end:Time){{viewer{{accounts(filter:{{accountTag:$account}}){{workersInvocationsAdaptive(limit:10000,filter:{{scriptName:$script,datetime_geq:$start,datetime_leq:$end}},orderBy:[{bucket}_ASC]){{dimensions{{{bucket}}} sum{{requests errors subrequests}} quantiles{{cpuTimeP50 cpuTimeP99}}}}}}}}}}"
    );
    let value = ask(
        adapter,
        token,
        &query,
        json!({"account": account, "script": script, "start": start, "end": end}),
    )
    .await?;
    let rows = value
        .pointer("/data/viewer/accounts/0/workersInvocationsAdaptive")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    Ok(metrics(
        from,
        to,
        worker_charts(&rows, bucket),
        &[],
        "Cloudflare",
    ))
}

/// A Worker's rows as charts. Each bucket can come back once per status,
/// so values at the same time are added up (and CPU times averaged).
fn worker_charts(rows: &[Value], bucket: &str) -> Vec<Option<ProviderChart>> {
    let at = |row: &Value| time_of(&row["dimensions"][bucket]);
    let sum = |field: &'static str| {
        points(
            rows.iter()
                .filter_map(|row| Some((at(row)?, number(&row["sum"][field])?))),
        )
    };
    // Each status gives its own percentile: keep the highest per bucket, a
    // fair worst case rather than a made-up mean of percentiles.
    let worst = |field: &'static str| {
        let mut by_time: std::collections::BTreeMap<String, f64> = Default::default();
        for row in rows {
            if let (Some(at), Some(us)) = (at(row), number(&row["quantiles"][field])) {
                let slot = by_time.entry(at).or_insert(0.0);
                *slot = slot.max(us / 1000.0);
            }
        }
        by_time
            .into_iter()
            .map(|(at, value)| crate::application::ports::providers::ProviderPoint { at, value })
            .collect()
    };
    vec![
        chart(
            "requests",
            "Requests",
            U::Count,
            S::Total,
            vec![
                line("Requests", sum("requests")),
                line("Errors", sum("errors")),
            ],
        ),
        chart(
            "subrequests",
            "Subrequests",
            U::Count,
            S::Total,
            vec![line("Subrequests", sum("subrequests"))],
        ),
        chart(
            "cpu_time",
            "CPU time",
            U::Milliseconds,
            S::Average,
            vec![
                line("p50", worst("cpuTimeP50")),
                line("p99", worst("cpuTimeP99")),
            ],
        ),
    ]
}

/// A domain's rows as charts: requests by status class and what was served.
fn zone_charts(rows: &[Value], bucket: &str) -> Vec<Option<ProviderChart>> {
    let at = |row: &Value| time_of(&row["dimensions"][bucket]);
    let sum = |field: &'static str| {
        points(
            rows.iter()
                .filter_map(|row| Some((at(row)?, number(&row["sum"][field])?))),
        )
    };
    let statuses = by_status_class(rows.iter().flat_map(|row| {
        let at = at(row);
        row["sum"]["responseStatusMap"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter_map(move |entry| {
                Some((
                    u16::try_from(entry["edgeResponseStatus"].as_u64()?).ok()?,
                    at.clone()?,
                    number(&entry["requests"])?,
                ))
            })
    }));
    let mut requests = statuses;
    requests.push(line("From cache", sum("cachedRequests")));
    vec![
        chart("requests", "Requests", U::Count, S::Total, requests),
        chart(
            "bandwidth",
            "Bandwidth",
            U::Bytes,
            S::Total,
            vec![
                line("Served", sum("bytes")),
                line("From cache", sum("cachedBytes")),
            ],
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worker_rows_add_up_by_time() {
        let rows = vec![
            json!({
                "dimensions": {"datetimeHour": "2026-11-01T10:00:00Z"},
                "sum": {"requests": 5, "errors": 1, "subrequests": 0},
                "quantiles": {"cpuTimeP50": 1200, "cpuTimeP99": 5000},
            }),
            json!({
                "dimensions": {"datetimeHour": "2026-11-01T10:00:00Z"},
                "sum": {"requests": 2, "errors": 0, "subrequests": 0},
                "quantiles": {"cpuTimeP50": 800, "cpuTimeP99": 9000},
            }),
        ];
        let charts: Vec<_> = worker_charts(&rows, "datetimeHour")
            .into_iter()
            .flatten()
            .collect();
        assert_eq!(charts[0].series[0].points[0].value, 7.0);
        let cpu = charts.iter().find(|c| c.id == "cpu_time").unwrap();
        assert_eq!(cpu.series[1].points[0].value, 9.0);
        assert_eq!(
            worker_bucket(Utc::now() - Duration::hours(1), Utc::now()),
            "datetimeMinute"
        );
    }

    #[test]
    fn domain_rows_split_by_status() {
        let rows = vec![json!({
            "dimensions": {"date": "2026-11-01"},
            "sum": {
                "requests": 10,
                "cachedRequests": 4,
                "bytes": 1000,
                "cachedBytes": 400,
                "responseStatusMap": [
                    {"edgeResponseStatus": 200, "requests": 9},
                    {"edgeResponseStatus": 404, "requests": 1},
                ],
            },
        })];
        let charts: Vec<_> = zone_charts(&rows, "date").into_iter().flatten().collect();
        let names: Vec<_> = charts[0].series.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["2xx", "4xx", "From cache"]);
        assert!(
            refused(&json!({"errors":[{"message":"not authorized for that account"}]})).is_some()
        );
    }
}
