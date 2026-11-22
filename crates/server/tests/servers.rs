//! Servers Hoista makes: the keys it keeps for them, the host key it replaces
//! on the first connection, what comes back from a command, and when agents
//! with Full access may work on one without asking.

use hoista_server::{
    Provider,
    application::{
        engine::{Actor, Engine},
        ports::{
            catalog::{CatalogRepository, EngineError},
            owner_check::FixedOwnerCheck,
            providers::{ProviderAdapter, ProviderDetails, ProviderNew, RemoteResource},
            servers::{CommandOutput, CommandRun, RemoteServer, ServerAddress},
            vault::TokenVault,
        },
        servers::{ServerAccess, ServerService, vault_reference},
    },
    domain::{Secret, model::*},
    infrastructure::{
        build::LocalBuilder, inspection::LocalInspector, paths::StatePaths, store::SqliteStore,
    },
};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

/// Keeps what's written; anything else read is the account's token.
#[derive(Default)]
struct Vault(Mutex<HashMap<String, String>>);

#[async_trait::async_trait]
impl TokenVault for Vault {
    async fn read(&self, reference: &str) -> Result<Secret, EngineError> {
        let kept = self.0.lock().expect("fixture").get(reference).cloned();
        match kept {
            Some(value) => Ok(Secret::new(value)),
            None if reference.starts_with("server") => Err(EngineError::NotFound),
            None => Ok(Secret::new("fixture-token".into())),
        }
    }

    async fn write(&self, reference: &str, value: Secret) -> Result<(), EngineError> {
        self.0
            .lock()
            .expect("fixture")
            .insert(reference.to_owned(), value.expose().to_owned());
        Ok(())
    }

    async fn delete(&self, reference: &str) -> Result<(), EngineError> {
        self.0.lock().expect("fixture").remove(reference);
        Ok(())
    }
}

/// A provider that makes servers and finds what it made.
#[derive(Default)]
struct Servers {
    made: Mutex<Vec<String>>,
    keys: Mutex<Vec<String>>,
}

#[async_trait::async_trait]
impl ProviderAdapter for Servers {
    fn provider(&self) -> Provider {
        Provider::Hetzner
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            discover: true,
            new_server: true,
            server_access: true,
            ..Capabilities::default()
        }
    }

    async fn verify(&self, _: &Secret, _: &str) -> Result<Vec<String>, EngineError> {
        Ok(vec![])
    }

    async fn discover(&self, _: &Secret, _: &str) -> Result<Vec<RemoteResource>, EngineError> {
        Ok(self
            .made
            .lock()
            .expect("fixture")
            .iter()
            .enumerate()
            .map(|(index, name)| RemoteResource {
                reference: format!("server:{index}"),
                name: name.clone(),
                kind: ResourceKind::Server,
                region: None,
                status: ResourceStatus::Deploying,
                url: None,
                group: None,
            })
            .collect())
    }

    async fn provider_add(
        &self,
        _: &Secret,
        _: &str,
        _: &str,
        _: Option<&str>,
        what: &ProviderNew<'_>,
    ) -> Result<(Vec<String>, String), EngineError> {
        let ProviderNew::Server {
            name,
            setup: Some(setup),
            ..
        } = *what
        else {
            return Err(hoista_server::application::ports::providers::unsupported());
        };
        self.keys
            .lock()
            .expect("fixture")
            .push(setup.authorized_key.clone());
        let mut made = self.made.lock().expect("fixture");
        made.push(name.to_owned());
        Ok((
            vec![format!("server:{}", made.len() - 1)],
            format!("Made {name}"),
        ))
    }

    async fn provider_details(
        &self,
        _: &Secret,
        _: &str,
        _: &str,
    ) -> Result<ProviderDetails, EngineError> {
        Ok(ProviderDetails {
            deployments: vec![],
            variable_names: None,
            shared_variable_names: None,
            domains: vec![],
            references: vec![],
            sleeps: None,
            config: None,
            volumes: vec![],
            tcp_proxies: vec![],
            private_domain: None,
            addresses: vec![],
            public_ips: vec!["2001:db8::1".into(), "203.0.113.5".into()],
        })
    }
}

/// A server that answers every command, and gives out a new host key.
#[derive(Default)]
struct Answering {
    ran: Mutex<Vec<(String, String, String)>>,
}

const NEW_HOST_KEY: &str =
    "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAINdamAGCsQq31Uv+08lkBzoO4XLz2qYjJa8CGmj3B1Ea root@web-1";

#[async_trait::async_trait]
impl RemoteServer for Answering {
    async fn run(
        &self,
        address: ServerAddress<'_>,
        access: &ServerAccess,
        run: CommandRun<'_>,
    ) -> Result<CommandOutput, EngineError> {
        self.ran.lock().expect("fixture").push((
            address.host.to_owned(),
            access.host_key.clone(),
            run.command.to_owned(),
        ));
        let stdout = if run.command.contains("ssh-keygen") {
            format!("{NEW_HOST_KEY}\n")
        } else if run.command.contains("cloud-init status") {
            "done\n".to_owned()
        } else {
            "deployed with AKIAIOSFODNN7EXAMPLE\n".to_owned()
        };
        Ok(CommandOutput {
            exit_code: Some(0),
            stdout: stdout.into_bytes(),
            ..CommandOutput::default()
        })
    }
}

struct Fixture {
    _temp: tempfile::TempDir,
    engine: Engine,
    vault: Arc<Vault>,
    server: Arc<Answering>,
    provider: Arc<Servers>,
}

async fn setup(owner_confirms: bool) -> Fixture {
    let temp = tempfile::tempdir().expect("temp");
    let paths = StatePaths::new(Some(temp.path().join("state"))).expect("paths");
    let store = SqliteStore::open(&paths).await.expect("store");
    let vault = Arc::new(Vault::default());
    let server = Arc::new(Answering::default());
    let provider = Arc::new(Servers::default());
    let engine = Engine::new(
        Arc::new(store.clone()),
        vault.clone(),
        Arc::new(LocalInspector),
        vec![provider.clone()],
        Arc::new(LocalBuilder::new(paths.root())),
    )
    .with_remote_servers(server.clone())
    .with_owner_check(Arc::new(FixedOwnerCheck(owner_confirms)));
    store
        .put_account(&Account {
            last_discovery: None,
            id: "hetzner".into(),
            provider: Provider::Hetzner,
            label: "Home lab".into(),
            external_id: "hz-0000000000000000".into(),
            scopes: vec![],
            status: AccountStatus::Connected,
            connected_at: "2026-11-22T00:00:00Z".into(),
            credential: None,
            in_1password: false,
        })
        .await
        .expect("account");
    Fixture {
        _temp: temp,
        engine,
        vault,
        server,
        provider,
    }
}

/// Makes server `web-1` in a new project and returns its resource id.
async fn made(fixture: &Fixture) -> String {
    let project = fixture
        .engine
        .projects()
        .create_project(
            &Actor::Person,
            "media".into(),
            Source::Unlinked,
            Mode::Watching,
        )
        .await
        .expect("project");
    let (_, resources) = fixture
        .engine
        .projects()
        .add_to_project(
            &Actor::Person,
            &project.id,
            "hetzner",
            ProviderNew::Server {
                name: "web-1",
                server_type: None,
                location: None,
                volume_gb: None,
                backups: false,
                setup: None,
            },
        )
        .await
        .expect("server made");
    resources
        .into_iter()
        .find(|resource| resource.kind == ResourceKind::Server)
        .expect("the server joins the project")
        .id
}

fn kept(fixture: &Fixture, resource_id: &str) -> ServerAccess {
    let value = fixture
        .vault
        .0
        .lock()
        .expect("fixture")
        .get(&vault_reference(resource_id))
        .cloned()
        .expect("Hoista keeps the server's key");
    ServerAccess::from_kept(&Secret::new(value)).expect("kept shape")
}

#[tokio::test]
async fn a_new_server_gets_its_own_keys_and_hoista_keeps_them() {
    let fixture = setup(true).await;
    let id = made(&fixture).await;
    let access = kept(&fixture, &id);
    assert_eq!(access.user, "hoista");
    assert!(access.replace_host_key);
    assert!(!access.agents_full_access);
    // The provider was given the public half of the key Hoista kept.
    let given = fixture.provider.keys.lock().expect("fixture")[0].clone();
    assert!(given.starts_with("ssh-ed25519 "));
    assert!(!access.private_key.expose().contains(&given));
    // Nothing is left waiting under the name it was made with.
    assert!(
        !fixture
            .vault
            .0
            .lock()
            .expect("fixture")
            .keys()
            .any(|key| key.starts_with("server-pending/"))
    );
}

#[tokio::test]
async fn the_first_connection_replaces_the_host_key_and_secrets_stay_out() {
    let fixture = setup(true).await;
    let id = made(&fixture).await;
    let first_key = kept(&fixture, &id).host_key;
    let status = fixture
        .engine
        .servers()
        .status(&Actor::Person, &id)
        .await
        .expect("status");
    let ran = fixture.server.ran.lock().expect("fixture").clone();
    // The host key is replaced over a connection checked with the first one,
    // and the look itself is checked with the new one, over IPv4.
    assert!(ran[0].2.contains("ssh-keygen"));
    assert_eq!(ran[0].1, first_key);
    assert_eq!(ran[1].1, NEW_HOST_KEY);
    assert_eq!(ran[1].0, "203.0.113.5");
    let access = kept(&fixture, &id);
    assert_eq!(access.host_key, NEW_HOST_KEY);
    assert!(!access.replace_host_key);
    assert!(!status.output.contains("AKIAIOSFODNN7EXAMPLE"));
}

#[tokio::test]
async fn full_access_on_a_server_is_the_persons_choice_and_risky_steps_still_ask() {
    let fixture = setup(true).await;
    let id = made(&fixture).await;
    let servers = fixture.engine.servers();
    let everyday = ServerService::run_target(&id, "docker compose up -d");
    let risky = ServerService::run_target(&id, "sudo reboot");
    let key = ServerService::deploy_key_target(&id, "ci", "/srv/app/deploy.sh");
    assert!(!servers.allowed_without_asking(&everyday).await.unwrap());
    servers
        .set_agents_full_access(&Actor::Person, &id, true)
        .await
        .expect("the person turns it on");
    assert!(servers.allowed_without_asking(&everyday).await.unwrap());
    assert!(!servers.allowed_without_asking(&risky).await.unwrap());
    assert!(!servers.allowed_without_asking(&key).await.unwrap());
    // An agent can't change it.
    let agent = Actor::Agent { id: "agent".into() };
    assert!(
        servers
            .set_agents_full_access(&agent, &id, false)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn turning_full_access_on_needs_the_person_to_confirm() {
    let fixture = setup(false).await;
    let id = made(&fixture).await;
    assert!(
        fixture
            .engine
            .servers()
            .set_agents_full_access(&Actor::Person, &id, true)
            .await
            .is_err()
    );
    assert!(!kept(&fixture, &id).agents_full_access);
}

#[tokio::test]
async fn secrets_on_the_server_are_never_read() {
    let fixture = setup(true).await;
    let id = made(&fixture).await;
    let refused = fixture
        .engine
        .servers()
        .read_file(&Actor::Person, &id, "/etc/shadow")
        .await
        .unwrap_err();
    assert!(matches!(refused, EngineError::Conflict(_)), "{refused:?}");
}

#[tokio::test]
async fn a_server_is_setting_up_until_its_first_boot_is_done_and_its_history_is_kept() {
    let fixture = setup(true).await;
    let id = made(&fixture).await;
    let resource = fixture
        .engine
        .repository
        .resource(&id)
        .await
        .expect("read")
        .expect("server");
    let servers = fixture.engine.servers();
    assert_eq!(
        servers.setup_status(&resource).await.expect("check"),
        Some(ResourceStatus::Live)
    );
    // Once it's done, Hoista stops checking and the provider's word stands.
    assert_eq!(servers.setup_status(&resource).await.expect("check"), None);
    assert!(kept(&fixture, &id).setup_finished);
    servers
        .run(&Actor::Person, &id, "docker compose up -d", None, None)
        .await
        .expect("run");
    let kinds: Vec<String> = servers
        .activity(&Actor::Person, &id)
        .await
        .expect("activity")
        .into_iter()
        .map(|event| event.kind)
        .collect();
    assert_eq!(kinds, ["server.command", "server.ready"]);
}
