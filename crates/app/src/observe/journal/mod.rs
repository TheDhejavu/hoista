//! Reading the operation journal, and the person's answer for a change
//! whose outcome Hoista doesn't know.

use crate::engine::{Actor, now};
use crate::observe::ObserveService;
use crate::{
    domain::model::{Operation, OperationState},
    ports::catalog::{EngineError, Page},
};
use std::ops::RangeInclusive;

/// How long the person's evidence for an answer can be, in bytes.
const EVIDENCE_CHARS: RangeInclusive<usize> = 10..=1000;

impl<'a> ObserveService<'a> {
    /// Changes Hoista asked providers to make, newest first. One still marked
    /// running that this engine isn't sending was cut off by a stop: it's
    /// shown as unclear.
    ///
    /// # Errors
    /// Returns unavailable persistent state or a bad cursor.
    pub async fn operations(self, cursor: Option<&str>) -> Result<Page<Operation>, EngineError> {
        let mut page = self.engine.repository.operations(cursor).await?;
        for operation in &mut page.items {
            if operation.state == OperationState::Running
                && !self.engine.observe().sending(&operation.key)
            {
                operation.state = OperationState::Unclear;
            }
            operation.key.clear();
        }
        Ok(page)
    }

    /// The person checked the provider: an unclear change happened, or didn't.
    /// Either way an identical change can be made again.
    ///
    /// # Errors
    /// Refuses agents, unknown ids and changes whose outcome is already known.
    pub async fn check_operation(
        self,
        actor: &Actor,
        id: &str,
        happened: bool,
    ) -> Result<Operation, EngineError> {
        if !matches!(actor, Actor::Person) {
            return Err(EngineError::Conflict(
                "Only the person can say whether a change happened at the provider.",
            ));
        }
        self.engine
            .observe()
            .settle_operation(actor, id, happened, None)
            .await
    }

    /// An agent read the provider and says whether an unclear change
    /// happened, with what it read as evidence. The evidence is kept with the
    /// change and in Activity, where the person sees who said what.
    ///
    /// # Errors
    /// Refuses agents without change access, missing or overlong evidence,
    /// unknown ids and changes whose outcome is already known.
    pub async fn resolve_operation(
        self,
        actor: &Actor,
        id: &str,
        happened: bool,
        evidence: &str,
    ) -> Result<Operation, EngineError> {
        self.engine.authorize(actor, "apps:write").await?;
        let evidence = evidence.trim();
        if !EVIDENCE_CHARS.contains(&evidence.len()) {
            return Err(EngineError::Invalid(
                "Give evidence: what you read at the provider that shows it, such as the list call and what it returned, in up to 1,000 characters.",
            ));
        }
        self.engine
            .observe()
            .settle_operation(actor, id, happened, Some(evidence))
            .await
    }

    async fn settle_operation(
        self,
        actor: &Actor,
        id: &str,
        happened: bool,
        evidence: Option<&str>,
    ) -> Result<Operation, EngineError> {
        let mut operation = self
            .engine
            .repository
            .operation(id)
            .await?
            .ok_or(EngineError::NotFound)?;
        let unclear = operation.state == OperationState::Unclear
            || (operation.state == OperationState::Running
                && !self.engine.observe().sending(&operation.key));
        if !unclear {
            return Err(EngineError::Conflict(
                "Hoista already knows how that change ended.",
            ));
        }
        operation.state = if happened {
            OperationState::Happened
        } else {
            OperationState::DidNotHappen
        };
        operation.finished_at = Some(now());
        if let Some(evidence) = evidence {
            operation.message = Some(format!("An agent checked: {evidence}"));
        }
        self.engine.repository.put_operation(&operation).await?;
        self.engine
            .repository
            .event(
                actor.id(),
                None,
                "operation.checked",
                &format!(
                    "Checked at {}: {} {}.{}",
                    operation.provider.display_name(),
                    operation.target,
                    if happened {
                        "happened"
                    } else {
                        "didn't happen"
                    },
                    evidence
                        .map(|evidence| format!(" Evidence: {evidence}"))
                        .unwrap_or_default()
                ),
            )
            .await?;
        operation.key.clear();
        Ok(operation)
    }

    fn sending(self, key: &str) -> bool {
        self.engine
            .in_flight
            .lock()
            .is_ok_and(|flying| flying.contains(key))
    }
}

/// Every change sent to a provider, written down first.
pub mod recorder;
