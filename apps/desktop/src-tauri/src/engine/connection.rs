use super::{Local, Reply, Request};
use crate::error::Result;
use crate::remotes::Session;
use std::sync::Arc;

/// Which Hoista the screens show: this computer's, or a remote this
/// computer is signed in to.
#[derive(Clone)]
pub enum Connection {
    Local(Local),
    Remote(Arc<Session>),
}

impl Connection {
    pub async fn request(&self, request: &Request) -> Result<Reply> {
        request.check()?;
        match self {
            Self::Local(local) => local.request(request).await,
            Self::Remote(session) => session.request(request).await,
        }
    }
}
