//! Reading and changing saved plans. Nothing changes at a provider.

use anyhow::{Context, bail};
use hoista_server::{client::LocalClient, domain::model::Plan};
use std::io::Read;
use zeroize::Zeroizing;

const MAX_VALUE_BYTES: usize = 128;

const LIMIT: usize = 128 * 1024;

pub(crate) fn valid_id(id: &str) -> anyhow::Result<()> {
    crate::context::check_id(id.strip_prefix("queued:").unwrap_or(id), "a plan")
}

pub(crate) async fn show(client: &LocalClient, id: &str) -> anyhow::Result<Plan> {
    valid_id(id)?;
    serde_json::from_value(client.get_json(&format!("/plans/{id}")).await?)
        .context("Invalid plan response")
}

pub(crate) fn read(input: impl Read, id: &str) -> anyhow::Result<Plan> {
    valid_id(id)?;
    let mut bytes = Zeroizing::new(Vec::new());
    input
        .take((LIMIT + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| anyhow::anyhow!("Could not read plan JSON."))?;
    if bytes.len() > LIMIT {
        bail!("Plan JSON exceeds 128 KiB.");
    }
    let plan: Plan = serde_json::from_slice(&bytes).map_err(|_| {
        anyhow::anyhow!("Give the whole plan as hoist plan show --json prints it. Unknown fields and secret values aren't accepted.")
    })?;
    if plan.id != id {
        bail!("The plan id in the JSON has to match the one named.");
    }
    Ok(plan)
}

pub(crate) async fn save(client: &LocalClient, plan: Plan) -> anyhow::Result<serde_json::Value> {
    valid_id(&plan.id)?;
    if plan.id.starts_with("queued:") {
        bail!("A plan that already started a deploy can't change. Make a new one with hoist plan.");
    }
    let value = serde_json::to_value(&plan)?;
    if serde_json::to_vec(&value)?.len() > LIMIT {
        bail!("Plan JSON exceeds 128 KiB.");
    }
    Ok(client
        .put_json(&format!("/plans/{}", plan.id), &value)
        .await?)
}

pub(crate) async fn change(
    client: &LocalClient,
    id: &str,
    resource_name: &str,
    field: &str,
    value: &str,
) -> anyhow::Result<serde_json::Value> {
    let mut plan = show(client, id).await?;
    if plan
        .resources
        .iter()
        .filter(|resource| resource.name == resource_name)
        .count()
        != 1
    {
        bail!("Name one resource exactly as hoist plan show lists it.");
    }
    let resource = plan
        .resources
        .iter_mut()
        .find(|resource| resource.name == resource_name)
        .context("Plan resource is unavailable")?;
    match field {
        "account" => {
            crate::context::check_id(value, "an account")?;
            resource.account_id = value.into();
        }
        "region" => {
            if value != "-"
                && (value.is_empty()
                    || value.len() > MAX_VALUE_BYTES
                    || !value
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_'))
            {
                bail!("Use the provider's region id, or - to clear it.");
            }
            resource.region = (value != "-").then(|| value.into());
        }
        _ => bail!("Unsupported plan change."),
    }
    save(client, plan).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn malformed_or_oversized_plan_input_never_echoes_submitted_values() {
        for input in [
            b"private-input-not-json".to_vec(),
            br#"{"secret":"private-input"}"#.to_vec(),
            vec![b'x'; LIMIT + 1],
        ] {
            let error = read(input.as_slice(), "plan-id").expect_err("invalid plan");
            assert!(!error.to_string().contains("private-input"));
        }
        assert!(valid_id("queued:abc-123").is_ok());
        assert!(valid_id("abc?query=1").is_err());
        assert!(valid_id("../plans").is_err());
    }
}
