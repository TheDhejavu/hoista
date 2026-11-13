//! Log lines from a provider, live or from its stored history. A line's time
//! is the provider's own; it's never made up from when Hoista received it.

/// The most a history search may cover.
const MAX_RANGE_DAYS: i64 = 31;
/// The last moment a time can be (the end of the year 9999), in Unix milliseconds.
const LATEST_MILLISECONDS: i64 = 253_402_300_799_999;
const MAX_QUERY_BYTES: usize = 256;
const MAX_CURSOR_BYTES: usize = 2048;
/// The longest message a log line may carry; providers' longer lines are cut.
pub const MAX_MESSAGE_BYTES: usize = 8192;

/// `message` cut to `MAX_MESSAGE_BYTES`, on a character boundary.
#[must_use]
pub fn clipped(mut message: String) -> String {
    if message.len() > MAX_MESSAGE_BYTES {
        let mut cut = MAX_MESSAGE_BYTES;
        while !message.is_char_boundary(cut) {
            cut -= 1;
        }
        message.truncate(cut);
    }
    message
}

/// One log line, with secrets taken out.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, schemars::JsonSchema, ts_rs::TS)]
pub struct LogEntry {
    /// When it happened, as the provider says. `null` when it doesn't say.
    pub timestamp: Option<String>,
    /// When Hoista received it.
    pub observed_at: String,
    /// How serious it is.
    pub level: LogLevel,
    /// Where in the resource it came from, such as the console or a request.
    pub source: String,
    /// The text, at most 8 KiB.
    pub message: String,
}

/// How serious a log line is, in the same four levels for every provider.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    serde::Serialize,
    serde::Deserialize,
    schemars::JsonSchema,
    ts_rs::TS,
)]
#[serde(rename_all = "snake_case")]
pub enum LogLevel {
    /// Detail for debugging.
    Debug,
    /// Normal activity.
    Info,
    /// Something worth a look.
    Warn,
    /// An error, or a request that failed.
    Error,
}

/// A search of a provider's stored logs. It can only find what the provider kept.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, schemars::JsonSchema, ts_rs::TS)]
#[serde(deny_unknown_fields)]
pub struct LogHistoryQuery {
    /// From this time (RFC 3339), no earlier than 1970.
    pub from: String,
    /// To this time (RFC 3339): after `from`, and at most 31 days later.
    pub to: String,
    /// Text to look for, matched as written. At most 256 bytes.
    #[serde(default)]
    pub query: String,
    /// From the last page's `next_cursor`, with the same resource and times.
    pub cursor: Option<String>,
}

/// One page of stored log lines. Never every request: providers sample and expire logs.
#[derive(Debug, serde::Serialize, serde::Deserialize, schemars::JsonSchema, ts_rs::TS)]
pub struct LogHistory {
    /// Up to 100 lines, with secrets taken out.
    pub items: Vec<LogEntry>,
    /// Pass this back for the next page. The last page can be empty.
    pub next_cursor: Option<String>,
    /// What share of lines the provider searched, when it says.
    pub query_sampling_level: Option<f64>,
    /// Always `false`: sampling, expiry and turned-off logging can leave lines out.
    pub coverage_complete: bool,
}

impl LogHistoryQuery {
    /// The search's start and end in Unix milliseconds, once it's checked.
    ///
    /// # Errors
    /// The times aren't RFC 3339, the range is backwards or longer than 31
    /// days, or the text or cursor is too long.
    pub fn milliseconds(&self) -> Result<(i64, i64), &'static str> {
        let from = chrono::DateTime::parse_from_rfc3339(&self.from)
            .map_err(|_| "Use RFC 3339 log history times.")?
            .timestamp_millis();
        let to = chrono::DateTime::parse_from_rfc3339(&self.to)
            .map_err(|_| "Use RFC 3339 log history times.")?
            .timestamp_millis();
        if from < 0
            || to <= from
            || to > LATEST_MILLISECONDS
            || to - from > MAX_RANGE_DAYS * 24 * 60 * 60 * 1000
        {
            return Err(
                "Log history requires an ordered range of at most 31 days after the Unix epoch.",
            );
        }
        if self.query.len() > MAX_QUERY_BYTES
            || self
                .cursor
                .as_ref()
                .is_some_and(|cursor| cursor.is_empty() || cursor.len() > MAX_CURSOR_BYTES)
        {
            return Err("Log history query or cursor exceeds its limit.");
        }
        Ok((from, to))
    }
}

#[cfg(test)]
mod history_tests {
    use super::*;
    #[test]
    fn history_ranges_and_inputs_are_bounded() {
        let mut query = LogHistoryQuery {
            from: "1970-01-01T00:00:00Z".into(),
            to: "1970-02-01T00:00:00Z".into(),
            query: String::new(),
            cursor: None,
        };
        assert!(query.milliseconds().is_ok());
        for to in [
            "invalid",
            "1970-01-01T00:00:00Z",
            "1970-02-01T00:00:00.001Z",
            "+10000-01-01T00:00:00Z",
        ] {
            query.to = to.into();
            assert!(query.milliseconds().is_err());
        }
        query.to = "1970-01-02T00:00:00Z".into();
        query.from = "1969-12-31T23:59:59Z".into();
        assert!(query.milliseconds().is_err());
        query.from = "1970-01-01T00:00:00Z".into();
        query.query = "x".repeat(257);
        assert!(query.milliseconds().is_err());
        query.query.clear();
        for cursor in [String::new(), "x".repeat(2049)] {
            query.cursor = Some(cursor);
            assert!(query.milliseconds().is_err());
        }
    }
}
