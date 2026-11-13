//! What the demo providers answer to an operation from their catalog, in each
//! provider's own shape. Fixed answers live in `answers.json`; anything that
//! changes with the asked-for time range is worked out here.
//!
//! A string such as `"@-40m"` or `"@+6d"` in `answers.json` is a time relative
//! to now (minutes, hours or days), so the demo never looks stale.

use chrono::{DateTime, Duration, Utc};
use hoista_server::domain::Provider;
use serde_json::{Value, json};
use std::sync::OnceLock;

/// The fixed answers, by provider slug, then operation name.
fn fixed() -> &'static Value {
    static ANSWERS: OnceLock<Value> = OnceLock::new();
    ANSWERS.get_or_init(|| {
        serde_json::from_str(include_str!("answers.json")).expect("answers.json is valid JSON")
    })
}

/// How many points a chart holds at most, whatever range is asked for.
pub const MAX_POINTS: i64 = 400;
/// A chart's step when none is asked for, and its finest.
const DEFAULT_STEP_SECONDS: i64 = 3600;
const MIN_STEP_SECONDS: i64 = 60;
/// Requests per minute behind the demo's HTTP charts.
const REQUESTS_PER_MINUTE: f64 = 40.0;

/// The answer to operation `name`, or None when the demo has no answer.
pub fn answer(provider: Provider, name: &str, args: &Value, now: DateTime<Utc>) -> Option<Value> {
    if let Some(generated) = generated(provider, name, args, now) {
        return Some(generated);
    }
    let found = fixed().get(provider.slug())?.get(name)?;
    Some(with_times(found.clone(), now))
}

/// What a provider's command-line tool prints for `name`, as the engine
/// reports a command run.
pub fn command(provider: Provider, name: &str, args: &Value, now: DateTime<Utc>) -> Value {
    let flags = args
        .as_object()
        .map(|args| {
            args.keys()
                .map(|key| format!("--{key}"))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let shown = std::iter::once(name.to_owned())
        .chain(flags)
        .collect::<Vec<_>>()
        .join(" ");
    let output = fixed()
        .get(provider.slug())
        .and_then(|answers| answers.get(name))
        .map(|output| with_times(output.clone(), now))
        .unwrap_or(Value::Null);
    json!({ "command": shown, "output": output, "messages": "Finished." })
}

/// `"@-40m"` and the like replaced by the times they stand for.
fn with_times(value: Value, now: DateTime<Utc>) -> Value {
    match value {
        Value::String(text) => relative(&text, now)
            .map(|at| Value::String(at.to_rfc3339()))
            .unwrap_or(Value::String(text)),
        Value::Array(items) => Value::Array(
            items
                .into_iter()
                .map(|item| with_times(item, now))
                .collect(),
        ),
        Value::Object(fields) => Value::Object(
            fields
                .into_iter()
                .map(|(key, item)| (key, with_times(item, now)))
                .collect(),
        ),
        other => other,
    }
}

fn relative(text: &str, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
    let rest = text.strip_prefix('@')?;
    let (sign, rest) = match rest.split_at_checked(1)? {
        ("-", rest) => (-1, rest),
        ("+", rest) => (1, rest),
        _ => return None,
    };
    let (count, unit) = rest.split_at_checked(rest.len().checked_sub(1)?)?;
    let count: i64 = count.parse().ok()?;
    let span = match unit {
        "m" => Duration::minutes(count),
        "h" => Duration::hours(count),
        "d" => Duration::days(count),
        _ => return None,
    };
    Some(if sign < 0 { now - span } else { now + span })
}

/// The answers that depend on the range asked for.
fn generated(provider: Provider, name: &str, args: &Value, now: DateTime<Utc>) -> Option<Value> {
    match (provider, name) {
        (Provider::Railway, "httpMetricsGroupedByStatus") => {
            let range = Range::asked(args, now);
            let per_sample = REQUESTS_PER_MINUTE * range.step.num_minutes() as f64;
            Some(json!([
                { "statusCode": 200, "samples": range.series(|i| (per_sample * wave(i, 1.0, 0.75)).round()) },
                { "statusCode": 404, "samples": range.series(|i| if i % 9 == 0 { 1.0 } else { 0.0 }) },
                { "statusCode": 502, "samples": range.series(|i| if i % 37 == 5 { 1.0 } else { 0.0 }) },
            ]))
        }
        (Provider::Railway, "httpDurationMetrics") => {
            let range = Range::asked(args, now);
            let samples: Vec<Value> = range
                .times()
                .enumerate()
                .map(|(i, at)| {
                    let p50 = wave(i, 38.0, 12.0);
                    json!({
                        "ts": at.to_rfc3339(),
                        "p50": p50,
                        "p90": p50 * 2.6,
                        "p95": wave(i, 150.0, 50.0),
                        "p99": wave(i, 520.0, 220.0),
                    })
                })
                .collect();
            Some(json!({ "samples": samples }))
        }
        (Provider::Railway, "metrics") => {
            let range = Range::asked(args, now);
            let asked: Vec<&str> = args["measurements"]
                .as_array()
                .map(|names| names.iter().filter_map(Value::as_str).collect())
                .unwrap_or_default();
            let measured: Vec<Value> = asked
                .into_iter()
                .filter_map(|measurement| {
                    let value: fn(usize) -> f64 = match measurement {
                        "CPU_USAGE" => |i| wave(i, 0.04, 0.08),
                        "CPU_LIMIT" => |_| 8.0,
                        "MEMORY_USAGE_GB" => |i| wave(i, 0.28, 0.06),
                        "MEMORY_LIMIT_GB" => |_| 8.0,
                        "NETWORK_RX_GB" => |i| wave(i, 0.002, 0.004),
                        "NETWORK_TX_GB" => |i| wave(i, 0.004, 0.006),
                        "DISK_USAGE_GB" => |_| 1.4,
                        _ => return None,
                    };
                    Some(json!({ "measurement": measurement, "values": range.series(value) }))
                })
                .collect();
            Some(Value::Array(measured))
        }
        (Provider::Supabase, "getProjectUsageApiCount") => {
            let result: Vec<Value> = (0..24)
                .map(|i| {
                    let x = f64::from(i);
                    json!({
                        "timestamp": (now - Duration::hours(i64::from(23 - i))).to_rfc3339(),
                        "total_rest_requests": (420.0 + 180.0 * (x / 3.0).sin() + f64::from(i % 5) * 12.0).round(),
                        "total_auth_requests": (60.0 + 25.0 * (x / 4.0 + 1.0).sin()).round(),
                        "total_storage_requests": (90.0 + 40.0 * (x / 3.0).cos()).round(),
                        "total_realtime_requests": 12 + (i % 4) * 3,
                    })
                })
                .collect();
            Some(json!({ "result": result }))
        }
        _ => None,
    }
}

/// A smooth, repeating rise and fall around `base`, so charts look lived in.
pub fn wave(i: usize, base: f64, swing: f64) -> f64 {
    let x = i as f64;
    base + swing * (0.5 + 0.5 * (x / 3.0).sin()) + swing * 0.3 * ((i * 7) % 5) as f64 / 5.0
}

/// The window an operation asked about, as its sample times.
struct Range {
    start: DateTime<Utc>,
    step: Duration,
    count: i64,
}

impl Range {
    fn asked(args: &Value, now: DateTime<Utc>) -> Self {
        let time = |key: &str| {
            args[key]
                .as_str()
                .and_then(|text| DateTime::parse_from_rfc3339(text).ok())
                .map(|at| at.with_timezone(&Utc))
        };
        let end = time("endDate").unwrap_or(now);
        let start = time("startDate").unwrap_or(end - Duration::days(1));
        let seconds = args["stepSeconds"]
            .as_i64()
            .or_else(|| args["sampleRateSeconds"].as_i64())
            .unwrap_or(DEFAULT_STEP_SECONDS)
            .max(MIN_STEP_SECONDS);
        let step = Duration::seconds(seconds);
        let count = ((end - start).num_seconds() / seconds).clamp(2, MAX_POINTS);
        Self { start, step, count }
    }

    fn times(&self) -> impl Iterator<Item = DateTime<Utc>> + '_ {
        (0..self.count).map(|i| self.start + self.step * i32::try_from(i).unwrap_or(0))
    }

    fn series(&self, value: impl Fn(usize) -> f64) -> Vec<Value> {
        self.times()
            .enumerate()
            .map(|(i, at)| json!({ "ts": at.to_rfc3339(), "value": value(i) }))
            .collect()
    }
}
