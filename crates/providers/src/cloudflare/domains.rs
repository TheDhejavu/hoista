//! Custom domains for Workers: attaching a hostname (in an existing or new
//! zone), detaching it, and reading whether its certificate is ready.

use super::{Cloudflare, invalid, text, worker};
use crate::{
    Provider,
    application::ports::catalog::EngineError,
    domain::{Secret, domains::*},
};
use reqwest::Method;
use serde_json::{Value, json};

const PAGE_SIZE: usize = 50;
const MAX_ID_BYTES: usize = 256;

impl Cloudflare {
    pub(super) async fn configure_hostname(
        &self,
        token: &Secret,
        account: &str,
        reference: &str,
        hostname: &str,
        zone_name: &str,
        create_zone: bool,
    ) -> Result<DomainObservation, EngineError> {
        if !belongs_to_zone(hostname, zone_name) {
            return Err(EngineError::Invalid(
                "Enter a hostname and its containing DNS zone, without schemes or paths.",
            ));
        }
        let script = worker(reference)?;
        let zones = self
            .call(
                token,
                Method::GET,
                &["zones"],
                &[
                    ("name", zone_name),
                    ("account.id", account),
                    ("per_page", "50"),
                ],
                None,
            )
            .await?;
        let zones = zones.as_array().ok_or_else(invalid)?;
        if zones.len() > 1 {
            return Err(invalid());
        }
        let zone = if let Some(zone) = zones.first() {
            zone.clone()
        } else if create_zone {
            self.call(
                token,
                Method::POST,
                &["zones"],
                &[],
                Some(&json!({"name":zone_name,"account":{"id":account},"type":"full"})),
            )
            .await?
        } else {
            return Err(EngineError::Invalid(
                "Add this DNS zone to the selected Cloudflare account, or explicitly enable zone creation.",
            ));
        };
        if zone.get("name").and_then(Value::as_str) != Some(zone_name)
            || zone.pointer("/account/id").and_then(Value::as_str) != Some(account)
        {
            return Err(invalid());
        }
        let zone_id = text(&zone, "id")?.to_owned();
        let mut observation = DomainObservation {
            dns_records: vec![],
            zone_id: zone_id.clone(),
            zone_name: zone_name.into(),
            nameservers: zone
                .get("name_servers")
                .and_then(Value::as_array)
                .ok_or_else(invalid)?
                .iter()
                .map(|v| {
                    v.as_str()
                        .filter(|v| valid_hostname(v))
                        .map(str::to_owned)
                        .ok_or_else(invalid)
                })
                .collect::<Result<_, _>>()?,
            registrar: zone
                .get("original_registrar")
                .and_then(Value::as_str)
                .filter(|v| !v.is_empty() && v.len() <= MAX_ID_BYTES)
                .map(str::to_owned),
            dns: match zone.get("status").and_then(Value::as_str) {
                Some("active") => DomainReadiness::Active,
                Some("pending" | "initializing") => DomainReadiness::Pending,
                Some("deactivated" | "moved" | "deleted") => DomainReadiness::Failing,
                _ => DomainReadiness::Unknown,
            },
            https: DomainReadiness::Unknown,
            provider_domain_id: None,
            certificate_id: None,
            warnings: vec![],
        };
        if observation.dns != DomainReadiness::Active {
            return Ok(observation);
        }
        let domains = self
            .call(
                token,
                Method::GET,
                &["accounts", account, "workers", "domains"],
                &[("hostname", hostname)],
                None,
            )
            .await?;
        let domains = domains.as_array().ok_or_else(invalid)?;
        if domains.len() > 1 {
            return Err(invalid());
        }
        let domain = if let Some(domain) = domains.first() {
            if domain.get("hostname").and_then(Value::as_str) != Some(hostname)
                || domain.get("service").and_then(Value::as_str) != Some(script)
                || domain.get("zone_id").and_then(Value::as_str) != Some(zone_id.as_str())
            {
                return Err(EngineError::Conflict(
                    "This hostname already routes to another Worker. Detach it there before assigning it here.",
                ));
            }
            domain.clone()
        } else {
            self.call(
                token,
                Method::PUT,
                &["accounts", account, "workers", "domains"],
                &[],
                Some(&json!({
                    "hostname": hostname,
                    "service": script,
                    "zone_id": zone_id,
                    "enabled": true,
                    "previews_enabled": false,
                })),
            )
            .await?
        };
        if domain.get("hostname").and_then(Value::as_str) != Some(hostname)
            || domain.get("service").and_then(Value::as_str) != Some(script)
        {
            return Err(invalid());
        }
        observation.provider_domain_id = Some(text(&domain, "id")?.into());
        observation.certificate_id = domain
            .get("cert_id")
            .and_then(Value::as_str)
            .map(str::to_owned);
        if domain.get("enabled").and_then(Value::as_bool) != Some(true) {
            observation.https = DomainReadiness::Failing;
            observation
                .warnings
                .push("The custom hostname is disabled at Cloudflare.".into());
            return Ok(observation);
        }
        match self.hostname_certificate(token, &zone_id, hostname).await {
            Ok(status) => observation.https = status,
            Err(error) => observation.warnings.push(format!(
                "Certificate readiness could not be checked: {}. SSL and Certificates Read may be required.",
                error.code()
            )),
        }
        Ok(observation)
    }

    async fn hostname_certificate(
        &self,
        token: &Secret,
        zone: &str,
        hostname: &str,
    ) -> Result<DomainReadiness, EngineError> {
        for page in 1..=100 {
            let page_number = page.to_string();
            let value = self
                .http
                .json(
                    Provider::Cloudflare,
                    token,
                    Method::GET,
                    &["zones", zone, "ssl", "certificate_packs"],
                    &[
                        ("deploy", "production"),
                        ("status", "all"),
                        ("per_page", "50"),
                        ("page", &page_number),
                    ],
                    None,
                )
                .await?;
            let rows = value
                .get("result")
                .and_then(Value::as_array)
                .ok_or_else(invalid)?;
            for pack in rows {
                if pack.get("status").and_then(Value::as_str) != Some("active") {
                    continue;
                }
                for cert in pack
                    .get("certificates")
                    .and_then(Value::as_array)
                    .ok_or_else(invalid)?
                {
                    let unexpired = cert
                        .get("expires_on")
                        .and_then(Value::as_str)
                        .and_then(|v| chrono::DateTime::parse_from_rfc3339(v).ok())
                        .is_some_and(|v| v > chrono::Utc::now());
                    if cert.get("status").and_then(Value::as_str) == Some("active")
                        && unexpired
                        && cert
                            .get("hosts")
                            .and_then(Value::as_array)
                            .ok_or_else(invalid)?
                            .iter()
                            .any(|v| v.as_str().is_some_and(|pattern| covers(pattern, hostname)))
                    {
                        return Ok(DomainReadiness::Active);
                    }
                }
            }
            let pages = value
                .pointer("/result_info/total_pages")
                .and_then(Value::as_u64);
            if pages.is_some_and(|pages| page >= pages)
                || (pages.is_none() && rows.len() < PAGE_SIZE)
            {
                return Ok(DomainReadiness::Pending);
            }
        }
        Err(EngineError::Conflict(
            "Certificate results exceed the supported pagination limit.",
        ))
    }

    pub(super) async fn detach_hostname(
        &self,
        token: &Secret,
        account: &str,
        reference: &str,
        id: Option<&str>,
        hostname: &str,
    ) -> Result<(), EngineError> {
        let domain = if let Some(id) = id {
            match self
                .call(
                    token,
                    Method::GET,
                    &["accounts", account, "workers", "domains", id],
                    &[],
                    None,
                )
                .await
            {
                Err(EngineError::Provider { code, .. }) if code == "provider.not_found" => {
                    return Ok(());
                }
                result => result?,
            }
        } else {
            let values = self
                .call(
                    token,
                    Method::GET,
                    &["accounts", account, "workers", "domains"],
                    &[("hostname", hostname)],
                    None,
                )
                .await?;
            let values = values.as_array().ok_or_else(invalid)?;
            if values.is_empty() {
                return Ok(());
            }
            if values.len() != 1 {
                return Err(invalid());
            }
            values[0].clone()
        };
        if domain.get("service").and_then(Value::as_str) != Some(worker(reference)?)
            || domain.get("hostname").and_then(Value::as_str) != Some(hostname)
        {
            return Err(EngineError::Conflict(
                "The hostname is now assigned to another Worker; no remote changes were made.",
            ));
        }
        self.call(
            token,
            Method::DELETE,
            &[
                "accounts",
                account,
                "workers",
                "domains",
                text(&domain, "id")?,
            ],
            &[],
            None,
        )
        .await?;
        Ok(())
    }
}

fn covers(pattern: &str, hostname: &str) -> bool {
    pattern == hostname
        || pattern.strip_prefix("*.").is_some_and(|suffix| {
            hostname
                .split_once('.')
                .is_some_and(|(_, tail)| tail == suffix)
        })
}

#[cfg(test)]
mod tests {
    use super::covers;
    #[test]
    fn wildcard_certificates_cover_exactly_one_label() {
        assert!(covers("*.example.com", "app.example.com"));
        assert!(!covers("*.example.com", "a.b.example.com"));
        assert!(!covers("*.example.com", "example.com"));
        assert!(!covers("*.example.com", "appexample.com"));
    }
}
