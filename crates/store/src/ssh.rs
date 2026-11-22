//! Running commands on a server over SSH with the `russh` client. The key
//! stays in memory; the server's host key must be the one Hoista keeps for
//! it, so a server that shows another is never reached.

use hoista_app::{
    ports::{
        catalog::EngineError,
        servers::{CommandOutput, CommandRun, RemoteServer, ServerAddress},
    },
    servers::ServerAccess,
};
use russh::{
    ChannelMsg, Disconnect,
    client::{self, Handler},
    keys::{PrivateKeyWithHashAlg, PublicKey, PublicKeyOrCertificate, decode_secret_key},
};
use std::{sync::Arc, time::Duration};

/// How long connecting and signing in may take.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(20);

/// The SSH client Hoista reaches its servers with.
#[derive(Debug, Default)]
pub struct SshServers;

/// Accepts only the host key Hoista keeps, and notes when the server showed
/// another, to say so.
struct Pinned {
    host_key: PublicKey,
    refused: Arc<std::sync::atomic::AtomicBool>,
}

impl Handler for Pinned {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        shown: &PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        let same = matches!(
            shown,
            PublicKeyOrCertificate::PublicKey { key, .. } if key.key_data() == self.host_key.key_data()
        );
        if !same {
            self.refused
                .store(true, std::sync::atomic::Ordering::Relaxed);
        }
        Ok(same)
    }
}

fn unreachable_server() -> EngineError {
    EngineError::Provider {
        evidence: None,
        code: "server.unreachable".into(),
        message: "Hoista couldn't reach the server over SSH. Check it's running, and that its firewall lets port 22 in.".into(),
    }
}

fn host_key_changed() -> EngineError {
    EngineError::Provider {
        evidence: None,
        code: "server.host_key_changed".into(),
        message: "The server showed a different host key from the one Hoista keeps for it, so Hoista didn't connect. Someone may be between you and the server, or it was rebuilt outside Hoista.".into(),
    }
}

fn refused_sign_in() -> EngineError {
    EngineError::Provider {
        evidence: None,
        code: "server.sign_in_refused".into(),
        message: "The server refused Hoista's key. It may have been removed from the server's authorized_keys.".into(),
    }
}

/// Keeps up to `max` bytes, noting when some were cut.
fn keep(into: &mut Vec<u8>, data: &[u8], max: usize, cut: &mut bool) {
    let room = max.saturating_sub(into.len());
    if data.len() > room {
        *cut = true;
    }
    into.extend_from_slice(&data[..data.len().min(room)]);
}

#[async_trait::async_trait]
impl RemoteServer for SshServers {
    async fn run(
        &self,
        address: ServerAddress<'_>,
        access: &ServerAccess,
        run: CommandRun<'_>,
    ) -> Result<CommandOutput, EngineError> {
        let host_key = PublicKey::from_openssh(&access.host_key).map_err(|_| EngineError::Store)?;
        let key =
            decode_secret_key(access.private_key.expose(), None).map_err(|_| EngineError::Store)?;
        let refused = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let config = Arc::new(client::Config {
            inactivity_timeout: Some(run.timeout + CONNECT_TIMEOUT),
            ..client::Config::default()
        });
        let handler = Pinned {
            host_key,
            refused: refused.clone(),
        };
        let connecting = client::connect(config, (address.host, address.port), handler);
        let mut session = match tokio::time::timeout(CONNECT_TIMEOUT, connecting).await {
            Ok(Ok(session)) => session,
            _ if refused.load(std::sync::atomic::Ordering::Relaxed) => {
                return Err(host_key_changed());
            }
            _ => return Err(unreachable_server()),
        };
        let signed_in = tokio::time::timeout(
            CONNECT_TIMEOUT,
            session.authenticate_publickey(
                access.user.clone(),
                PrivateKeyWithHashAlg::new(Arc::new(key), None),
            ),
        )
        .await
        .map_err(|_| unreachable_server())?
        .map_err(|_| unreachable_server())?;
        if !signed_in.success() {
            return Err(refused_sign_in());
        }
        let mut channel = session
            .channel_open_session()
            .await
            .map_err(|_| unreachable_server())?;
        channel
            .exec(true, run.command)
            .await
            .map_err(|_| unreachable_server())?;
        if let Some(input) = run.input {
            channel
                .data(input)
                .await
                .map_err(|_| unreachable_server())?;
        }
        channel.eof().await.map_err(|_| unreachable_server())?;
        let mut output = CommandOutput::default();
        let reading = async {
            while let Some(message) = channel.wait().await {
                match message {
                    ChannelMsg::Data { data } => {
                        keep(
                            &mut output.stdout,
                            &data,
                            run.max_output,
                            &mut output.truncated,
                        );
                    }
                    ChannelMsg::ExtendedData { data, .. } => {
                        keep(
                            &mut output.stderr,
                            &data,
                            run.max_output,
                            &mut output.truncated,
                        );
                    }
                    ChannelMsg::ExitStatus { exit_status } => output.exit_code = Some(exit_status),
                    _ => {}
                }
            }
        };
        if tokio::time::timeout(run.timeout, reading).await.is_err() {
            output.timed_out = true;
        }
        let _ = session
            .disconnect(Disconnect::ByApplication, "", "en")
            .await;
        Ok(output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_is_kept_up_to_the_limit() {
        let (mut kept, mut cut) = (Vec::new(), false);
        keep(&mut kept, b"hello ", 8, &mut cut);
        assert!(!cut);
        keep(&mut kept, b"world", 8, &mut cut);
        assert_eq!(kept, b"hello wo");
        assert!(cut);
    }

    /// A small SSH server in this process: it takes only `hoista` with one
    /// key, and answers a command with what it was asked and exit code 3.
    struct Answering {
        user_key: PublicKey,
        channels: Vec<russh::Channel<russh::server::Msg>>,
    }

    impl russh::server::Handler for Answering {
        type Error = russh::Error;

        async fn auth_publickey(
            &mut self,
            user: &str,
            key: russh::server::VerifiedPublicKey<'_>,
        ) -> Result<russh::server::Auth, Self::Error> {
            Ok(
                if user == "hoista" && key.public_key.key_data() == self.user_key.key_data() {
                    russh::server::Auth::Accept
                } else {
                    russh::server::Auth::reject()
                },
            )
        }

        async fn channel_open_session(
            &mut self,
            channel: russh::Channel<russh::server::Msg>,
            reply: russh::server::ChannelOpenHandle,
            _: &mut russh::server::Session,
        ) -> Result<(), Self::Error> {
            reply.accept().await;
            self.channels.push(channel);
            Ok(())
        }

        async fn exec_request(
            &mut self,
            channel: russh::ChannelId,
            command: &[u8],
            session: &mut russh::server::Session,
        ) -> Result<(), Self::Error> {
            session.channel_success(channel)?;
            session.data(
                channel,
                format!("ran: {}", String::from_utf8_lossy(command)),
            )?;
            session.exit_status_request(channel, 3)?;
            session.eof(channel)?;
            session.close(channel)?;
            Ok(())
        }
    }

    /// Serves one connection on a local port, with `host_private` as its
    /// host key.
    async fn serve_once(host_private: &str, user_key: PublicKey) -> u16 {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let config = Arc::new(russh::server::Config {
            keys: vec![decode_secret_key(host_private, None).unwrap()],
            auth_rejection_time: Duration::from_millis(10),
            ..Default::default()
        });
        tokio::spawn(async move {
            let (socket, _) = listener.accept().await.unwrap();
            let handler = Answering {
                user_key,
                channels: Vec::new(),
            };
            if let Ok(session) = russh::server::run_stream(config, socket, handler).await {
                let _ = session.await;
            }
        });
        port
    }

    fn made() -> (hoista_app::servers::ServerSetup, ServerAccess) {
        let setup = hoista_app::servers::ServerSetup::generate("web-1").unwrap();
        let access = ServerAccess::from_kept(&setup.kept()).unwrap();
        (setup, access)
    }

    fn run(command: &str) -> CommandRun<'_> {
        CommandRun {
            command,
            input: None,
            timeout: Duration::from_secs(10),
            max_output: 1024,
        }
    }

    #[tokio::test]
    async fn a_command_runs_on_the_server_it_was_made_for() {
        let (setup, access) = made();
        let user_key = PublicKey::from_openssh(&setup.authorized_key).unwrap();
        let port = serve_once(setup.host_private_key.expose(), user_key).await;
        let done = SshServers
            .run(
                ServerAddress {
                    host: "127.0.0.1",
                    port,
                },
                &access,
                run("uptime"),
            )
            .await
            .unwrap();
        assert_eq!(done.stdout, b"ran: uptime");
        assert_eq!(done.exit_code, Some(3));
        assert!(!done.timed_out);
    }

    #[tokio::test]
    async fn a_server_with_another_host_key_is_never_reached() {
        let (setup, access) = made();
        let (other, _) = made();
        let user_key = PublicKey::from_openssh(&setup.authorized_key).unwrap();
        // The server answers with a host key Hoista didn't keep.
        let port = serve_once(other.host_private_key.expose(), user_key).await;
        let refused = SshServers
            .run(
                ServerAddress {
                    host: "127.0.0.1",
                    port,
                },
                &access,
                run("uptime"),
            )
            .await
            .unwrap_err();
        assert!(
            matches!(&refused, EngineError::Provider { code, .. } if code == "server.host_key_changed"),
            "{refused:?}"
        );
    }

    #[tokio::test]
    async fn a_refused_key_says_so() {
        let (setup, access) = made();
        let (other, _) = made();
        // The server only takes another key.
        let user_key = PublicKey::from_openssh(&other.authorized_key).unwrap();
        let port = serve_once(setup.host_private_key.expose(), user_key).await;
        let refused = SshServers
            .run(
                ServerAddress {
                    host: "127.0.0.1",
                    port,
                },
                &access,
                run("uptime"),
            )
            .await
            .unwrap_err();
        assert!(
            matches!(&refused, EngineError::Provider { code, .. } if code == "server.sign_in_refused"),
            "{refused:?}"
        );
    }
}
