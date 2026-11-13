//! Preparing deploy secrets. Only metadata is kept, never a generated value.

use crate::{
    domain::{Secret, model::*},
    ports::catalog::EngineError,
};
use std::collections::BTreeSet;

const MAX_NAMES: usize = 256;
const MAX_NAME_BYTES: usize = 128;
/// Random bytes in a generated secret, written out in hex.
const GENERATED_SECRET_BYTES: usize = 32;

/// A new random secret, such as a session-signing key, in hex.
fn generated_secret() -> Result<Secret, EngineError> {
    let mut bytes = zeroize::Zeroizing::new([0u8; GENERATED_SECRET_BYTES]);
    ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), bytes.as_mut())
        .map_err(|_| EngineError::Store)?;
    Ok(Secret::new(
        bytes.iter().map(|byte| format!("{byte:02x}")).collect(),
    ))
}

/// A new resource can get a freshly generated key; an existing one keeps the value at its
/// provider. An existing resource with no record of the secret needs a value given, because a
/// generated one could sign everyone out.
pub(crate) fn resolve(
    plan: &Plan,
    project_id: &str,
    resources: &[Resource],
    metadata: &[SecretMetadata],
    values: &mut Vec<(String, Secret)>,
) -> Result<(), EngineError> {
    if plan
        .generated_secret_names
        .iter()
        .any(|name| name != "BETTER_AUTH_SECRET" || !plan.env_names.contains(name))
    {
        return Err(EngineError::Invalid(
            "Unsupported generated authentication secret.",
        ));
    }
    if plan.env_names.len() > MAX_NAMES
        || plan.env_names.iter().any(|name| {
            name.is_empty()
                || name.len() > MAX_NAME_BYTES
                || !name.bytes().enumerate().all(|(index, byte)| {
                    byte == b'_'
                        || byte.is_ascii_alphabetic()
                        || (index > 0 && byte.is_ascii_digit())
                })
        })
    {
        return Err(EngineError::Invalid(
            "Use at most 256 valid environment names of up to 128 characters.",
        ));
    }
    let mut missing = BTreeSet::new();
    let runtime: Vec<_> = plan
        .resources
        .iter()
        .filter(|resource| resource.kind != ResourceKind::Database)
        .collect();
    for name in &plan.env_names {
        if values.iter().any(|(key, _)| key == name) {
            continue;
        }
        if matches!(name.as_str(), "DATABASE_URL" | "DIRECT_URL")
            && plan
                .resources
                .iter()
                .any(|resource| resource.kind == ResourceKind::Database)
        {
            continue;
        }
        let known = |planned: &&ResourcePlan| {
            resources
                .iter()
                .find(|resource| {
                    resource.project_id.as_deref() == Some(project_id)
                        && resource.account_id == planned.account_id
                        && resource.name == planned.name
                })
                .is_some_and(|resource| {
                    metadata
                        .iter()
                        .any(|value| value.resource_id == resource.id && value.name == *name)
                })
        };
        if !runtime.is_empty() && runtime.iter().all(known) {
            continue;
        }
        if plan.generated_secret_names.contains(name)
            && !runtime.is_empty()
            && runtime
                .iter()
                .all(|resource| resource.provider_ref.is_none())
        {
            let value = generated_secret()?;
            values.push((name.clone(), value));
            continue;
        }
        missing.insert(name.clone());
    }
    if !missing.is_empty() {
        return Err(EngineError::MissingSecrets(missing.into_iter().collect()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::Provider;
    use crate::environments::secrets::*;
    fn plan() -> Plan {
        Plan {
            revision: 0,
            id: "plan".into(),
            source: Source::Folder {
                path: "/fixture".into(),
            },
            framework: "node".into(),
            package_manager: Some("npm".into()),
            build_command: vec![],
            output_directory: ".".into(),
            env_names: vec!["BETTER_AUTH_SECRET".into()],
            generated_secret_names: vec!["BETTER_AUTH_SECRET".into()],
            prisma_migration: None,
            warnings: vec![],
            resources: vec![ResourcePlan {
                schedules: None,
                name: "web".into(),
                kind: ResourceKind::Web,
                provider: Provider::Cloudflare,
                account_id: "account".into(),
                region: None,
                depends_on: vec![],
                provider_ref: Some("worker:web".into()),
            }],
        }
    }

    #[test]
    fn missing_inputs_are_complete_unique_sorted_and_values_never_escape() {
        let mut plan = plan();
        plan.env_names = vec![
            "Z_KEY".into(),
            "A_KEY".into(),
            "Z_KEY".into(),
            "SUPPLIED".into(),
        ];
        plan.generated_secret_names.clear();
        let mut values = vec![(
            "SUPPLIED".into(),
            Secret::new("never-output-this-value".into()),
        )];
        let error = resolve(&plan, "project", &[], &[], &mut values).expect_err("missing inputs");
        assert_eq!(error.code(), "deploy.secret_missing");
        assert_eq!(error.missing_secrets(), ["A_KEY", "Z_KEY"]);
        assert!(!format!("{error:?}").contains("never-output-this-value"));
        plan.env_names = vec!["BAD\nNAME".into()];
        let error = resolve(&plan, "project", &[], &[], &mut values).expect_err("invalid name");
        assert!(error.missing_secrets().is_empty());
        assert!(!error.to_string().contains("BAD"));
    }

    #[test]
    fn existing_unknown_keys_and_other_resources_metadata_cannot_trigger_generation() {
        let plan = plan();
        let resource = Resource {
            id: "web".into(),
            project_id: Some("project".into()),
            kind: ResourceKind::Web,
            provider: Provider::Cloudflare,
            account_id: "account".into(),
            provider_ref: "worker:web".into(),
            name: "web".into(),
            region: None,
            status: ResourceStatus::Live,
            mode: Mode::Managed,
            url: None,
            group: None,
        };
        let mut metadata = SecretMetadata {
            requires_source_sync: false,
            set_by: None,
            managed_by: None,
            project_id: "project".into(),
            resource_id: "other".into(),
            name: "BETTER_AUTH_SECRET".into(),
            fingerprint: "fingerprint".into(),
            set_at: "time".into(),
        };
        let mut values = vec![];
        assert!(
            resolve(
                &plan,
                "project",
                std::slice::from_ref(&resource),
                std::slice::from_ref(&metadata),
                &mut values
            )
            .is_err()
        );
        assert!(values.is_empty());
        metadata.resource_id = resource.id.clone();
        resolve(&plan, "project", &[resource], &[metadata], &mut values)
            .expect("preserve known key");
        assert!(values.is_empty());
        values.push(("BETTER_AUTH_SECRET".into(), Secret::new("explicit".into())));
        resolve(&plan, "project", &[], &[], &mut values).expect("supplied value");
        assert_eq!(values[0].1.expose(), "explicit");
    }
}
