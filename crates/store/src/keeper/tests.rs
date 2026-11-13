use super::*;
use std::collections::HashMap;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

#[derive(Default)]
struct Entries(std::sync::Mutex<HashMap<String, String>>);

#[async_trait::async_trait]
impl TokenVault for Entries {
    async fn read(&self, key: &str) -> Result<Secret, EngineError> {
        self.0
            .lock()
            .map_err(|_| EngineError::Store)?
            .get(key)
            .map(|value| Secret::new(value.clone()))
            .ok_or(EngineError::NotFound)
    }

    async fn write(&self, key: &str, value: Secret) -> Result<(), EngineError> {
        self.0
            .lock()
            .map_err(|_| EngineError::Store)?
            .insert(key.into(), value.expose().into());
        Ok(())
    }

    async fn delete(&self, key: &str) -> Result<(), EngineError> {
        self.0.lock().map_err(|_| EngineError::Store)?.remove(key);
        Ok(())
    }
}

struct Running {
    _folder: tempfile::TempDir,
    client: Arc<KeeperClient>,
    stop: tokio_util::sync::CancellationToken,
}

impl Drop for Running {
    fn drop(&mut self) {
        self.stop.cancel();
    }
}

async fn keeper(allowed_uid: Option<u32>) -> Running {
    keeper_for(allowed_uid, crate::Provider::Railway, "real-token").await
}

/// A keeper whose test provider on 127.0.0.1 stands in for `provider`, holding
/// `token` as account a1's.
async fn keeper_for(allowed_uid: Option<u32>, provider: crate::Provider, token: &str) -> Running {
    let folder = tempfile::tempdir().expect("test");
    let socket = folder.path().join("keeper.sock");
    let vault = Arc::new(Entries::default());
    vault
        .write("account/a1", Secret::new(token.into()))
        .await
        .expect("test");
    vault
        .write("agent/k1", Secret::new("agent-key".into()))
        .await
        .expect("test");
    let mut keeper = Keeper::new(vault, allowed_uid).expect("test");
    keeper.local_provider = Some(provider);
    let stop = tokio_util::sync::CancellationToken::new();
    tokio::spawn(Arc::new(keeper).serve(socket.clone(), stop.clone()));
    for _ in 0..100 {
        if socket.exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    Running {
        _folder: folder,
        client: Arc::new(KeeperClient::new(socket)),
        stop,
    }
}

/// A provider on this machine that answers once and hands back what it was sent.
async fn host() -> (String, tokio::task::JoinHandle<String>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("test");
    let address = format!("http://{}/graphql", listener.local_addr().expect("test"));
    let seen = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.expect("test");
        let mut request = Vec::new();
        let mut buffer = [0u8; 4096];
        loop {
            let read = stream.read(&mut buffer).await.expect("test");
            request.extend_from_slice(&buffer[..read]);
            let text = String::from_utf8_lossy(&request);
            if let Some(end) = text.find("\r\n\r\n") {
                let length = text[..end]
                    .lines()
                    .find_map(|line| {
                        line.to_ascii_lowercase()
                            .strip_prefix("content-length:")
                            .map(|value| value.trim().parse::<usize>().unwrap_or(0))
                    })
                    .unwrap_or(0);
                if request.len() >= end + 4 + length {
                    break;
                }
            }
            if read == 0 {
                break;
            }
        }
        stream
            .write_all(b"HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: 11\r\nconnection: close\r\n\r\n{\"ok\":true}")
            .await
            .expect("test");
        String::from_utf8_lossy(&request).into_owned()
    });
    (address, seen)
}

#[tokio::test]
async fn a_handle_reaches_the_provider_with_the_real_token() {
    let running = keeper(None).await;
    let (url, seen) = host().await;
    let answer = running
        .client
        .http(
            "account/a1",
            "POST",
            &url,
            vec![("content-type".into(), "application/json".into())],
            b"{\"query\":\"{ me { id } }\"}",
            false,
        )
        .await
        .expect("test");
    assert_eq!(answer.status, 200);
    assert_eq!(answer.body, b"{\"ok\":true}");
    let request = seen.await.expect("test").to_ascii_lowercase();
    assert!(
        request.contains("authorization: bearer real-token"),
        "{request}"
    );
    assert!(request.contains("{\"query\":\"{ me { id } }\"}"));
}

#[tokio::test]
async fn provider_tokens_are_never_handed_out() {
    let running = keeper(None).await;
    let vault = HandleVault::new(running.client.clone());
    let token = vault.read("account/a1").await.expect("test");
    assert_eq!(token.handle_key(), Some("account/a1"));
    assert!(!token.expose().contains("real-token"));
    // Asked for directly, the keeper refuses.
    let direct = running
        .client
        .call(&Request::Read {
            key: "account/a1".into(),
        })
        .await
        .expect("test");
    assert!(matches!(direct, Response::Refused { code, .. } if code == "keeper.provider_token"));
    assert!(matches!(
        vault.read("account/missing").await,
        Err(EngineError::NotFound)
    ));
}

#[tokio::test]
async fn other_entries_can_be_read_and_written() {
    let running = keeper(None).await;
    let vault = HandleVault::new(running.client.clone());
    assert_eq!(
        vault.read("agent/k1").await.expect("test").expose(),
        "agent-key"
    );
    vault
        .write("webhook/w1", Secret::new("signing".into()))
        .await
        .expect("test");
    assert_eq!(
        vault.read("webhook/w1").await.expect("test").expose(),
        "signing"
    );
    vault.delete("webhook/w1").await.expect("test");
    assert!(matches!(
        vault.read("webhook/w1").await,
        Err(EngineError::NotFound)
    ));
}

#[tokio::test]
async fn tokens_only_go_to_provider_apis() {
    let running = keeper(None).await;
    for url in [
        "https://example.com/steal",
        "http://backboard.railway.com/graphql/v2",
    ] {
        let refused = running
            .client
            .http("account/a1", "GET", url, Vec::new(), b"", false)
            .await;
        assert!(
            matches!(&refused, Err(EngineError::Provider { code, .. }) if code == "keeper.host"),
            "{url}"
        );
    }
    // Nor with a sign-in of the caller's own, nor for an entry that isn't a provider token.
    let (url, _seen) = host().await;
    let own = running
        .client
        .http(
            "account/a1",
            "GET",
            &url,
            vec![("Authorization".into(), "Bearer other".into())],
            b"",
            false,
        )
        .await;
    assert!(
        matches!(&own, Err(EngineError::Provider { code, .. }) if code == "keeper.authorization")
    );
    let agent = running
        .client
        .http("agent/k1", "GET", &url, Vec::new(), b"", false)
        .await;
    assert!(
        matches!(&agent, Err(EngineError::Provider { code, .. }) if code == "keeper.not_provider_token")
    );
}

/// What the provider on 127.0.0.1 saw when the keeper signed a request in as
/// `provider` with `token`.
async fn signed_as(provider: crate::Provider, token: &str) -> String {
    let running = keeper_for(None, provider, token).await;
    let (url, seen) = host().await;
    running
        .client
        .http("account/a1", "GET", &url, Vec::new(), b"", false)
        .await
        .expect("test");
    seen.await.expect("test").to_ascii_lowercase()
}

#[tokio::test]
async fn providers_sign_in_the_way_their_definition_says() {
    // Most providers take a Bearer token.
    let railway = signed_as(crate::Provider::Railway, "real-token").await;
    assert!(railway.contains("authorization: bearer real-token"));
    // Fly's macaroon tokens carry their own scheme and go as they are.
    let fly = signed_as(crate::Provider::Fly, "FlyV1 fm2_abc").await;
    assert!(fly.contains("authorization: flyv1 fm2_abc"), "{fly}");
    assert!(!fly.contains("bearer"));
    // Modal takes the ID and secret in its own headers, and no Authorization.
    let modal = signed_as(crate::Provider::Modal, "ak-1 as-2").await;
    assert!(modal.contains("x-modal-token-id: ak-1"), "{modal}");
    assert!(modal.contains("x-modal-token-secret: as-2"));
    assert!(!modal.contains("authorization:"));
}

#[tokio::test]
async fn a_caller_cannot_bring_the_providers_own_sign_in_headers() {
    let running = keeper_for(None, crate::Provider::Modal, "ak-1 as-2").await;
    let (url, _seen) = host().await;
    let own = running
        .client
        .http(
            "account/a1",
            "GET",
            &url,
            vec![("X-Modal-Token-Id".into(), "ak-other".into())],
            b"",
            false,
        )
        .await;
    assert!(
        matches!(&own, Err(EngineError::Provider { code, .. }) if code == "keeper.authorization")
    );
}

#[tokio::test]
async fn only_the_commands_a_provider_declares_run() {
    let running = keeper(None).await;
    let refused = running
        .client
        .provider_cli(
            crate::Provider::Fly,
            "account/a1",
            &["ssh", "console"],
            None,
            None,
        )
        .await;
    assert!(matches!(&refused, Err(EngineError::Provider { code, .. }) if code == "keeper.cli"));
    // A provider that declares no command-line tool runs nothing.
    let refused = running
        .client
        .provider_cli(
            crate::Provider::Neon,
            "account/a1",
            &["version"],
            None,
            None,
        )
        .await;
    assert!(matches!(&refused, Err(EngineError::Provider { code, .. }) if code == "keeper.cli"));
}

#[tokio::test]
async fn another_user_is_turned_away() {
    let someone_else = nix::unistd::Uid::effective().as_raw().wrapping_add(1);
    let running = keeper(Some(someone_else)).await;
    let vault = HandleVault::new(running.client.clone());
    assert!(matches!(
        vault.read("agent/k1").await,
        Err(EngineError::Provider { code, .. }) if code == "keeper.unreachable"
    ));
    // Its own user is answered.
    let mine = keeper(Some(nix::unistd::Uid::effective().as_raw())).await;
    let vault = HandleVault::new(mine.client.clone());
    assert_eq!(
        vault.read("agent/k1").await.expect("test").expose(),
        "agent-key"
    );
}
