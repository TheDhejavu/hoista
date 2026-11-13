//! The agents service: agents, their access, approvals and sign-in.

use crate::engine::Engine;

/// Agents, their access, approvals and sign-in.
#[derive(Clone, Copy)]
pub struct AgentService<'a> {
    pub(crate) engine: &'a Engine,
}

impl Engine {
    /// Agents, their access, approvals and sign-in.
    #[must_use]
    pub fn agents(&self) -> AgentService<'_> {
        AgentService { engine: self }
    }
}
