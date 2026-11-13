//! Log lines the demo providers report: the requests a service served, a few
//! failures among them, and a database's own messages. Lines are made from
//! their time, so reading the same range twice gives the same lines.

use crate::world::Resource;
use chrono::{DateTime, Duration, Timelike, Utc};
use hoista_server::application::ports::{catalog::EngineError, providers::LogSession};
use hoista_server::domain::{
    logs::{LogEntry, LogLevel},
    model::ResourceKind,
};

/// The most lines one read returns, newest kept.
const MAX_LINES: usize = 200;
/// How far apart lines are in a quiet stretch, and how often a live tail adds one.
const LINE_EVERY: Duration = Duration::seconds(37);
/// One request line in this many is a failure.
const FAILURE_EVERY: usize = 23;
const TAIL_EVERY: std::time::Duration = std::time::Duration::from_secs(3);

/// What a web service is asked for, with how long it took.
const REQUESTS: &[(&str, &str, u16, u32)] = &[
    ("GET", "/", 200, 31),
    ("GET", "/api/boards", 200, 44),
    ("POST", "/api/boards", 201, 88),
    ("GET", "/assets/app.js", 200, 6),
    ("GET", "/api/me", 200, 22),
    ("POST", "/api/upload", 200, 412),
    ("GET", "/health", 200, 3),
    ("GET", "/api/search?q=", 200, 61),
];

/// The rare failures, and what each one says.
const FAILURES: &[(&str, &str, u16, &str)] = &[
    (
        "POST",
        "/api/upload",
        413,
        "Payload Too Large: image is 14.2 MB, limit is 10 MB",
    ),
    (
        "GET",
        "/api/boards/9f2c",
        502,
        "upstream connect error: database is waking up",
    ),
    (
        "POST",
        "/api/login",
        429,
        "Too Many Requests from one address",
    ),
];

const DATABASE: &[&str] = &[
    "checkpoint complete: wrote 412 buffers (2.5%)",
    "connection received: host=10.0.4.12 port=51342",
    "connection authorized: user=app database=app",
    "automatic vacuum of table \"app.public.sessions\": index scans: 1",
    "compute woke up for a connection",
    "compute suspended after 5 minutes without connections",
];

const JOB: &[&str] = &[
    "Started scheduled run",
    "Cleaned up 128 expired sessions",
    "Sent 12 digest emails",
    "Finished in 3.2s",
];

/// The line `resource` wrote at `at`.
fn line(resource: &Resource, at: DateTime<Utc>) -> LogEntry {
    let tick = usize::try_from(at.timestamp() / LINE_EVERY.num_seconds()).unwrap_or(0);
    let (level, message) = match resource.kind {
        ResourceKind::Database => (LogLevel::Info, DATABASE[tick % DATABASE.len()].to_owned()),
        ResourceKind::Job => (LogLevel::Info, JOB[tick % JOB.len()].to_owned()),
        _ if tick % FAILURE_EVERY == 0 => {
            let (method, path, status, said) = FAILURES[tick / FAILURE_EVERY % FAILURES.len()];
            let level = if status >= 500 {
                LogLevel::Error
            } else {
                LogLevel::Warn
            };
            (level, format!("{method} {path} {status} · {said}"))
        }
        _ => {
            let (method, path, status, ms) = REQUESTS[tick % REQUESTS.len()];
            let ms = ms + at.second();
            (LogLevel::Info, format!("{method} {path} {status} {ms}ms"))
        }
    };
    LogEntry {
        timestamp: Some(at.to_rfc3339()),
        observed_at: Utc::now().to_rfc3339(),
        level,
        source: resource.name.to_owned(),
        message,
    }
}

/// The lines between `from` and `to`, oldest first, the newest `MAX_LINES`.
pub fn between(
    resource: &Resource,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    query: &str,
) -> Vec<LogEntry> {
    let step = LINE_EVERY.num_seconds();
    let first = from.timestamp().div_euclid(step) + 1;
    let last = to.timestamp().div_euclid(step);
    let wanted = query.to_lowercase();
    let mut lines: Vec<LogEntry> = (first..=last)
        .rev()
        .filter_map(|tick| DateTime::from_timestamp(tick * step, 0))
        .map(|at| line(resource, at))
        .filter(|entry| wanted.is_empty() || entry.message.to_lowercase().contains(&wanted))
        .take(MAX_LINES)
        .collect();
    lines.reverse();
    lines
}

/// A live tail: a new line every few seconds until it's closed.
pub struct Tail {
    resource: &'static Resource,
    query: String,
}

impl Tail {
    pub fn new(resource: &'static Resource, query: &str) -> Self {
        Self {
            resource,
            query: query.to_lowercase(),
        }
    }
}

#[async_trait::async_trait]
impl LogSession for Tail {
    async fn next(&mut self) -> Result<Option<LogEntry>, EngineError> {
        loop {
            tokio::time::sleep(TAIL_EVERY).await;
            let entry = line(self.resource, Utc::now());
            if self.query.is_empty() || entry.message.to_lowercase().contains(&self.query) {
                return Ok(Some(entry));
            }
        }
    }

    async fn close(&mut self) -> Result<(), EngineError> {
        Ok(())
    }
}
