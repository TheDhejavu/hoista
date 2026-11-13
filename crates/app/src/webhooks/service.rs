//! The webhooks service: webhooks and their deliveries.

use crate::engine::Engine;

/// Webhooks and their deliveries.
#[derive(Clone, Copy)]
pub struct WebhookService<'a> {
    pub(crate) engine: &'a Engine,
}

impl Engine {
    /// Webhooks and their deliveries.
    #[must_use]
    pub fn webhooks(&self) -> WebhookService<'_> {
        WebhookService { engine: self }
    }
}
