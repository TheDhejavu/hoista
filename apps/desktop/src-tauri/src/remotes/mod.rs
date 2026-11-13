//! Your remotes: other Hoistas the app can show instead of this computer's,
//! each on a server of yours. The app keeps a list of them, signs this
//! computer in to each one, and decides which Hoista every screen shows.
//! Anything a remote's owner check guards still happens on its own page.

mod address;
mod book;
mod keys;
mod moves;
mod relay;
mod session;
mod sign_in;

pub use book::Remote;
pub use moves::{Contents, Move};
pub use session::Session;

use crate::engine::{Connection, Local, Request};
use crate::error::{Error, Result};
use book::{Book, THIS_COMPUTER};
use keys::Keys;
use serde::Serialize;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::async_runtime::JoinHandle;
use tokio::sync::{OnceCell, watch};

/// How long a health check's answer is reused in the list.
/// The longest a remote's name is shown, in characters.
const MAX_NAME_CHARS: usize = 40;
const REACH_FRESH_FOR: Duration = Duration::from_secs(15);
const LIST_HEALTH_TIMEOUT: Duration = Duration::from_secs(4);
const SWITCH_HEALTH_TIMEOUT: Duration = Duration::from_secs(3);
const NEW_SERVER_HEALTH_TIMEOUT: Duration = Duration::from_secs(8);
/// How long a sign-in waits for the person in the browser.
const SIGN_IN_WAIT: Duration = Duration::from_secs(10 * 60);

/// `GET /remotes` for the screens.
#[derive(Serialize)]
pub struct View {
    showing: String,
    items: Vec<Item>,
    waiting: Option<Waiting>,
    problem: Option<Problem>,
}

#[derive(Serialize)]
struct Item {
    #[serde(flatten)]
    remote: Remote,
    signed_in: bool,
    reachable: Option<bool>,
}

#[derive(Serialize, Clone)]
struct Waiting {
    remote_id: String,
    link: String,
}

#[derive(Serialize, Clone)]
struct Problem {
    remote_id: String,
    message: String,
}

/// What changes while the app runs.
#[derive(Default)]
struct State {
    showing: String,
    waiting: Option<(Waiting, JoinHandle<()>)>,
    problem: Option<Problem>,
    reach: HashMap<String, (Instant, bool)>,
}

/// The remotes service. Cheap to clone; every clone is the same service.
#[derive(Clone)]
pub struct Remotes {
    inner: Arc<Inner>,
}

struct Inner {
    book: Book,
    keys: Keys,
    http: reqwest::Client,
    local: Local,
    shown: watch::Sender<Connection>,
    state: Mutex<State>,
    restored: OnceCell<()>,
    computer: String,
    moves_folder: PathBuf,
    open_page: Box<dyn Fn(&str) + Send + Sync>,
}

pub struct Setup {
    pub local: Local,
    pub state_folder: PathBuf,
    pub config_folder: PathBuf,
    pub moves_folder: PathBuf,
    pub computer: String,
    pub open_page: Box<dyn Fn(&str) + Send + Sync>,
}

impl Remotes {
    pub fn new(setup: Setup) -> (Self, watch::Receiver<Connection>) {
        let (shown, receiver) = watch::channel(Connection::Local(setup.local.clone()));
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .expect("an HTTP client with no special settings");
        let inner = Inner {
            book: Book::new(setup.state_folder, setup.config_folder),
            keys: Keys,
            http,
            local: setup.local,
            shown,
            state: Mutex::new(State {
                showing: THIS_COMPUTER.to_owned(),
                ..State::default()
            }),
            restored: OnceCell::new(),
            computer: setup.computer,
            moves_folder: setup.moves_folder,
            open_page: setup.open_page,
        };
        (
            Self {
                inner: Arc::new(inner),
            },
            receiver,
        )
    }

    /// The Hoista the screens show now.
    pub async fn connection(&self) -> Connection {
        self.restore().await;
        self.inner.shown.borrow().clone()
    }

    /// Shows the Hoista the app showed last time, once. If that remote is
    /// signed out or gone, the app starts on this computer.
    async fn restore(&self) {
        self.inner
            .restored
            .get_or_init(|| async {
                let last = self.inner.book.last_shown();
                if last != THIS_COMPUTER {
                    let _started_here = self.switch_to(&last).await;
                }
            })
            .await;
    }

    pub async fn view(&self) -> Result<View> {
        let remotes = self.list().await;
        let checks = remotes.iter().map(|remote| self.reachable(remote));
        let reachable = futures_util::future::join_all(checks).await;
        let items = remotes
            .into_iter()
            .zip(reachable)
            .map(|(remote, reachable)| Item {
                signed_in: matches!(self.inner.keys.read(&remote.id), Ok(Some(_))),
                reachable: Some(reachable),
                remote,
            })
            .collect();
        let state = self.state();
        Ok(View {
            showing: state.showing.clone(),
            items,
            waiting: state.waiting.as_ref().map(|(waiting, _)| waiting.clone()),
            problem: state.problem.clone(),
        })
    }

    /// Adds a remote by its address. It must answer first; then this
    /// computer signs in to it straight away, so it's ready to show.
    pub async fn add(&self, typed: &str, name: Option<&str>) -> Result<View> {
        let url = address::from_typed(typed).ok_or_else(|| {
            Error::new(
                "server.invalid",
                "Give its address, such as https://hoista.example.com, or http://localhost:8787 for one in Docker on this computer.",
            )
        })?;
        let mut remotes = self.list().await;
        if remotes.iter().any(|remote| remote.url == url) {
            return Err(Error::new(
                "remote.exists",
                "That remote is already in your list.",
            ));
        }
        if !self.answers_health(&url, LIST_HEALTH_TIMEOUT).await {
            let host = url.split("://").nth(1).unwrap_or(&url);
            return Err(Error::new(
                "server.unreachable",
                format!(
                    "Nothing answered at {host}. Check the address, and that Hoista is running there."
                ),
            ));
        }
        let name = name
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .map_or_else(
                || address::default_name(&url),
                |name| name.chars().take(MAX_NAME_CHARS).collect(),
            );
        let remote = new_remote(name, url.clone());
        remotes.push(remote.clone());
        self.inner.book.save(&remotes)?;
        self.state()
            .reach
            .insert(remote.id.clone(), (Instant::now(), true));
        // The first remote is also the one this computer's agents are told about.
        if self.engine_remote().await.is_none() {
            let connect = Request::new(
                "POST",
                "/server/connect",
                Some(serde_json::json!({ "url": url })),
            );
            self.inner
                .local
                .request(&connect)
                .await?
                .data::<serde_json::Value>()?;
        }
        self.start_sign_in(&remote).await?;
        self.view().await
    }

    pub async fn rename(&self, id: &str, name: &str) -> Result<View> {
        let name = name.trim();
        if name.is_empty() {
            return Err(Error::new("remote.invalid", "Give it a name."));
        }
        let mut remotes = self.list().await;
        let remote = remotes
            .iter_mut()
            .find(|remote| remote.id == id)
            .ok_or_else(not_found)?;
        remote.name = name.chars().take(MAX_NAME_CHARS).collect();
        self.inner.book.save(&remotes)?;
        self.view().await
    }

    /// Forgets a remote on this computer only: its key here and its place
    /// in the list. It keeps running where it is.
    pub async fn forget(&self, id: &str) -> Result<View> {
        let remotes = self.list().await;
        let Some(remote) = remotes.iter().find(|remote| remote.id == id).cloned() else {
            return self.view().await;
        };
        self.cancel_sign_in(id);
        if self.state().showing == id {
            self.switch_to(THIS_COMPUTER).await?;
        }
        self.inner.keys.remove(id)?;
        let kept: Vec<Remote> = remotes.into_iter().filter(|item| item.id != id).collect();
        self.inner.book.save(&kept)?;
        self.state().reach.remove(id);
        if self.engine_remote().await.as_deref() == Some(remote.url.as_str()) {
            self.inner
                .local
                .request(&Request::new("DELETE", "/server", None))
                .await?;
        }
        self.view().await
    }

    pub async fn sign_in(&self, id: &str) -> Result<View> {
        let remote = self.find(id).await?;
        self.start_sign_in(&remote).await?;
        self.view().await
    }

    /// Cancels a sign-in under way, or signs this computer out of the remote.
    pub async fn sign_out(&self, id: &str) -> Result<View> {
        let remote = self.find(id).await?;
        self.cancel_sign_in(&remote.id);
        {
            let mut state = self.state();
            if state
                .problem
                .as_ref()
                .is_some_and(|problem| problem.remote_id == remote.id)
            {
                state.problem = None;
            }
        }
        if self.state().showing == remote.id {
            self.switch_to(THIS_COMPUTER).await?;
        }
        self.inner.keys.remove(&remote.id)?;
        self.view().await
    }

    /// Shows this computer or a remote on every screen. The remote is asked
    /// fresh: switching to one that isn't running would leave every screen
    /// waiting on it.
    pub async fn show(&self, showing: &str) -> Result<View> {
        if showing != THIS_COMPUTER {
            let remote = self.find(showing).await?;
            let up = self
                .answers_health(&remote.url, SWITCH_HEALTH_TIMEOUT)
                .await;
            self.state()
                .reach
                .insert(remote.id.clone(), (Instant::now(), up));
            if !up {
                return Err(Error::new(
                    "remote.not_answering",
                    format!("{} isn’t answering. Start it, then try again.", remote.name),
                ));
            }
        }
        self.switch_to(showing).await?;
        self.view().await
    }

    async fn switch_to(&self, showing: &str) -> Result<()> {
        let connection = if showing == THIS_COMPUTER {
            Connection::Local(self.inner.local.clone())
        } else {
            Connection::Remote(Arc::new(self.session(&self.find(showing).await?)?))
        };
        self.inner.shown.send_replace(connection);
        self.state().showing = showing.to_owned();
        self.inner.book.remember_shown(showing)
    }

    /// A session with a remote this computer is signed in to.
    fn session(&self, remote: &Remote) -> Result<Session> {
        let key = self.inner.keys.read(&remote.id)?.ok_or_else(|| {
            Error::new(
                "server.signed_out",
                format!("Sign this computer in to {} first.", remote.name),
            )
        })?;
        Ok(Session::new(remote.clone(), key, self.inner.http.clone()))
    }

    /// Opens the remote's sign-in page and waits for the browser in the background.
    async fn start_sign_in(&self, remote: &Remote) -> Result<()> {
        self.cancel_sign_in(&remote.id);
        self.state().problem = None;
        let pending = sign_in::start(&self.inner.http, remote, &self.inner.computer).await?;
        let link = pending.link.clone();
        let service = self.clone();
        let waiting_for = remote.clone();
        let task = tauri::async_runtime::spawn(async move {
            let inner = &service.inner;
            let finished = sign_in::finish(pending, &inner.http, &waiting_for, &inner.keys);
            let outcome = tokio::time::timeout(SIGN_IN_WAIT, finished)
                .await
                .unwrap_or_else(|_| {
                    Err("It wasn’t finished within ten minutes. Try again.".to_owned())
                });
            let mut state = service.state();
            state.waiting = None;
            state.problem = outcome.err().map(|message| Problem {
                remote_id: waiting_for.id.clone(),
                message,
            });
        });
        let waiting = Waiting {
            remote_id: remote.id.clone(),
            link: link.clone(),
        };
        self.state().waiting = Some((waiting, task));
        (self.inner.open_page)(&link);
        Ok(())
    }

    fn cancel_sign_in(&self, remote_id: &str) {
        let mut state = self.state();
        if state
            .waiting
            .as_ref()
            .is_some_and(|(waiting, _)| waiting.remote_id == remote_id)
            && let Some((_, task)) = state.waiting.take()
        {
            task.abort();
        }
    }

    /// The list, with the engine's own remote in it (added the first time it's seen).
    async fn list(&self) -> Vec<Remote> {
        let mut remotes = self.inner.book.list();
        if let Some(own) = self.engine_remote().await
            && !remotes.iter().any(|remote| remote.url == own)
        {
            remotes.push(new_remote(address::default_name(&own), own));
            // Shown without it next time if the file can't be written.
            let _unsaved = self.inner.book.save(&remotes);
        }
        remotes
    }

    async fn find(&self, id: &str) -> Result<Remote> {
        self.list()
            .await
            .into_iter()
            .find(|remote| remote.id == id)
            .ok_or_else(not_found)
    }

    /// The remote this computer's engine was set up with, or connected to first.
    async fn engine_remote(&self) -> Option<String> {
        #[derive(serde::Deserialize)]
        struct Settings {
            server: Option<Server>,
        }

        #[derive(serde::Deserialize)]
        struct Server {
            url: Option<String>,
        }
        let reply = self
            .inner
            .local
            .request(&Request::get("/settings"))
            .await
            .ok()?;
        let settings: Settings = reply.data().ok()?;
        address::origin(&settings.server?.url?)
    }

    async fn reachable(&self, remote: &Remote) -> bool {
        if let Some((at, up)) = self.state().reach.get(&remote.id)
            && at.elapsed() < REACH_FRESH_FOR
        {
            return *up;
        }
        let up = self.answers_health(&remote.url, LIST_HEALTH_TIMEOUT).await;
        self.state()
            .reach
            .insert(remote.id.clone(), (Instant::now(), up));
        up
    }

    /// Whether a Hoista answers at `address`, such as one just put on a server.
    pub async fn answers_at(&self, address: &str) -> bool {
        match address::origin(address) {
            Some(origin) => {
                self.answers_health(&origin, NEW_SERVER_HEALTH_TIMEOUT)
                    .await
            }
            None => false,
        }
    }

    /// Whether a Hoista answers its health check at `origin` in time.
    async fn answers_health(&self, origin: &str, within: Duration) -> bool {
        let check = self
            .inner
            .http
            .get(format!("{origin}/healthz"))
            .timeout(within);
        matches!(check.send().await, Ok(response) if response.status().is_success())
    }

    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.inner.state.lock().expect("remotes state lock")
    }
}

fn new_remote(name: String, url: String) -> Remote {
    let added_at = time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_default();
    Remote {
        id: uuid::Uuid::new_v4().to_string(),
        name,
        url,
        added_at,
    }
}

fn not_found() -> Error {
    Error::new(
        "remote.not_found",
        "That remote isn’t in your list any more.",
    )
}

/// One path segment, percent-encoded.
fn segment(text: &str) -> String {
    percent_encoding::utf8_percent_encode(text, percent_encoding::NON_ALPHANUMERIC).to_string()
}
