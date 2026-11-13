//! Moving this computer's Hoista to a remote: an encrypted export with a
//! one-time passphrase, restored there, each project put back in its mode,
//! then this computer's agents paused so the old copy stops taking changes.

use super::{Remotes, Session, segment};
use crate::engine::{Connection, Request};
use crate::error::{Error, Result};
use base64::Engine as _;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// How many projects and accounts a Hoista holds.
#[derive(Serialize)]
pub struct Holdings {
    projects: usize,
    accounts: usize,
}

impl Holdings {
    fn is_empty(&self) -> bool {
        self.projects == 0 && self.accounts == 0
    }
}

/// What a remote and this computer hold, so a move is offered only into an
/// empty remote, and only when there is something here to bring.
#[derive(Serialize)]
pub struct Contents {
    remote: Holdings,
    here: Holdings,
}

/// What moved to the remote.
#[derive(Serialize)]
pub struct Move {
    projects: usize,
    sign_ins: u64,
    /// Projects that stayed watching on the remote: switch them to managed there.
    left_watching: Vec<String>,
}

#[derive(Deserialize)]
struct Page<T> {
    items: Vec<T>,
}

#[derive(Deserialize)]
struct Project {
    id: String,
    name: String,
    mode: String,
}

impl Remotes {
    pub async fn contents(&self, id: &str) -> Result<Contents> {
        let there = Connection::Remote(self.session(&self.find(id).await?)?.into());
        let here = Connection::Local(self.inner.local.clone());
        let (remote, here) = tokio::try_join!(holdings(&there), holdings(&here))?;
        Ok(Contents { remote, here })
    }

    pub async fn push(&self, id: &str) -> Result<Move> {
        let remote = self.find(id).await?;
        let session = self.session(&remote)?;
        session
            .send(&Request::get("/health"))
            .await?
            .data::<Value>()?;
        if !holdings(&Connection::Remote(session.clone().into()))
            .await?
            .is_empty()
        {
            return Err(Error::new(
                "remote.not_empty",
                format!(
                    "{} already has projects or accounts. A move only goes into an empty remote; nothing is merged.",
                    remote.name
                ),
            ));
        }
        let local = &self.inner.local;
        let projects: Page<Project> = local.request(&Request::get("/projects")).await?.data()?;

        crate::files::create_private_folder(&self.inner.moves_folder)?;
        let file = self
            .inner
            .moves_folder
            .join(format!("move-{}.hoista", uuid::Uuid::new_v4()));
        let passphrase = random_hex(32);
        let restored = send_export(self, &session, &file, &passphrase).await;
        // The export holds every secret; it's gone whether or not the move worked.
        let _already_gone = std::fs::remove_file(&file);
        let restored = restored?;

        let mut left_watching = Vec::new();
        for project in projects
            .items
            .iter()
            .filter(|project| project.mode == "managed")
        {
            let path = format!("/projects/{}/mode", segment(&project.id));
            let managed = Request::new("PUT", path, Some(json!({ "mode": "managed" })));
            let reply = session.send(&managed).await;
            if !matches!(reply, Ok(reply) if !reply.failed()) {
                left_watching.push(project.name.clone());
            }
        }
        let pause = Request::new(
            "PATCH",
            "/settings",
            Some(json!({ "agents_enabled": false })),
        );
        local.request(&pause).await?.data::<Value>()?;
        Ok(Move {
            projects: projects.items.len(),
            sign_ins: restored,
            left_watching,
        })
    }
}

/// Exports this computer's Hoista to `file` and restores it on the remote.
/// Returns how many provider sign-ins the remote restored.
async fn send_export(
    remotes: &Remotes,
    session: &Session,
    file: &std::path::Path,
    passphrase: &str,
) -> Result<u64> {
    #[derive(Deserialize)]
    struct Restored {
        credentials_staged: u64,
    }
    let export = json!({ "path": file, "passphrase": passphrase });
    let local = &remotes.inner.local;
    local
        .request(&Request::new("POST", "/transfer/export", Some(export)))
        .await?
        .data::<Value>()?;
    let data = base64::engine::general_purpose::STANDARD.encode(std::fs::read(file)?);
    let receive = json!({ "passphrase": passphrase, "data": data });
    let restored: Restored = session
        .send(&Request::new("POST", "/transfer/receive", Some(receive)))
        .await?
        .data()?;
    Ok(restored.credentials_staged)
}

async fn holdings(connection: &Connection) -> Result<Holdings> {
    let count = |path: &'static str| async move {
        let page: Page<Value> = connection.request(&Request::get(path)).await?.data()?;
        Ok::<_, Error>(page.items.len())
    };
    let (projects, accounts) = tokio::try_join!(count("/projects"), count("/accounts"))?;
    Ok(Holdings { projects, accounts })
}

fn random_hex(bytes: usize) -> String {
    let mut buffer = vec![0u8; bytes];
    rand::rng().fill_bytes(&mut buffer);
    buffer.iter().map(|byte| format!("{byte:02x}")).collect()
}
