//! Pause and resume for an app's Fly Machines. The receipt kept in each
//! Machine's metadata holds only its startup settings (autostart, restart,
//! schedule) and a hash of its services, never its env or secrets.

use super::{Fly, field, invalid};
use crate::{Provider, application::ports::catalog::EngineError, domain::Secret};
use reqwest::Method;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

const MAX_MACHINES: usize = 100;
const MAX_ROWS: usize = 16;
const MAX_VALUE_BYTES: usize = 1024;
/// How often a machine is checked while it changes state.
const STATE_POLL: std::time::Duration = std::time::Duration::from_secs(1);

const RECEIPT: &str = "hoista_pause_v1";
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Receipt {
    services: String,
    autostart: Vec<Option<bool>>,
    restart: Option<Value>,
    schedule: Option<String>,
    started: bool,
    restored: bool,
}

impl Fly {
    pub(super) async fn control(
        &self,
        token: &Secret,
        app: &str,
        resume: bool,
    ) -> Result<(), EngineError> {
        let machines = self.machines(token, app).await?;
        if machines.is_empty() || machines.len() > MAX_MACHINES {
            return Err(EngineError::Conflict(
                "Pause/resume requires 1–100 Fly Machines.",
            ));
        }
        for listed in machines {
            let id = field(&listed, "id")?;
            let machine = self
                .http
                .json(
                    Provider::Fly,
                    token,
                    Method::GET,
                    &["apps", app, "machines", &id],
                    &[],
                    None,
                )
                .await?;
            let config = machine.get("config").ok_or_else(invalid)?;
            if config
                .pointer("/metadata/fly_process_group")
                .and_then(Value::as_str)
                .is_some_and(|v| v == "fly_app_release_command" || v == "fly_app_console")
            {
                continue;
            }
            if config.get("auto_destroy").and_then(Value::as_bool) == Some(true)
                || config
                    .get("standbys")
                    .and_then(Value::as_array)
                    .is_some_and(|v| !v.is_empty())
            {
                return Err(EngineError::Conflict(
                    "Automatic pause is unavailable for disposable or standby Machines.",
                ));
            }
            let state = field(&machine, "state")?;
            if !matches!(
                state.as_str(),
                "started" | "stopped" | "suspended" | "created"
            ) {
                return Err(EngineError::Conflict(
                    "Wait for the Fly Machine operation to finish before changing its state.",
                ));
            }
            let (next, start) = controlled(config.clone(), state == "started", resume)?;
            if &next != config {
                let version = field(&machine, "instance_id")?;
                self.http
                    .json(
                        Provider::Fly,
                        token,
                        Method::POST,
                        &["apps", app, "machines", &id],
                        &[],
                        Some(&json!({"config":next,"current_version":version,"skip_launch":true})),
                    )
                    .await?;
            }
            let fresh = self
                .http
                .json(
                    Provider::Fly,
                    token,
                    Method::GET,
                    &["apps", app, "machines", &id],
                    &[],
                    None,
                )
                .await?;
            let state = field(&fresh, "state")?;
            if resume && start && state != "started" {
                self.http
                    .json(
                        Provider::Fly,
                        token,
                        Method::POST,
                        &["apps", app, "machines", &id, "start"],
                        &[],
                        Some(&json!({})),
                    )
                    .await?;
            } else if !resume && state == "started" {
                self.http
                    .json(
                        Provider::Fly,
                        token,
                        Method::POST,
                        &["apps", app, "machines", &id, "stop"],
                        &[],
                        Some(&json!({"signal":"SIGTERM","timeout":"30s"})),
                    )
                    .await?;
            }
            let mut settled = false;
            for _ in 0..20 {
                let current = self
                    .http
                    .json(
                        Provider::Fly,
                        token,
                        Method::GET,
                        &["apps", app, "machines", &id],
                        &[],
                        None,
                    )
                    .await?;
                let state = field(&current, "state")?;
                if (resume && start && state == "started")
                    || ((!resume || !start)
                        && matches!(state.as_str(), "stopped" | "suspended" | "created"))
                {
                    settled = true;
                    break;
                }
                tokio::time::sleep(STATE_POLL).await;
            }
            if !settled {
                return Err(EngineError::Conflict(
                    "Fly has not reached the requested state. Recheck before retrying.",
                ));
            }
        }
        Ok(())
    }
}

fn service_hash(config: &Value) -> Result<String, EngineError> {
    let mut services = config.get("services").cloned().unwrap_or_else(|| json!([]));
    let rows = services.as_array_mut().ok_or_else(invalid)?;
    if rows.len() > MAX_ROWS {
        return Err(EngineError::Conflict(
            "Pause supports at most 16 services per Machine.",
        ));
    }
    for service in rows {
        service
            .as_object_mut()
            .ok_or_else(invalid)?
            .remove("autostart");
    }
    Ok(format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&services).map_err(|_| invalid())?)
    ))
}

fn controlled(
    mut config: Value,
    started: bool,
    resume: bool,
) -> Result<(Value, bool), EngineError> {
    let prior = config
        .pointer(&format!("/metadata/{RECEIPT}"))
        .and_then(Value::as_str);
    let mut saved: Option<Receipt> = prior
        .map(|value| {
            if value.len() > MAX_VALUE_BYTES {
                return Err(invalid());
            }
            serde_json::from_str(value).map_err(|_| invalid())
        })
        .transpose()?;
    if saved.as_ref().is_some_and(|receipt| !receipt.restored) {
        let services = config
            .get("services")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        if services
            .iter()
            .any(|service| service.get("autostart") != Some(&Value::Bool(false)))
            || config.pointer("/restart/policy").and_then(Value::as_str) != Some("no")
            || config
                .get("schedule")
                .is_some_and(|value| value.as_str() != Some(""))
        {
            return Err(EngineError::Conflict(
                "Fly startup controls changed after pausing. Review them before restoring.",
            ));
        }
    }
    let hash = service_hash(&config)?;
    if resume {
        let receipt = saved.as_ref().ok_or(EngineError::Conflict(
            "This Machine has no Hoista pause receipt. Inspect its settings in Fly.",
        ))?;
        if receipt.services != hash {
            return Err(EngineError::Conflict(
                "Fly service configuration changed after pausing. Review it before restoring startup settings.",
            ));
        }
        if receipt.restored {
            return Ok((config, receipt.started));
        }
    } else if saved.as_ref().is_none_or(|receipt| receipt.restored) {
        let services = config
            .get("services")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let restart = config.get("restart").cloned();
        if let Some(restart) = &restart {
            let object = restart.as_object().ok_or_else(invalid)?;
            if object
                .keys()
                .any(|key| !matches!(key.as_str(), "policy" | "max_retries"))
            {
                return Err(invalid());
            }
        }
        saved = Some(Receipt {
            services: hash.clone(),
            autostart: services
                .iter()
                .map(|service| service.get("autostart").and_then(Value::as_bool))
                .collect(),
            restart,
            schedule: config
                .get("schedule")
                .and_then(Value::as_str)
                .map(str::to_owned),
            started,
            restored: false,
        });
    }
    let mut receipt = saved.ok_or_else(invalid)?;
    if receipt.services != hash {
        return Err(EngineError::Conflict(
            "Fly service configuration changed while paused.",
        ));
    }
    if let Some(services) = config.get_mut("services") {
        let services = services.as_array_mut().ok_or_else(invalid)?;
        if services.len() != receipt.autostart.len() {
            return Err(invalid());
        }
        for (index, service) in services.iter_mut().enumerate() {
            let service = service.as_object_mut().ok_or_else(invalid)?;
            if resume {
                match receipt.autostart[index] {
                    Some(value) => {
                        service.insert("autostart".into(), value.into());
                    }
                    None => {
                        service.remove("autostart");
                    }
                }
            } else {
                service.insert("autostart".into(), false.into());
            }
        }
    }
    let object = config.as_object_mut().ok_or_else(invalid)?;
    if resume {
        match &receipt.restart {
            Some(value) => {
                object.insert("restart".into(), value.clone());
            }
            None => {
                object.remove("restart");
            }
        }
        match &receipt.schedule {
            Some(value) => {
                object.insert("schedule".into(), value.clone().into());
            }
            None => {
                object.remove("schedule");
            }
        }
    } else {
        object.insert("restart".into(), json!({"policy":"no"}));
        object.remove("schedule");
    }
    receipt.restored = resume;
    let serialized = serde_json::to_string(&receipt).map_err(|_| invalid())?;
    if serialized.len() > MAX_VALUE_BYTES {
        return Err(EngineError::Conflict(
            "The Machine pause receipt exceeds the metadata limit.",
        ));
    }
    object
        .entry("metadata")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .ok_or_else(invalid)?
        .insert(RECEIPT.into(), serialized.into());
    Ok((config, receipt.started))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pause_receipts_restore_original_controls_without_copying_secrets() {
        let original = json!({
            "image": "app@sha256:abc",
            "env": {"SECRET": "never-in-receipt"},
            "services": [{"internal_port": 8080, "autostart": true}, {"internal_port": 9000}],
            "restart": {"policy": "always", "max_retries": 2},
            "schedule": "daily",
            "metadata": {"owner": "user"},
        });
        let (paused, _) = controlled(original.clone(), true, false).expect("pause");
        assert_eq!(paused["services"][0]["autostart"], false);
        assert_eq!(paused["restart"]["policy"], "no");
        assert!(paused.get("schedule").is_none());
        assert!(
            !paused["metadata"][RECEIPT]
                .as_str()
                .expect("receipt")
                .contains("never-in-receipt")
        );
        assert_eq!(
            controlled(paused.clone(), false, false).expect("retry").0,
            paused
        );
        let (mut restored, start) = controlled(paused.clone(), false, true).expect("resume");
        assert!(start);
        assert_eq!(
            controlled(restored.clone(), true, true)
                .expect("retry resume")
                .0,
            restored
        );
        restored["metadata"]
            .as_object_mut()
            .expect("metadata")
            .remove(RECEIPT);
        assert_eq!(restored, original);
        let mut changed = paused;
        changed["services"][0]["internal_port"] = 8081.into();
        assert!(controlled(changed, false, true).is_err());
        assert!(controlled(original, false, true).is_err());
    }
}
