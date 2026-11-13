//! Listing R2 buckets (in every jurisdiction) and domains (zones). Being listed
//! doesn't make a resource changeable: each change checks its reference itself.

use super::{Cloudflare, invalid, limit, text};
use crate::{
    Provider,
    application::ports::{
        catalog::EngineError,
        providers::{DiscoveryReport, RemoteResource},
    },
    domain::{Secret, model::*},
};
use reqwest::Method;
use serde_json::Value;
use std::collections::HashSet;

/// The most resources one account's inventory can hold.
const MAX_RESOURCES: usize = 10_000;
const MAX_PAGE_ROWS: usize = 1000;
/// Zones come 50 to a page.
const ZONE_PAGE: usize = 50;
const MAX_ZONE_PAGES: u64 = 200;
const MAX_CURSOR_BYTES: usize = 4096;
const MAX_ID_BYTES: usize = 128;
const MAX_LOCATION_CHARS: usize = 64;

impl Cloudflare {
    /// Lists R2 buckets in one jurisdiction, sent as Cloudflare's
    /// `cf-r2-jurisdiction` header.
    async fn r2_buckets(
        &self,
        token: &Secret,
        account: &str,
        jurisdiction: &str,
        query: &[(&str, &str)],
    ) -> Result<Value, EngineError> {
        if !matches!(
            jurisdiction,
            "default" | "eu" | "us" | "fedramp" | "fedramp-high"
        ) {
            return Err(EngineError::Invalid("Unsupported R2 jurisdiction."));
        }
        let url = self.http.url(
            Provider::Cloudflare,
            &["accounts", account, "r2", "buckets"],
            query,
        )?;
        let request = self
            .http
            .signed(Method::GET, Provider::Cloudflare, url, token)?
            .header("cf-r2-jurisdiction", jurisdiction);
        self.http.send_request(request, true).await
    }

    /// Whether one R2 bucket is there, in its jurisdiction.
    pub(super) async fn r2_bucket(
        &self,
        token: &Secret,
        account: &str,
        jurisdiction: &str,
        name: &str,
    ) -> Result<Value, EngineError> {
        if !matches!(
            jurisdiction,
            "default" | "eu" | "us" | "fedramp" | "fedramp-high"
        ) {
            return Err(EngineError::Invalid("Unsupported R2 jurisdiction."));
        }
        let url = self.http.url(
            Provider::Cloudflare,
            &["accounts", account, "r2", "buckets", name],
            &[],
        )?;
        let request = self
            .http
            .signed(Method::GET, Provider::Cloudflare, url, token)?
            .header("cf-r2-jurisdiction", jurisdiction);
        self.http.send_request(request, true).await
    }

    pub(super) async fn discover_inventory(
        &self,
        token: &Secret,
        account: &str,
        report: &mut DiscoveryReport,
    ) -> Result<(), EngineError> {
        for jurisdiction in ["default", "eu", "us", "fedramp", "fedramp-high"] {
            let family = format!("r2:{jurisdiction}");
            let found = match self.buckets(token, account, jurisdiction).await {
                // An account without that jurisdiction (FedRAMP is for some
                // plans only) is refused there: it has no buckets there.
                Err(error)
                    if jurisdiction != "default"
                        && matches!(error.code(), "provider.token_scope" | "provider.rejected") =>
                {
                    Ok(Vec::new())
                }
                found => found,
            };
            merge(report, &family, found)?;
        }
        merge(report, "zones", self.zones(token, account).await)
    }

    async fn buckets(
        &self,
        token: &Secret,
        account: &str,
        jurisdiction: &str,
    ) -> Result<Vec<RemoteResource>, EngineError> {
        let mut resources = Vec::new();
        let mut cursor = String::new();
        let mut cursors = HashSet::new();
        let mut names = HashSet::new();
        for _ in 0..100 {
            let mut query = vec![
                ("per_page", "1000"),
                ("order", "name"),
                ("direction", "asc"),
            ];
            if !cursor.is_empty() {
                query.push(("cursor", &cursor));
            }
            let value = self
                .r2_buckets(token, account, jurisdiction, &query)
                .await?;
            let rows = value
                .pointer("/result/buckets")
                .and_then(Value::as_array)
                .ok_or_else(invalid)?;
            if rows.len() > MAX_PAGE_ROWS || resources.len() + rows.len() > MAX_RESOURCES {
                return Err(limit());
            }
            for row in rows {
                let name = text(row, "name")?;
                if !(3..=64).contains(&name.len())
                    || !name
                        .bytes()
                        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
                    || !names.insert(name.to_owned())
                {
                    return Err(invalid());
                }
                if row
                    .get("jurisdiction")
                    .is_some_and(|value| value.as_str() != Some(jurisdiction))
                {
                    return Err(invalid());
                }
                let region = match row.get("location") {
                    None | Some(Value::Null) => None,
                    Some(Value::String(location))
                        if !location.is_empty()
                            && location.len() <= MAX_LOCATION_CHARS
                            && !location.chars().any(char::is_control) =>
                    {
                        Some(location.clone())
                    }
                    _ => return Err(invalid()),
                };
                resources.push(RemoteResource {
                    reference: format!("r2:{jurisdiction}:{name}"),
                    name: name.into(),
                    kind: ResourceKind::Bucket,
                    region,
                    status: ResourceStatus::Unknown,
                    url: None,
                    group: None,
                });
            }
            if let Some(count) = value.pointer("/result_info/total_count")
                && count.as_u64().is_none()
            {
                return Err(invalid());
            }
            let has_cursor = value
                .pointer("/result_info/cursor")
                .and_then(Value::as_str)
                .is_some_and(|cursor| !cursor.is_empty());
            if !has_cursor
                && value
                    .pointer("/result_info/total_count")
                    .and_then(Value::as_u64)
                    .is_some_and(|count| count > resources.len() as u64)
            {
                return Err(invalid());
            }
            match value.pointer("/result_info/cursor") {
                None | Some(Value::Null) => return Ok(resources),
                Some(Value::String(next)) if next.is_empty() => return Ok(resources),
                Some(Value::String(next))
                    if next.len() <= MAX_CURSOR_BYTES
                        && !next.chars().any(char::is_control)
                        && !rows.is_empty()
                        && cursors.insert(next.clone()) =>
                {
                    cursor.clone_from(next)
                }
                _ => return Err(invalid()),
            }
        }
        Err(limit())
    }

    async fn zones(
        &self,
        token: &Secret,
        account: &str,
    ) -> Result<Vec<RemoteResource>, EngineError> {
        let mut resources = Vec::new();
        let mut ids = HashSet::new();
        for page in 1..=200u64 {
            let number = page.to_string();
            let value = self
                .http
                .json(
                    Provider::Cloudflare,
                    token,
                    Method::GET,
                    &["zones"],
                    &[
                        ("account.id", account),
                        ("page", &number),
                        ("per_page", "50"),
                        ("type", "full,partial,secondary,internal"),
                    ],
                    None,
                )
                .await?;
            let rows = value
                .get("result")
                .and_then(Value::as_array)
                .ok_or_else(invalid)?;
            if rows.len() > ZONE_PAGE || resources.len() + rows.len() > MAX_RESOURCES {
                return Err(limit());
            }
            for row in rows {
                let id = text(row, "id")?;
                let name = text(row, "name")?;
                if id.is_empty()
                    || id.len() > MAX_ID_BYTES
                    || !id.bytes().all(|b| b.is_ascii_alphanumeric())
                    || !ids.insert(id.to_owned())
                    || row.pointer("/account/id").and_then(Value::as_str) != Some(account)
                    || !(crate::domain::domains::valid_hostname(name)
                        || (row.get("type").and_then(Value::as_str) == Some("internal")
                            && valid_slug(name)))
                {
                    return Err(invalid());
                }
                resources.push(RemoteResource {
                    reference: format!("zone:{id}"),
                    name: name.into(),
                    kind: ResourceKind::Domain,
                    region: None,
                    status: ResourceStatus::Unknown,
                    url: None,
                    group: None,
                });
            }
            if let Some(observed_page) = value.pointer("/result_info/page")
                && observed_page.as_u64() != Some(page)
            {
                return Err(invalid());
            }
            match value.pointer("/result_info/total_pages") {
                Some(total) => {
                    let total = total.as_u64().ok_or_else(invalid)?;
                    if total > MAX_ZONE_PAGES
                        || (total < page && !(page == 1 && total == 0 && rows.is_empty()))
                    {
                        return Err(invalid());
                    }
                    if page >= total {
                        return Ok(resources);
                    }
                    if rows.is_empty() {
                        return Err(invalid());
                    }
                }
                None if rows.len() < ZONE_PAGE => return Ok(resources),
                None => (),
            }
        }
        Err(limit())
    }
}

fn merge(
    report: &mut DiscoveryReport,
    family: &str,
    result: Result<Vec<RemoteResource>, EngineError>,
) -> Result<(), EngineError> {
    match result {
        Ok(resources) => {
            if report.resources.len() + resources.len() > MAX_RESOURCES {
                return Err(limit());
            }
            report.resources.extend(resources);
        }
        Err(error) if error.code() == "provider.token_expired" => return Err(error),
        Err(error) => report.warnings.push(DiscoveryWarning {
            resource_type: family.into(),
            code: error.code().into(),
            message: format!("Cloudflare inventory for {family} could not be completed. Check account access and retry; existing resources were retained."),
        }),
    }
    Ok(())
}
