//! Custom hostnames and their ACME certificates on Fly, through the Machines
//! API. Hoista never changes DNS; the person adds the records Fly asks for.

use super::{Fly, invalid};
use crate::{
    Provider,
    application::ports::catalog::EngineError,
    domain::{Secret, domains::*},
};
use reqwest::Method;
use serde_json::{Value, json};

const MAX_RECORD_VALUES: usize = 16;
const MAX_RECORD_BYTES: usize = 512;

impl Fly {
    async fn domain_owner(
        &self,
        token: &Secret,
        org: &str,
        app: &str,
        hostname: &str,
    ) -> Result<(), EngineError> {
        if !crate::domain::model::valid_slug(app) || !valid_hostname(hostname) {
            return Err(EngineError::Invalid(
                "Use a Fly app name and a non-wildcard ASCII hostname.",
            ));
        }
        let value = self
            .http
            .json(Provider::Fly, token, Method::GET, &["apps", app], &[], None)
            .await?;
        if value["name"].as_str() != Some(app)
            || value.pointer("/organization/slug").and_then(Value::as_str) != Some(org)
        {
            return Err(EngineError::Conflict(
                "The Fly app no longer belongs to the connected organization. Refresh account resources.",
            ));
        }
        Ok(())
    }

    pub(super) async fn configure_certificate(
        &self,
        token: &Secret,
        org: &str,
        app: &str,
        hostname: &str,
        zone: &str,
        attach: bool,
    ) -> Result<DomainObservation, EngineError> {
        self.domain_owner(token, org, app, hostname).await?;
        let current = self
            .http
            .json(
                Provider::Fly,
                token,
                Method::GET,
                &["apps", app, "certificates", hostname],
                &[],
                None,
            )
            .await;
        let value = match current {
            Ok(value) => {
                if value["hostname"].as_str() != Some(hostname) {
                    return Err(invalid());
                }
                self.http
                    .json(
                        Provider::Fly,
                        token,
                        Method::POST,
                        &["apps", app, "certificates", hostname, "check"],
                        &[],
                        None,
                    )
                    .await?
            }
            Err(error) if attach && error.code() == "provider.not_found" => {
                self.http
                    .json(
                        Provider::Fly,
                        token,
                        Method::POST,
                        &["apps", app, "certificates", "acme"],
                        &[],
                        Some(&json!({"hostname":hostname})),
                    )
                    .await?
            }
            Err(error) => return Err(error),
        };
        observation(&value, hostname, zone)
    }

    pub(super) async fn detach_certificate(
        &self,
        token: &Secret,
        org: &str,
        app: &str,
        hostname: &str,
    ) -> Result<(), EngineError> {
        self.domain_owner(token, org, app, hostname).await?;
        match self
            .http
            .json(
                Provider::Fly,
                token,
                Method::DELETE,
                &["apps", app, "certificates", hostname],
                &[],
                None,
            )
            .await
        {
            Ok(_) => Ok(()),
            // A retry after a lost success answers 404. The app and its owner
            // were checked above, so here 404 means the certificate is gone.
            Err(error) if error.code() == "provider.not_found" => Ok(()),
            Err(error) => Err(error),
        }
    }
}

fn observation(
    value: &Value,
    hostname: &str,
    zone: &str,
) -> Result<DomainObservation, EngineError> {
    if value["hostname"].as_str() != Some(hostname) {
        return Err(invalid());
    }
    let status = value["status"].as_str().ok_or_else(invalid)?;
    let https = match status {
        "active" if value["configured"].as_bool() == Some(true) => DomainReadiness::Active,
        "pending_validation" | "pending_ownership" | "pending" => DomainReadiness::Pending,
        "failed" | "expired" => DomainReadiness::Failing,
        _ => DomainReadiness::Unknown,
    };
    let dns = match value
        .pointer("/validation/dns_configured")
        .and_then(Value::as_bool)
    {
        Some(true) => DomainReadiness::Active,
        Some(false) => DomainReadiness::Pending,
        None => DomainReadiness::Unknown,
    };
    let mut dns_records = Vec::new();
    let mut warnings = vec!["DNS is managed outside Fly. Configure the applicable records at your DNS provider; no zone or nameservers were changed.".into()];
    // Only the DNS fields Hoista knows are kept, never Fly's error text or PEM
    // data. A/AAAA and CNAME are alternatives: set one, not both, at a name.
    for key in ["a", "aaaa"] {
        if let Some(values) = value.pointer(&format!("/dns_requirements/{key}")) {
            let values = values.as_array().ok_or_else(invalid)?;
            if values.len() > MAX_RECORD_VALUES {
                return Err(invalid());
            }
            for address in values {
                let address = safe_text(address)?;
                let ip = address.parse::<std::net::IpAddr>().map_err(|_| invalid())?;
                if (key == "a") != ip.is_ipv4() {
                    return Err(invalid());
                }
                dns_records.push(DomainDnsRecord {
                    name: hostname.into(),
                    record_type: if key == "a" {
                        DomainDnsRecordType::A
                    } else {
                        DomainDnsRecordType::AAAA
                    },
                    value: address.into(),
                    purpose: DomainDnsPurpose::Routing,
                });
                warnings.push(format!(
                    "DNS option: {hostname} {} {address}",
                    key.to_ascii_uppercase()
                ));
            }
        }
    }
    for (pointer, label) in [("/dns_requirements/cname", "CNAME")] {
        if let Some(target) = value.pointer(pointer).filter(|v| !v.is_null()) {
            let target = dns_target(target)?;
            dns_records.push(DomainDnsRecord {
                name: hostname.into(),
                record_type: DomainDnsRecordType::CNAME,
                value: target.into(),
                purpose: DomainDnsPurpose::Routing,
            });
            warnings.push(format!("DNS option: {hostname} {label} {}", target));
        }
    }
    for (key, kind, targets) in [
        ("acme_challenge", "CNAME", &["target"][..]),
        ("ownership", "TXT", &["app_value", "org_value"][..]),
    ] {
        if let Some(record) = value
            .pointer(&format!("/dns_requirements/{key}"))
            .filter(|v| !v.is_null())
        {
            let name = safe_text(&record["name"])?;
            let expected = if key == "acme_challenge" {
                format!("_acme-challenge.{hostname}")
            } else {
                format!("_fly-ownership.{hostname}")
            };
            if name.trim_end_matches('.').to_ascii_lowercase() != expected {
                return Err(invalid());
            }
            for target in targets {
                if let Some(data) = record.get(*target).filter(|v| !v.is_null()) {
                    let data = if key == "acme_challenge" {
                        dns_target(data)?
                    } else {
                        safe_text(data)?
                    };
                    dns_records.push(DomainDnsRecord {
                        name: name.into(),
                        record_type: if key == "acme_challenge" {
                            DomainDnsRecordType::CNAME
                        } else {
                            DomainDnsRecordType::TXT
                        },
                        value: data.into(),
                        purpose: if key == "acme_challenge" {
                            DomainDnsPurpose::AcmeChallenge
                        } else {
                            DomainDnsPurpose::Ownership
                        },
                    });
                    warnings.push(format!("Validation option: {name} {kind} {}", data));
                }
            }
        }
    }
    if value["validation_errors"]
        .as_array()
        .is_some_and(|errors| !errors.is_empty())
    {
        warnings.push("Fly reports validation errors. Check DNS records and inspect the hostname in Fly before retrying.".into());
    }
    if value
        .get("rate_limited_until")
        .is_some_and(|v| !v.is_null())
    {
        warnings.push("Fly reports certificate issuance rate limiting. Wait before requesting another certificate.".into());
    }
    Ok(DomainObservation {
        zone_id: String::new(),
        zone_name: zone.into(),
        nameservers: vec![],
        registrar: None,
        dns,
        https,
        provider_domain_id: Some(hostname.into()),
        certificate_id: None,
        dns_records,
        warnings,
    })
}

fn safe_text(value: &Value) -> Result<&str, EngineError> {
    value
        .as_str()
        .filter(|s| {
            !s.is_empty()
                && s.len() <= MAX_RECORD_BYTES
                && s.is_ascii()
                && !s.chars().any(char::is_control)
        })
        .ok_or_else(invalid)
}

// Accept a DNS absolute-name trailing dot, but never URLs or whitespace.
fn dns_target(value: &Value) -> Result<&str, EngineError> {
    let text = safe_text(value)?;
    if !valid_hostname(&text.strip_suffix('.').unwrap_or(text).to_ascii_lowercase()) {
        return Err(invalid());
    }
    Ok(text)
}
