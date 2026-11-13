//! Pieces every provider's metrics share: charts in Hoista's shape, lines added
//! up by time, and a sentence for what a provider couldn't answer.

use crate::application::ports::providers::{
    MetricSummary, MetricUnit, ProviderChart, ProviderMetrics, ProviderPoint, ProviderSeries,
};
use chrono::{DateTime, Utc};
use std::collections::BTreeMap;

/// A Unix time this large is in milliseconds; in seconds it would be past
/// the year 5000.
const MILLISECONDS_FROM: i64 = 100_000_000_000;
const HTTP_STATUSES: std::ops::Range<u16> = 100..600;

/// A time as the charts carry it.
pub(super) fn stamp(at: DateTime<Utc>) -> String {
    at.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// A provider's time as Hoista writes it, or None when it isn't one. Takes RFC
/// 3339 text and Unix seconds or milliseconds.
pub(super) fn time_of(value: &serde_json::Value) -> Option<String> {
    let at = match value {
        serde_json::Value::String(text) => DateTime::parse_from_rfc3339(text)
            .ok()
            .map(|at| at.with_timezone(&Utc))
            .or_else(|| {
                // `2026-11-01 10:00:00` and `2026-11-01`, read as UTC.
                chrono::NaiveDateTime::parse_from_str(text, "%Y-%m-%d %H:%M:%S")
                    .ok()
                    .or_else(|| {
                        chrono::NaiveDate::parse_from_str(text, "%Y-%m-%d")
                            .ok()
                            .and_then(|day| day.and_hms_opt(0, 0, 0))
                    })
                    .map(|at| at.and_utc())
            }),
        serde_json::Value::Number(number) => {
            let n = number.as_i64()?;
            if n > MILLISECONDS_FROM {
                DateTime::from_timestamp_millis(n)
            } else {
                DateTime::from_timestamp(n, 0)
            }
        }
        _ => None,
    }?;
    Some(stamp(at))
}

/// A number the provider sent as a number or as text.
pub(super) fn number(value: &serde_json::Value) -> Option<f64> {
    value
        .as_f64()
        .or_else(|| value.as_str().and_then(|text| text.parse().ok()))
        .filter(|n: &f64| n.is_finite())
}

/// Points from `(time, value)` pairs, adding up values at the same time and
/// putting them oldest first.
pub(super) fn points(pairs: impl IntoIterator<Item = (String, f64)>) -> Vec<ProviderPoint> {
    let mut by_time: BTreeMap<String, f64> = BTreeMap::new();
    for (at, value) in pairs {
        *by_time.entry(at).or_default() += value;
    }
    by_time
        .into_iter()
        .map(|(at, value)| ProviderPoint { at, value })
        .collect()
}

/// One line.
pub(super) fn line(name: &str, points: Vec<ProviderPoint>) -> ProviderSeries {
    ProviderSeries {
        name: name.to_owned(),
        points,
    }
}

/// A chart, or None when none of its lines has a point.
pub(super) fn chart(
    id: &str,
    label: &str,
    unit: MetricUnit,
    summary: MetricSummary,
    series: Vec<ProviderSeries>,
) -> Option<ProviderChart> {
    let series: Vec<ProviderSeries> = series
        .into_iter()
        .filter(|line| !line.points.is_empty())
        .collect();
    (!series.is_empty()).then(|| ProviderChart {
        id: id.to_owned(),
        label: label.to_owned(),
        unit,
        summary,
        series,
    })
}

/// Lines by HTTP status class (2xx, 3xx, 4xx, 5xx), from `(status, time,
/// count)` rows, in that order.
pub(super) fn by_status_class(
    rows: impl IntoIterator<Item = (u16, String, f64)>,
) -> Vec<ProviderSeries> {
    let mut classes: BTreeMap<u16, Vec<(String, f64)>> = BTreeMap::new();
    for (status, at, count) in rows {
        if HTTP_STATUSES.contains(&status) {
            classes.entry(status / 100).or_default().push((at, count));
        }
    }
    classes
        .into_iter()
        .map(|(class, pairs)| line(&format!("{class}xx"), points(pairs)))
        .collect()
}

/// The metrics for a range, with a sentence naming what couldn't be read.
pub(super) fn metrics(
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    charts: Vec<Option<ProviderChart>>,
    missing: &[&str],
    provider: &str,
) -> ProviderMetrics {
    let note = match missing {
        [] => None,
        [one] => Some(format!("{provider} didn't answer for {one}.")),
        [rest @ .., last] => Some(format!(
            "{provider} didn't answer for {} and {last}.",
            rest.join(", ")
        )),
    };
    ProviderMetrics {
        from: stamp(from),
        to: stamp(to),
        charts: charts.into_iter().flatten().collect(),
        note,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_add_up_by_time_and_statuses_by_class() {
        let lines = by_status_class([
            (200, "b".into(), 2.0),
            (201, "b".into(), 1.0),
            (200, "a".into(), 4.0),
            (503, "a".into(), 1.0),
            (0, "a".into(), 9.0),
        ]);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].name, "2xx");
        assert_eq!(lines[0].points[0].at, "a");
        assert_eq!(lines[0].points[1].value, 3.0);
        assert_eq!(lines[1].name, "5xx");
        assert!(
            chart(
                "x",
                "X",
                MetricUnit::Count,
                MetricSummary::Total,
                vec![line("a", vec![])]
            )
            .is_none()
        );
    }

    #[test]
    fn times_and_notes() {
        assert_eq!(
            time_of(&serde_json::json!(1_762_000_000)).as_deref(),
            Some("2025-11-01T12:26:40Z")
        );
        assert_eq!(
            time_of(&serde_json::json!("2026-11-01")).as_deref(),
            Some("2026-11-01T00:00:00Z")
        );
        let now = Utc::now();
        let m = metrics(now, now, vec![], &["CPU", "memory", "disk"], "Render");
        assert_eq!(
            m.note.as_deref(),
            Some("Render didn't answer for CPU, memory and disk.")
        );
    }
}
