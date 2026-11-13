//! Numbers as providers send them, turned into the ones Hoista keeps: dollar
//! amounts as whole cents, and Unix times in seconds (often with a fraction)
//! as dates. A float out of range saturates rather than wraps.

use chrono::{DateTime, Utc};

/// A dollar amount as whole cents, rounded to the nearest cent.
pub(crate) fn cents(dollars: f64) -> i64 {
    (dollars * 100.0).round() as i64
}

/// A Unix time in seconds, fraction and all, as a date.
pub(crate) fn from_unix_seconds(seconds: f64) -> Option<DateTime<Utc>> {
    DateTime::from_timestamp_millis((seconds * 1000.0).round() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rounds_dollars_to_cents() {
        assert_eq!(cents(12.345), 1235);
        assert_eq!(cents(0.0), 0);
    }

    #[test]
    fn keeps_the_fraction_of_a_second() {
        let at = from_unix_seconds(1_700_000_000.25).unwrap();
        assert_eq!(at.timestamp_millis(), 1_700_000_000_250);
    }
}
