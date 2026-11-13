//! Custom domains on managed resources. Creating a zone never changes the registrar's
//! nameservers.

use crate::engine::{Actor, Engine, now};
use crate::environments::EnvironmentService;
use crate::{
    domain::{domains::*, model::*},
    ports::catalog::EngineError,
};

const MAX_DOMAINS: usize = 10_000;
impl Engine {
    pub(crate) async fn detach_saved_domain(
        &self,
        resource: &Resource,
        domain: &CustomDomain,
    ) -> Result<(), EngineError> {
        let account = self
            .repository
            .account(&resource.account_id)
            .await?
            .ok_or(EngineError::NotFound)?;
        let token = self.accounts().account_token(&account.id).await?;
        self.adapter(resource.provider)?
            .detach_domain(
                &token,
                &account.external_id,
                &resource.provider_ref,
                domain.observed.provider_domain_id.as_deref(),
                &domain.hostname,
            )
            .await?;
        self.repository.remove_custom_domain(&domain.id).await
    }
}

impl<'a> EnvironmentService<'a> {
    /// The approval target for a hostname, tied to the resource it is assigned to.
    pub fn domain_approval_target(self, domain: &CustomDomain, resource: &Resource) -> String {
        domain_target(domain, resource)
    }

    /// Reads every saved domain, up to 10,000.
    ///
    /// # Errors
    /// Reports missing state or more than 10,000 domains.
    pub async fn all_domains(self) -> Result<Vec<CustomDomain>, EngineError> {
        let mut values = Vec::new();
        let mut cursor = None;
        loop {
            let page = self
                .engine
                .repository
                .custom_domains(None, cursor.as_deref())
                .await?;
            values.extend(page.items);
            if values.len() > MAX_DOMAINS {
                return Err(EngineError::Conflict(
                    "This machine exceeds 10,000 custom hostnames.",
                ));
            }
            cursor = page.next_cursor;
            if cursor.is_none() {
                return Ok(values);
            }
        }
    }

    /// Reserves a hostname locally, then sets up the zone and route at the provider.
    ///
    /// # Errors
    /// Rejects watched resources, invalid hostnames, conflicting assignments and provider failures.
    pub async fn add_domain(
        self,
        actor: &Actor,
        project_id: &str,
        resource_id: &str,
        hostname: &str,
        zone_name: &str,
        create_zone: bool,
    ) -> Result<CustomDomain, EngineError> {
        let _mutation = self.engine.mutation.lock().await;
        self.engine.authorize(actor, "apps:write").await?;
        self.engine.authorize(actor, "apps:create").await?;
        let hostname = hostname.trim().to_ascii_lowercase();
        let zone_name = zone_name.trim().to_ascii_lowercase();
        if !belongs_to_zone(&hostname, &zone_name) {
            return Err(EngineError::Invalid(
                "Enter an ASCII hostname and its containing DNS zone.",
            ));
        }
        let resource = self
            .engine
            .environments()
            .managed_domain_resource(project_id, resource_id)
            .await?;
        if create_zone
            && !self
                .engine
                .adapter(resource.provider)?
                .capabilities()
                .dns_zones
        {
            return Err(EngineError::Unsupported(format!(
                "{} doesn't create DNS zones. Add the records at your existing DNS provider and omit --create-zone.",
                resource.provider.display_name()
            )));
        }

        let previous = self
            .engine
            .environments()
            .all_domains()
            .await?
            .into_iter()
            .find(|value| value.hostname == hostname);
        if previous.as_ref().is_some_and(|value| {
            value.resource_id != resource_id || value.observed.zone_name != zone_name
        }) {
            return Err(EngineError::Conflict(
                "This hostname is already reserved for another resource or zone.",
            ));
        }
        let mut value = previous.unwrap_or_else(|| CustomDomain {
            id: uuid::Uuid::new_v4().to_string(),
            project_id: project_id.into(),
            resource_id: resource_id.into(),
            hostname,
            observed: DomainObservation {
                dns_records: vec![],
                zone_id: String::new(),
                zone_name,
                nameservers: vec![],
                registrar: None,
                dns: DomainReadiness::Unknown,
                https: DomainReadiness::Unknown,
                provider_domain_id: None,
                certificate_id: None,
                warnings: vec![],
            },
            checked_at: now(),
        });
        // Save it first, so a retry can find a route made before a connection failure.
        self.engine.repository.put_custom_domain(&value).await?;
        self.engine
            .environments()
            .configure_saved_domain(&resource, &mut value, create_zone, true)
            .await?;
        Ok(value)
    }

    /// Checks DNS and the certificate again, and attaches the route once the zone is active.
    ///
    /// # Errors
    /// Rejects watching resources or reports unavailable providers and state.
    pub async fn refresh_domain(self, id: &str) -> Result<CustomDomain, EngineError> {
        let _mutation = self.engine.mutation.lock().await;
        let mut value = self
            .engine
            .repository
            .custom_domain(id)
            .await?
            .ok_or(EngineError::NotFound)?;
        let resource = self
            .engine
            .environments()
            .managed_domain_resource(&value.project_id, &value.resource_id)
            .await?;
        self.engine
            .environments()
            .configure_saved_domain(&resource, &mut value, false, false)
            .await?;
        Ok(value)
    }

    /// Asks for approval to detach a hostname as it is assigned now, with its certificates.
    /// If the assignment changes, the approval no longer applies; changes at the provider are
    /// not tracked.
    pub async fn request_domain_deletion(
        self,
        actor: &Actor,
        id: &str,
    ) -> Result<Approval, EngineError> {
        self.engine.authorize(actor, "apps:write").await?;
        let domain = self
            .engine
            .repository
            .custom_domain(id)
            .await?
            .ok_or(EngineError::NotFound)?;
        let resource = self
            .engine
            .environments()
            .managed_domain_resource(&domain.project_id, &domain.resource_id)
            .await?;
        self.engine
            .request_approval(
                actor,
                GatedAction::DeleteDomain,
                domain_target(&domain, &resource),
                format!(
                    "Detach {} and all its provider certificates. DNS records and the project remain.",
                    domain.hostname
                ),
            )
            .await
    }

    /// Refreshes a managed domain for an agent with access; the check can start validation at
    /// the provider.
    pub async fn check_domain(self, actor: &Actor, id: &str) -> Result<CustomDomain, EngineError> {
        self.engine.authorize(actor, "apps:write").await?;
        self.engine.environments().refresh_domain(id).await
    }

    /// Detaches an approved hostname. The zone and the provider's project stay.
    ///
    /// # Errors
    /// Rejects watched resources, missing approvals, a reassigned hostname and provider failures.
    pub async fn delete_domain(
        self,
        actor: &Actor,
        id: &str,
        approval: Option<&str>,
    ) -> Result<(), EngineError> {
        let _mutation = self.engine.mutation.lock().await;
        self.engine.authorize(actor, "apps:write").await?;
        let domain = self
            .engine
            .repository
            .custom_domain(id)
            .await?
            .ok_or(EngineError::NotFound)?;
        let resource = self
            .engine
            .environments()
            .managed_domain_resource(&domain.project_id, &domain.resource_id)
            .await?;
        self.engine
            .gate(
                actor,
                GatedAction::DeleteDomain,
                &domain_target(&domain, &resource),
                approval,
            )
            .await?;
        self.engine.detach_saved_domain(&resource, &domain).await
    }

    async fn managed_domain_resource(
        self,
        project_id: &str,
        resource_id: &str,
    ) -> Result<Resource, EngineError> {
        let project = self
            .engine
            .repository
            .project(project_id)
            .await?
            .ok_or(EngineError::NotFound)?;
        let resource = self
            .engine
            .repository
            .resource(resource_id)
            .await?
            .ok_or(EngineError::NotFound)?;
        if resource.project_id.as_deref() != Some(project_id) {
            return Err(EngineError::Invalid(
                "The resource does not belong to this project.",
            ));
        }
        if project.mode != Mode::Managed || resource.mode != Mode::Managed {
            return Err(EngineError::Watching);
        }
        if !self
            .engine
            .adapter(resource.provider)?
            .capabilities()
            .domains
        {
            return Err(crate::ports::providers::unsupported());
        }
        Ok(resource)
    }

    async fn configure_saved_domain(
        self,
        resource: &Resource,
        domain: &mut CustomDomain,
        create_zone: bool,
        attach: bool,
    ) -> Result<(), EngineError> {
        let account = self
            .engine
            .repository
            .account(&resource.account_id)
            .await?
            .ok_or(EngineError::NotFound)?;
        let token = self.engine.accounts().account_token(&account.id).await?;
        let adapter = self.engine.adapter(resource.provider)?;
        let result = if attach {
            adapter
                .configure_domain(
                    &token,
                    &account.external_id,
                    &resource.provider_ref,
                    &domain.hostname,
                    &domain.observed.zone_name,
                    create_zone,
                )
                .await
        } else {
            adapter
                .observe_domain(
                    &token,
                    &account.external_id,
                    &resource.provider_ref,
                    &domain.hostname,
                    &domain.observed.zone_name,
                )
                .await
        };
        domain.checked_at = now();
        match result {
            Ok(observation) => domain.observed = observation,
            Err(error) => {
                domain.observed.dns = DomainReadiness::Unknown;
                domain.observed.https = DomainReadiness::Unknown;
                domain.observed.warnings = vec![format!("Domain check failed: {}.", error.code())];
                self.engine.repository.put_custom_domain(domain).await?;
                return Err(error);
            }
        }
        self.engine.repository.put_custom_domain(domain).await
    }
}

// Tie approvals to who owns the hostname, not to its readiness, which changes on each check.
fn domain_target(domain: &CustomDomain, resource: &Resource) -> String {
    use sha2::{Digest, Sha256};
    let mut hash = Sha256::new();
    for value in [
        &domain.id,
        &domain.project_id,
        &domain.resource_id,
        &domain.hostname,
        &domain.observed.zone_name,
        &resource.account_id,
        &resource.provider_ref,
    ] {
        hash.update((value.len() as u64).to_be_bytes());
        hash.update(value.as_bytes());
    }
    hash.update(resource.provider.slug().as_bytes());
    format!("domain:{:x}", hash.finalize())
}
