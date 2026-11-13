//! A Neon project's branches, computes and operations, read with read-only
//! API requests and no connection credentials.

use super::{Neon, reject_incomplete};
use crate::answers::{field, invalid};
use crate::{
    application::ports::catalog::EngineError,
    domain::{Secret, database_observation::*},
};
use serde_json::Value;
use std::collections::BTreeSet;

const PAGE_SIZE: usize = 100;
const MAX_ROWS: usize = 10_000;
const MAX_TEXT_BYTES: usize = 256;
const MAX_CURSOR_BYTES: usize = 4096;

fn text(value: &Value, key: &str) -> Result<String, EngineError> {
    let text = field(value, key)?;
    if text.len() > MAX_TEXT_BYTES || text.chars().any(char::is_control) {
        return Err(invalid());
    }
    Ok(text)
}

fn optional_text(value: &Value, key: &str) -> Result<Option<String>, EngineError> {
    if value.get(key).is_none_or(Value::is_null) {
        Ok(None)
    } else {
        text(value, key).map(Some)
    }
}

fn optional_integer(value: &Value, key: &str, min: i64) -> Result<Option<i64>, EngineError> {
    value
        .get(key)
        .filter(|v| !v.is_null())
        .map(|v| v.as_i64().filter(|n| *n >= min).ok_or_else(invalid))
        .transpose()
}

fn units(value: &Value, key: &str) -> Result<Option<f64>, EngineError> {
    value
        .get(key)
        .filter(|v| !v.is_null())
        .map(|v| {
            v.as_f64()
                .filter(|n| n.is_finite() && *n > 0.0)
                .ok_or_else(invalid)
        })
        .transpose()
}

pub(super) async fn read(
    adapter: &Neon,
    token: &Secret,
    org: &str,
    project: &str,
) -> Result<DatabaseObservation, EngineError> {
    let owner = adapter.get(token, &["projects", project], &[]).await?;
    if owner["project"]["id"].as_str() != Some(project)
        || (!org.is_empty() && owner["project"]["org_id"].as_str() != Some(org))
    {
        return Err(EngineError::Invalid(
            "The Neon project does not match this account.",
        ));
    }
    let mut branches = vec![];
    let mut branch_ids = BTreeSet::new();
    let mut cursors = BTreeSet::new();
    let mut cursor = String::new();
    let mut complete = false;
    for _ in 0..100 {
        let mut query = vec![("limit", "100")];
        if !cursor.is_empty() {
            query.push(("cursor", cursor.as_str()));
        }
        let response = adapter
            .get(token, &["projects", project, "branches"], &query)
            .await?;
        reject_incomplete(&response)?;
        if response
            .get("pagination")
            .is_some_and(|value| !value.is_null() && !value.is_object())
        {
            return Err(invalid());
        }
        let rows = response["branches"].as_array().ok_or_else(invalid)?;
        if rows.len() > PAGE_SIZE {
            return Err(invalid());
        }
        for row in rows {
            let id = text(row, "id")?;
            if row["project_id"].as_str() != Some(project) || !branch_ids.insert(id.clone()) {
                return Err(invalid());
            }
            branches.push(DatabaseBranch {
                id,
                name: text(row, "name")?,
                parent_id: optional_text(row, "parent_id")?,
                state: text(row, "current_state")?,
                is_default: row
                    .get("default")
                    .filter(|v| !v.is_null())
                    .map(|v| v.as_bool().ok_or_else(invalid))
                    .transpose()?,
                logical_size_bytes: optional_integer(row, "logical_size", 0)?,
            });
        }
        match response.pointer("/pagination/next") {
            None | Some(Value::Null) => {
                complete = true;
                break;
            }
            Some(Value::String(next)) if next.is_empty() => {
                complete = true;
                break;
            }
            Some(Value::String(next))
                if next.len() <= MAX_CURSOR_BYTES
                    && !rows.is_empty()
                    && cursors.insert(next.clone()) =>
            {
                cursor = next.clone()
            }
            _ => return Err(invalid()),
        }
    }
    if !complete {
        return Err(EngineError::Conflict(
            "The project exceeds the 10,000-branch observation limit.",
        ));
    }
    let response = adapter
        .get(token, &["projects", project, "endpoints"], &[])
        .await?;
    reject_incomplete(&response)?;
    if response
        .get("pagination")
        .is_some_and(|value| !value.is_null())
    {
        return Err(invalid());
    }
    let rows = response["endpoints"].as_array().ok_or_else(invalid)?;
    if rows.len() > MAX_ROWS {
        return Err(EngineError::Conflict(
            "The project exceeds the 10,000-compute observation limit.",
        ));
    }
    let mut computes = vec![];
    let mut ids = BTreeSet::new();
    for row in rows {
        let id = text(row, "id")?;
        let branch_id = text(row, "branch_id")?;
        if row["project_id"].as_str() != Some(project)
            || !branch_ids.contains(&branch_id)
            || !ids.insert(id.clone())
        {
            return Err(invalid());
        }
        let min_compute_units = units(row, "autoscaling_limit_min_cu")?;
        let max_compute_units = units(row, "autoscaling_limit_max_cu")?;
        if min_compute_units
            .zip(max_compute_units)
            .is_some_and(|(min, max)| min > max)
        {
            return Err(invalid());
        }
        computes.push(DatabaseCompute {
            id,
            branch_id,
            kind: text(row, "type")?,
            state: text(row, "current_state")?,
            pending_state: optional_text(row, "pending_state")?,
            region: text(row, "region_id")?,
            min_compute_units,
            max_compute_units,
            suspend_timeout_seconds: optional_integer(row, "suspend_timeout_seconds", -1)?,
        });
    }
    branches.sort_by(|a, b| a.id.cmp(&b.id));
    computes.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(DatabaseObservation {
        observed_at: chrono::Utc::now().to_rfc3339(),
        branches,
        computes,
        wake_count: None,
    })
}

pub(super) async fn operations(
    adapter: &Neon,
    token: &Secret,
    org: &str,
    project: &str,
    cursor: Option<&str>,
) -> Result<DatabaseOperationHistory, EngineError> {
    if cursor.is_some_and(|v| {
        v.is_empty() || v.len() > MAX_CURSOR_BYTES || v.chars().any(char::is_control)
    }) {
        return Err(EngineError::Invalid(
            "Use the cursor returned by the preceding operation page.",
        ));
    }
    let owner = adapter.get(token, &["projects", project], &[]).await?;
    if owner["project"]["id"].as_str() != Some(project)
        || (!org.is_empty() && owner["project"]["org_id"].as_str() != Some(org))
    {
        return Err(EngineError::Invalid(
            "The Neon project does not match this account.",
        ));
    }
    let mut query = vec![("limit", "100")];
    if let Some(cursor) = cursor {
        query.push(("cursor", cursor));
    }
    let response = adapter
        .get(token, &["projects", project, "operations"], &query)
        .await?;
    // Not `reject_incomplete`: this list pages, and its cursor is read below.
    if response
        .get("unavailable")
        .is_some_and(|v| !v.is_null() && v.as_array().is_none_or(|a| !a.is_empty()))
    {
        return Err(invalid());
    }
    let rows = response["operations"].as_array().ok_or_else(invalid)?;
    if rows.len() > PAGE_SIZE {
        return Err(invalid());
    }
    let mut ids = BTreeSet::new();
    let mut items = Vec::new();
    for row in rows {
        let id = text(row, "id")?;
        if row["project_id"].as_str() != Some(project) || !ids.insert(id.clone()) {
            return Err(invalid());
        }
        let created_at = text(row, "created_at")?;
        let updated_at = text(row, "updated_at")?;
        let created = chrono::DateTime::parse_from_rfc3339(&created_at).map_err(|_| invalid())?;
        let updated = chrono::DateTime::parse_from_rfc3339(&updated_at).map_err(|_| invalid())?;
        if updated < created {
            return Err(invalid());
        }
        items.push(DatabaseOperation {
            id,
            branch_id: optional_text(row, "branch_id")?,
            compute_id: optional_text(row, "endpoint_id")?,
            action: text(row, "action")?,
            status: text(row, "status")?,
            created_at,
            updated_at,
        });
    }
    let next_cursor = match response.get("pagination") {
        None | Some(Value::Null) => None,
        Some(Value::Object(page)) => {
            let next = page.get("next").filter(|v| !v.is_null());
            let older = page.get("cursor").filter(|v| !v.is_null());
            if next.zip(older).is_some_and(|(a, b)| a != b) {
                return Err(invalid());
            }
            match next.or(older) {
                None => None,
                Some(Value::String(v)) if v.is_empty() => None,
                Some(Value::String(v))
                    if v.len() <= MAX_CURSOR_BYTES
                        && !v.chars().any(char::is_control)
                        && Some(v.as_str()) != cursor
                        && !rows.is_empty() =>
                {
                    Some(v.clone())
                }
                _ => return Err(invalid()),
            }
        }
        _ => return Err(invalid()),
    };
    let completed_starts_in_page = items
        .iter()
        .filter(|op| op.action == "start_compute" && op.status == "finished")
        .count();
    Ok(DatabaseOperationHistory {
        observed_at: chrono::Utc::now().to_rfc3339(),
        items,
        next_cursor,
        completed_starts_in_page,
        complete_history: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::ProviderHttp;
    use axum::{
        Json, Router,
        extract::State,
        http::{Method, Uri},
        routing::get,
    };
    use serde_json::json;
    use std::{collections::VecDeque, sync::Arc};
    type Responses = Arc<tokio::sync::Mutex<VecDeque<(&'static str, Value)>>>;
    fn branch(id: &str) -> Value {
        json!({
            "id": id,
            "project_id": "project",
            "name": id,
            "current_state": "ready",
            "default": id=="main",
            "logical_size": 123,
            "password": "must-not-appear",
        })
    }

    fn endpoint() -> Value {
        json!({
            "id": "ep-one",
            "project_id": "project",
            "branch_id": "main",
            "type": "read_write",
            "current_state": "idle",
            "pending_state": "active",
            "region_id": "aws-us-east-1",
            "autoscaling_limit_min_cu": 0.25,
            "autoscaling_limit_max_cu": 2,
            "suspend_timeout_seconds": -1,
            "connection_uri": "must-not-appear",
        })
    }

    fn owner() -> Value {
        json!({"project":{"id":"project","org_id":"org"}})
    }

    async fn fixture(
        responses: Vec<(&'static str, Value)>,
    ) -> Result<DatabaseObservation, EngineError> {
        let responses = Arc::new(tokio::sync::Mutex::new(VecDeque::from(responses)));
        let router = Router::new()
            .fallback(get(
                |State(rows): State<Responses>, uri: Uri, method: Method| async move {
                    assert_eq!(method, Method::GET);
                    let (path, value) = rows
                        .lock()
                        .await
                        .pop_front()
                        .expect("unexpected provider request");
                    assert_eq!(uri.path_and_query().unwrap().as_str(), path);
                    Json(value)
                },
            ))
            .with_state(responses.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
        let server = tokio::spawn(async move {
            axum::serve(listener, router)
                .with_graceful_shutdown(async { stopped.await.unwrap() })
                .await
                .unwrap()
        });
        let adapter = Neon::new(ProviderHttp::local_fixture(
            url::Url::parse(&format!("http://{address}/")).unwrap(),
        ));
        let result = read(&adapter, &Secret::new("fixture".into()), "org", "project").await;
        stop.send(()).unwrap();
        server.await.unwrap();
        assert!(responses.lock().await.is_empty());
        result
    }

    #[tokio::test]
    async fn observation_follows_branch_pages_and_returns_only_safe_measured_fields() {
        let mut child = branch("child");
        child["parent_id"] = json!("main");
        child.as_object_mut().unwrap().remove("logical_size");
        let result = fixture(vec![
            ("/projects/project", owner()),
            (
                "/projects/project/branches?limit=100",
                json!({"branches":[branch("main")],"pagination":{"next":"next"}}),
            ),
            (
                "/projects/project/branches?limit=100&cursor=next",
                json!({"branches":[child]}),
            ),
            (
                "/projects/project/endpoints",
                json!({"endpoints":[endpoint()]}),
            ),
        ])
        .await
        .unwrap();
        assert_eq!(result.branches.len(), 2);
        assert_eq!(result.branches[0].parent_id.as_deref(), Some("main"));
        assert!(result.branches[0].logical_size_bytes.is_none());
        assert_eq!(result.branches[1].logical_size_bytes, Some(123));
        assert_eq!(result.computes[0].state, "idle");
        assert_eq!(result.computes[0].pending_state.as_deref(), Some("active"));
        assert_eq!(result.computes[0].min_compute_units, Some(0.25));
        assert_eq!(result.computes[0].suspend_timeout_seconds, Some(-1));
        assert!(result.wake_count.is_none());
        assert!(
            !serde_json::to_string(&result)
                .unwrap()
                .contains("must-not-appear")
        );
    }

    #[tokio::test]
    async fn observation_rejects_foreign_partial_duplicate_and_invalid_provider_data() {
        assert!(
            fixture(vec![
                ("/projects/project", owner()),
                (
                    "/projects/project/branches?limit=100",
                    json!({"branches":[branch("main")],"pagination":"unrecognized"})
                )
            ])
            .await
            .is_err()
        );
        assert!(
            fixture(vec![(
                "/projects/project",
                json!({"project":{"id":"project","org_id":"foreign"}})
            )])
            .await
            .is_err()
        );
        for (key, value) in [
            ("project_id", json!("foreign")),
            ("logical_size", json!(-1)),
            ("default", json!("false")),
        ] {
            let mut row = branch("main");
            row[key] = value;
            assert!(
                fixture(vec![
                    ("/projects/project", owner()),
                    (
                        "/projects/project/branches?limit=100",
                        json!({"branches":[row]})
                    )
                ])
                .await
                .is_err()
            );
        }
        assert!(
            fixture(vec![
                ("/projects/project", owner()),
                (
                    "/projects/project/branches?limit=100",
                    json!({"branches":[branch("main"),branch("main")]})
                )
            ])
            .await
            .is_err()
        );
        for (key, value) in [
            ("project_id", json!("foreign")),
            ("branch_id", json!("missing")),
            ("autoscaling_limit_min_cu", json!(3)),
            ("suspend_timeout_seconds", json!(-2)),
        ] {
            let mut row = endpoint();
            row[key] = value;
            assert!(
                fixture(vec![
                    ("/projects/project", owner()),
                    (
                        "/projects/project/branches?limit=100",
                        json!({"branches":[branch("main")]})
                    ),
                    ("/projects/project/endpoints", json!({"endpoints":[row]}))
                ])
                .await
                .is_err()
            );
        }
        for response in [
            json!({"endpoints":[endpoint(),endpoint()]}),
            json!({"endpoints":[],"unavailable":["ep-one"]}),
            json!({"endpoints":[],"pagination":{"next":"later"}}),
        ] {
            assert!(
                fixture(vec![
                    ("/projects/project", owner()),
                    (
                        "/projects/project/branches?limit=100",
                        json!({"branches":[branch("main")]})
                    ),
                    ("/projects/project/endpoints", response)
                ])
                .await
                .is_err()
            );
        }
        assert!(
            fixture(vec![
                ("/projects/project", owner()),
                (
                    "/projects/project/branches?limit=100",
                    json!({"branches":[branch("main")],"pagination":{"next":"same"}})
                ),
                (
                    "/projects/project/branches?limit=100&cursor=same",
                    json!({"branches":[branch("child")],"pagination":{"next":"same"}})
                )
            ])
            .await
            .is_err()
        );
    }
}
