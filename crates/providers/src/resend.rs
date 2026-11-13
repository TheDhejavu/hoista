//! Resend sending domains for one team, through Resend's API.
//!
//! The API never names the team a key belongs to, so the team is identified
//! by its oldest API key, which outlives newer ones.

use super::http::ProviderHttp;
use crate::{
    Provider,
    application::ports::{
        catalog::EngineError,
        providers::{Observed, ProviderAdapter, RemoteResource},
    },
    domain::{Secret, model::*},
};
use reqwest::Method;
use serde_json::Value;

/// Resend's API adapter, for the team its key reaches.
pub struct Resend {
    http: ProviderHttp,
}

const MAX_TEXT_CHARS: usize = 256;
/// DNS records shown for a sending domain.
const MAX_DNS_RECORDS: usize = 20;
/// Ranges up to this many days are charted by the hour, longer ones by the day.
const MAX_HOURLY_DAYS: i64 = 3;

/// The provider's mark, for pages Hoista serves.
const LOGO: &str = r##"<svg width="26" height="26" viewBox="0 0 24 24"><path d="M14.679 0c4.648 0 7.413 2.765 7.413 6.434s-2.765 6.434-7.413 6.434H12.33L24 24h-8.245l-8.88-8.44c-.636-.588-.93-1.273-.93-1.86 0-.831.587-1.565 1.713-1.883l4.574-1.224c1.737-.465 2.936-1.81 2.936-3.572 0-2.153-1.761-3.4-3.939-3.4H0V0z" fill="currentColor"/></svg>"##;

/// Resend's entry in the provider list. Resend has no list of teams, so the
/// adapter answers account choices itself; this listing is the team's keys,
/// which it reads to find the team.
pub(crate) const ENTRY: super::ProviderEntry = super::ProviderEntry {
    provider: Provider::Resend,
    logo: LOGO,
    capabilities: Capabilities {
        discover: true,
        provider_operations: true,
        metrics: true,
        logs: true,
        delete: true,
        new_email_domain: true,
        ..Capabilities::NONE
    },
    cli: None,
    api: super::ProviderApi {
        base: "https://api.resend.com/",
        hosts: &["api.resend.com"],
        explains_refusals: true,
        sign_in: super::SignIn::Bearer,
    },
    accounts: super::accounts::AccountListing {
        fetch: super::accounts::Fetch::Get {
            path: &["api-keys"],
            query: &[("limit", "100")],
            page: None,
            cursor: None,
        },
        rows: &["data"],
        item: &[],
        id: "id",
        pages: super::accounts::Pages::One,
    },
    adapter: |http| std::sync::Arc::new(Resend::new(http)),
    #[cfg(test)]
    fake: FAKE,
};

/// A fake of the provider for the conformance suite: enough to connect and
/// find one domain.
#[cfg(test)]
const FAKE: super::conformance::Fake = super::conformance::Fake {
    token: super::conformance::TOKEN,
    account: "key-1",
    resource: "dom-1",
    kind: crate::domain::model::ResourceKind::Email,
    routes: &[
        super::conformance::Route {
            path: "/api-keys",
            body: "",
            answer: r#"{"object":"list","has_more":false,"data":[{"id":"key-2","name":"new","created_at":"2026-10-02 10:00:00.000000+00"},{"id":"key-1","name":"first","created_at":"2026-01-02 10:00:00.000000+00"}]}"#,
        },
        super::conformance::Route {
            path: "/domains",
            body: "",
            answer: r#"{"object":"list","has_more":false,"data":[{"id":"dom-1","name":"mail.example.com","status":"verified","region":"us-east-1","created_at":"2026-01-02 10:00:00.000000+00"}]}"#,
        },
    ],
    empty: r#"{"object":"list","has_more":false,"data":[]}"#,
};

fn invalid() -> EngineError {
    super::answers::incomplete("Resend")
}

/// A key that can only send email can't see the team's domains or keys.
fn sending_only(error: EngineError) -> EngineError {
    match error {
        EngineError::Provider { code, .. }
            if code == "provider.token_scope" || code == "provider.rejected" =>
        {
            EngineError::Provider {
                evidence: None,
                code: "provider.token_scope".into(),
                message: "This Resend key can only send email. Create one with full access \
                          so Hoista can see your domains."
                    .into(),
            }
        }
        other => other,
    }
}

fn text(value: &Value, key: &str) -> Result<String, EngineError> {
    value[key]
        .as_str()
        .filter(|text| !text.is_empty() && text.len() <= MAX_TEXT_CHARS)
        .map(str::to_owned)
        .ok_or_else(invalid)
}

/// Resend's times (`2026-11-07 13:20:11.123456+00`) as RFC 3339, which
/// everything downstream reads; None when it isn't a time.
fn rfc3339(at: &str) -> Option<String> {
    let mut text = at.trim().replacen(' ', "T", 1);
    // A bare `+00` offset needs its minutes.
    if let Some(sign) = text.rfind(['+', '-']).filter(|at| *at > 10)
        && text.len() - sign == 3
    {
        text.push_str(":00");
    }
    chrono::DateTime::parse_from_rfc3339(&text)
        .ok()
        .map(|time| time.with_timezone(&chrono::Utc).to_rfc3339())
}

/// Resend's IDs: letters, digits and dashes.
fn plain_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

/// A domain's verification status as a resource status: a domain that
/// isn't verified can't send yet.
fn domain_status(status: &str) -> ResourceStatus {
    match status {
        "verified" => ResourceStatus::Live,
        "pending" | "partially_verified" => ResourceStatus::Deploying,
        "not_started" => ResourceStatus::NotDeployed,
        "failed" | "partially_failed" => ResourceStatus::Failing,
        _ => ResourceStatus::Unknown,
    }
}

impl Resend {
    /// Shares the allow-listed HTTP boundary.
    pub fn new(http: ProviderHttp) -> Self {
        Self { http }
    }

    /// Every row of one of Resend's lists, which page by the last row's ID.
    async fn every(&self, token: &Secret, path: &str) -> Result<Vec<Value>, EngineError> {
        let mut all: Vec<Value> = Vec::new();
        let mut after = String::new();
        for _ in 0..100 {
            let mut query = vec![("limit", "100")];
            if !after.is_empty() {
                query.push(("after", after.as_str()));
            }
            let value = self
                .http
                .json(Provider::Resend, token, Method::GET, &[path], &query, None)
                .await
                .map_err(sending_only)?;
            let rows = value["data"].as_array().ok_or_else(invalid)?;
            all.extend(rows.iter().cloned());
            if value["has_more"].as_bool() != Some(true) {
                return Ok(all);
            }
            let next = rows
                .last()
                .map(|row| text(row, "id"))
                .transpose()?
                .ok_or_else(invalid)?;
            if next == after {
                return Err(invalid());
            }
            after = next;
        }
        Err(invalid())
    }

    /// The IDs of the team's API keys, oldest first.
    async fn key_ids(&self, token: &Secret) -> Result<Vec<String>, EngineError> {
        let keys = self.every(token, "api-keys").await?;
        let mut keys = keys
            .iter()
            .map(|key| Ok((text(key, "created_at")?, text(key, "id")?)))
            .collect::<Result<Vec<_>, EngineError>>()?;
        keys.sort();
        Ok(keys.into_iter().map(|(_, id)| id).collect())
    }

    /// The team's oldest API key, which names the team in Hoista.
    async fn team(&self, token: &Secret) -> Result<String, EngineError> {
        self.key_ids(token)
            .await?
            .into_iter()
            .next()
            .filter(|id| plain_id(id))
            .ok_or_else(invalid)
    }
}

/// Resend's metric rows as charts: what was sent and how it went, and who
/// opened and clicked.
fn email_charts(value: &Value) -> Vec<Option<crate::application::ports::providers::ProviderChart>> {
    use super::metrics::{chart, line, number, points, time_of};
    use crate::application::ports::providers::{MetricSummary as S, MetricUnit as U};
    let rows = value["data"].as_array().cloned().unwrap_or_default();
    let series = |field: &str, name: &str| {
        line(
            name,
            points(rows.iter().filter_map(|row| {
                Some((time_of(&row["period"])?, number(&row[field]).unwrap_or(0.0)))
            })),
        )
    };
    vec![
        chart(
            "emails",
            "Emails",
            U::Count,
            S::Total,
            vec![
                series("sent", "Sent"),
                series("delivered", "Delivered"),
                series("bounced", "Bounced"),
                series("failed", "Failed"),
                series("complained", "Marked as spam"),
            ],
        ),
        chart(
            "engagement",
            "Opens and clicks",
            U::Count,
            S::Total,
            vec![series("opened", "Opened"), series("clicked", "Clicked")],
        ),
    ]
}

#[async_trait::async_trait]
impl ProviderAdapter for Resend {
    fn provider(&self) -> Provider {
        Provider::Resend
    }

    fn capabilities(&self) -> Capabilities {
        ENTRY.capabilities.clone()
    }

    /// Adds a domain to send from. Resend answers with the DNS records that
    /// prove it's yours; they're in the summary, to add at the domain's DNS.
    async fn provider_add(
        &self,
        token: &Secret,
        _team: &str,
        _project: &str,
        _beside: Option<&str>,
        what: &crate::application::ports::providers::ProviderNew<'_>,
    ) -> Result<(Vec<String>, String), EngineError> {
        let crate::application::ports::providers::ProviderNew::EmailDomain { domain } = *what
        else {
            return Err(crate::application::ports::providers::unsupported());
        };
        let domain = domain.trim().trim_end_matches('.').to_ascii_lowercase();
        if !crate::domain::domains::valid_hostname(&domain) {
            return Err(EngineError::Invalid(
                "Give the domain to send from, such as mail.example.com.",
            ));
        }
        let value = self
            .http
            .json(
                Provider::Resend,
                token,
                Method::POST,
                &["domains"],
                &[],
                Some(&serde_json::json!({"name": domain})),
            )
            .await?;
        let id = text(&value, "id")?;
        if !plain_id(&id) {
            return Err(invalid());
        }
        let records: Vec<String> = value["records"]
            .as_array()
            .map(|records| {
                records
                    .iter()
                    .filter_map(|record| {
                        Some(format!(
                            "{} {} {}",
                            record["type"].as_str()?,
                            record["name"].as_str()?,
                            record["value"].as_str()?
                        ))
                    })
                    .take(MAX_DNS_RECORDS)
                    .collect()
            })
            .unwrap_or_default();
        let summary = if records.is_empty() {
            format!("Added {domain} to Resend; its DNS records are on the domain's page in Resend")
        } else {
            format!(
                "Added {domain} to Resend. Add these DNS records, then Resend verifies it: {}",
                records.join("; ")
            )
        };
        Ok((vec![id], summary))
    }

    /// How much of the plan this account has used, as Resend counts it:
    /// emails this month and today, domains and contacts.
    async fn provider_plan_usage(
        &self,
        token: &Secret,
        _team: &str,
    ) -> Result<Vec<crate::application::ports::providers::ProviderPlanUsage>, EngineError> {
        use crate::application::ports::providers::ProviderPlanUsage;
        let usage = self
            .http
            .json(Provider::Resend, token, Method::GET, &["usage"], &[], None)
            .await?;
        let line = |label: &str, part: &serde_json::Value, unit: &str| {
            part["used"].as_f64().map(|used| ProviderPlanUsage {
                label: label.to_owned(),
                used,
                limit: part["limit"].as_f64(),
                unit: unit.to_owned(),
            })
        };
        Ok([
            line("Emails this month", &usage["emails"]["monthly"], "emails"),
            line("Emails today", &usage["emails"]["daily"], "emails"),
            line("Domains", &usage["domains"], "domains"),
            line("Contacts", &usage["contacts"], "contacts"),
        ]
        .into_iter()
        .flatten()
        .collect())
    }

    async fn account_choices(&self, token: &Secret) -> Result<Vec<AccountChoice>, EngineError> {
        Ok(vec![AccountChoice {
            external_id: self.team(token).await?,
            name: "Resend team".into(),
        }])
    }

    async fn verify(&self, token: &Secret, team: &str) -> Result<Vec<String>, EngineError> {
        if !plain_id(team) {
            return Err(EngineError::Invalid("A Resend team is required."));
        }
        // Same team when the key that names it is still one of the team's
        // keys, even if older ones have since gone or two share a time.
        if !self.key_ids(token).await?.iter().any(|id| id == team) {
            return Err(EngineError::Provider {
                evidence: None,
                code: "provider.token_scope".into(),
                message: "This key's Resend team doesn't have the API key Hoista knows the \
                          connected team by: it's another team, or that key was deleted in \
                          Resend. Connect this key as a new account."
                    .into(),
            });
        }
        Ok(vec!["Full access to the team (verified by request)".into()])
    }

    async fn discover(
        &self,
        token: &Secret,
        _team: &str,
    ) -> Result<Vec<RemoteResource>, EngineError> {
        let mut resources = Vec::new();
        for domain in self.every(token, "domains").await? {
            let id = text(&domain, "id")?;
            if !plain_id(&id) {
                return Err(invalid());
            }
            resources.push(RemoteResource {
                reference: id,
                name: text(&domain, "name")?,
                kind: ResourceKind::Email,
                region: domain["region"].as_str().map(str::to_owned),
                status: domain_status(domain["status"].as_str().unwrap_or_default()),
                url: None,
                group: None,
            });
        }
        if resources.len() > MAX_DISCOVERED_RESOURCES {
            return Err(invalid());
        }
        Ok(resources)
    }

    /// A key that can only send email, and only from this domain.
    async fn issue_key(
        &self,
        token: &Secret,
        _team: &str,
        reference: &str,
        name: &str,
    ) -> Result<Secret, EngineError> {
        if !plain_id(reference) {
            return Err(invalid());
        }
        let made = self
            .http
            .json(
                Provider::Resend,
                token,
                Method::POST,
                &["api-keys"],
                &[],
                Some(&serde_json::json!({
                    "name": name,
                    "permission": "sending_access",
                    "domain_id": reference,
                })),
            )
            .await?;
        made["token"]
            .as_str()
            .filter(|key| !key.is_empty())
            .map(|key| Secret::new(key.to_owned()))
            .ok_or_else(invalid)
    }

    fn key_variable(&self) -> Option<&'static str> {
        Some("RESEND_API_KEY")
    }

    /// The domain's logs are the emails sent from it: when, to whom, the
    /// subject and what happened to each (delivered, bounced, ...), newest
    /// first, from Resend's own list of sent emails.
    async fn log_history(
        &self,
        token: &Secret,
        _team: &str,
        reference: &str,
        query: &crate::domain::logs::LogHistoryQuery,
    ) -> Result<crate::domain::logs::LogHistory, EngineError> {
        use crate::domain::logs::{LogEntry, LogHistory, LogLevel};
        if !plain_id(reference) {
            return Err(invalid());
        }
        let (from, to) = query.milliseconds().map_err(EngineError::Invalid)?;
        let domain = self
            .http
            .json(
                Provider::Resend,
                token,
                Method::GET,
                &["domains", reference],
                &[],
                None,
            )
            .await?;
        let name = text(&domain, "name")?.to_lowercase();
        let sent = self
            .http
            .json(
                Provider::Resend,
                token,
                Method::GET,
                &["emails"],
                &[("limit", "100")],
                None,
            )
            .await?;
        let scanner = crate::domain::leaks::LeakScanner::new().map_err(|_| EngineError::Store)?;
        let observed = chrono::Utc::now().to_rfc3339();
        let needle = query.query.to_lowercase();
        let mut items = Vec::new();
        for email in sent["data"].as_array().into_iter().flatten() {
            let sender = email["from"].as_str().unwrap_or_default().to_lowercase();
            // Only this domain's mail: the address ends in @domain.
            let address = sender
                .rsplit_once('<')
                .map_or(sender.as_str(), |(_, rest)| rest.trim_end_matches('>'));
            if !address.ends_with(&format!("@{name}")) {
                continue;
            }
            let at = email["created_at"].as_str().and_then(rfc3339);
            let when = at
                .as_deref()
                .and_then(|at| chrono::DateTime::parse_from_rfc3339(at).ok())
                .map(|at| at.timestamp_millis());
            if when.is_some_and(|when| when < from || when > to) {
                continue;
            }
            let event = email["last_event"].as_str().unwrap_or("sent");
            let recipient = email["to"]
                .as_array()
                .and_then(|to| to.first())
                .and_then(Value::as_str)
                .unwrap_or_default();
            let subject = email["subject"].as_str().unwrap_or_default();
            let line = format!("{event} · from {address} · to {recipient} · {subject}");
            if !needle.is_empty() && !line.to_lowercase().contains(&needle) {
                continue;
            }
            let message =
                crate::domain::logs::clipped(scanner.redact(&line, std::slice::from_ref(token)));
            items.push(LogEntry {
                timestamp: at,
                observed_at: observed.clone(),
                level: match event {
                    "bounced" | "complained" | "failed" => LogLevel::Error,
                    "delivery_delayed" => LogLevel::Warn,
                    _ => LogLevel::Info,
                },
                source: "email".into(),
                message,
            });
        }
        Ok(LogHistory {
            items,
            next_cursor: None,
            query_sampling_level: None,
            coverage_complete: false,
        })
    }

    /// A domain is as its verification says; a deleted one is gone.
    async fn observe_resource(
        &self,
        token: &Secret,
        _team: &str,
        resource: &Resource,
    ) -> Result<Observed, EngineError> {
        let id = resource.provider_ref.as_str();
        if !plain_id(id) {
            return Err(invalid());
        }
        let status = match self
            .http
            .json(
                Provider::Resend,
                token,
                Method::GET,
                &["domains", id],
                &[],
                None,
            )
            .await
        {
            Ok(domain) => domain_status(domain["status"].as_str().unwrap_or_default()),
            Err(EngineError::Provider { code, .. }) if code == "provider.not_found" => {
                ResourceStatus::Stopped
            }
            Err(error) => return Err(error),
        };
        Ok(Observed {
            status,
            url: None,
            kind: Some(ResourceKind::Email),
        })
    }

    /// Resend's email counts for the domain: what was sent and what became
    /// of it, by the hour for short ranges and by the day for longer ones.
    async fn provider_metrics(
        &self,
        token: &Secret,
        _team: &str,
        reference: &str,
        from: chrono::DateTime<chrono::Utc>,
        to: chrono::DateTime<chrono::Utc>,
        _step_seconds: u32,
    ) -> Result<crate::application::ports::providers::ProviderMetrics, EngineError> {
        use super::metrics::stamp;
        if !plain_id(reference) {
            return Err(invalid());
        }
        let granularity = if to - from <= chrono::Duration::days(MAX_HOURLY_DAYS) {
            "hourly"
        } else {
            "daily"
        };
        let (start, end) = (stamp(from), stamp(to));
        let value = self
            .http
            .json(
                Provider::Resend,
                token,
                Method::GET,
                &["emails", "metrics"],
                &[
                    ("start_date", start.as_str()),
                    ("end_date", end.as_str()),
                    ("granularity", granularity),
                    ("dimensions", "period"),
                    ("domain_id", reference),
                    (
                        "metrics",
                        "sent,delivered,bounced,failed,complained,opened,clicked",
                    ),
                ],
                None,
            )
            .await?;
        Ok(super::metrics::metrics(
            from,
            to,
            email_charts(&value),
            &[],
            "Resend",
        ))
    }

    /// Removes the sending domain from Resend.
    async fn delete(&self, token: &Secret, _team: &str, id: &str) -> Result<(), EngineError> {
        if !plain_id(id) {
            return Err(invalid());
        }
        self.http
            .json(
                Provider::Resend,
                token,
                Method::DELETE,
                &["domains", id],
                &[],
                None,
            )
            .await?;
        Ok(())
    }

    fn provider_operations(
        &self,
    ) -> Vec<crate::application::ports::providers::ProviderOperationInfo> {
        super::manifest::builtin("resend").operations()
    }

    fn provider_quick_actions(
        &self,
    ) -> Vec<crate::application::ports::providers::ProviderQuickAction> {
        super::manifest::builtin("resend").actions.clone()
    }

    /// Resend's API, aimed at the domain.
    async fn provider_operation(
        &self,
        token: &Secret,
        _team: &str,
        reference: Option<&str>,
        _folder: Option<&std::path::Path>,
        name: &str,
        args: Value,
    ) -> Result<Value, EngineError> {
        let mut context = Vec::new();
        if let Some(id) = reference {
            context.push(("domain", id));
        }
        super::api_catalog::run_manifest(
            &self.http,
            Provider::Resend,
            super::manifest::builtin("resend"),
            token,
            &context,
            name,
            args,
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn email_counts_become_charts() {
        let value = serde_json::json!({
            "data": [
                {
                    "period": "2026-11-01T10:00:00.000Z",
                    "sent": 4,
                    "delivered": 3,
                    "bounced": 1,
                    "opened": 2,
                },
                {"period": "2026-11-01T11:00:00.000Z", "sent": 1, "delivered": 1},
            ],
        });
        let charts: Vec<_> = email_charts(&value).into_iter().flatten().collect();
        assert_eq!(charts[0].series[0].name, "Sent");
        assert_eq!(charts[0].series[0].points.len(), 2);
        assert_eq!(charts[1].series[1].points[1].value, 0.0);
    }

    #[test]
    fn a_domain_can_send_once_verified() {
        assert_eq!(domain_status("verified"), ResourceStatus::Live);
        assert_eq!(domain_status("pending"), ResourceStatus::Deploying);
        assert_eq!(domain_status("not_started"), ResourceStatus::NotDeployed);
        assert_eq!(domain_status("failed"), ResourceStatus::Failing);
        assert!(plain_id("4dd369bc-aa82-4ff3-97de-514ae3000ee0"));
        assert!(!plain_id("a/../b"));
    }

    #[test]
    fn resend_times_read_as_rfc3339() {
        assert_eq!(
            super::rfc3339("2026-11-07 13:20:11.123456+00").as_deref(),
            Some("2026-11-07T13:20:11.123456+00:00")
        );
        assert_eq!(
            super::rfc3339("2026-11-07T13:20:11Z").as_deref(),
            Some("2026-11-07T13:20:11+00:00")
        );
        assert_eq!(super::rfc3339("soon"), None);
    }

    #[test]
    fn the_api_is_aimed_at_the_domain_and_keeps_keys_out() {
        let manifest = super::super::manifest::builtin("resend");
        assert!(manifest.cli.is_none());
        let get = manifest.api_operation("domainsGet").unwrap();
        assert!(get.aimed_at("domain"));
        for name in [
            "apiKeysCreate",
            "apiKeysList",
            "oauthListGrants",
            "logsGet",
            "domainsRemove",
        ] {
            assert!(manifest.api_operation(name).is_none(), "{name}");
        }
        // Email can't be unsent.
        let send = manifest
            .operations()
            .into_iter()
            .find(|op| op.name == "emailsSend")
            .unwrap();
        assert_eq!(
            send.risk,
            crate::application::ports::providers::ProviderRisk::Destroy
        );
    }
}
