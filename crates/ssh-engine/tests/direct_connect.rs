use std::{net::SocketAddr, path::Path, sync::Arc, time::Duration};

use russh::{
    Channel, ChannelId,
    keys::{Algorithm, PrivateKey},
    server,
};
use ssh_engine::{
    DirectSshSession, HopSpec, KeyboardInteractiveApproval, SshChain, SshConnectError,
    SshCredential,
};
use tempfile::TempDir;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::mpsc,
    task::JoinHandle,
    time::timeout,
};
use tokio_util::sync::CancellationToken;
use tunnel_domain::{AuthConfig, HostId, HostKeyPolicy, SecretRef, SshHost, TunnelErrorKind};
use zeroize::Zeroizing;

#[cfg(windows)]
use futures::stream;
#[cfg(windows)]
use russh::keys::agent::{client::AgentClient, server as agent_server};
#[cfg(windows)]
use tokio::net::windows::named_pipe::ServerOptions;

struct PasswordServer {
    authorized_key: Option<russh::keys::PublicKey>,
}

#[derive(Default)]
struct InteractiveServer {
    round: usize,
}

impl server::Handler for InteractiveServer {
    type Error = russh::Error;

    async fn auth_keyboard_interactive<'a>(
        &'a mut self,
        user: &str,
        _submethods: &str,
        response: Option<server::Response<'a>>,
    ) -> Result<server::Auth, Self::Error> {
        if user != "alice" {
            return Ok(server::Auth::reject());
        }
        match (self.round, response) {
            (0, None) => Ok(server::Auth::Partial {
                name: "Login\u{1b}".into(),
                instructions: "Enter password".into(),
                prompts: vec![("Password: ".into(), false)].into(),
            }),
            (0, Some(mut response)) => {
                if response.next().as_deref() != Some(b"secret") {
                    return Ok(server::Auth::reject());
                }
                self.round = 1;
                Ok(server::Auth::Partial {
                    name: "Second factor".into(),
                    instructions: "Enter code".into(),
                    prompts: vec![("Code: ".into(), true)].into(),
                })
            }
            (1, Some(mut response)) => {
                if response.next().as_deref() == Some(b"123456") {
                    Ok(server::Auth::Accept)
                } else {
                    Ok(server::Auth::reject())
                }
            }
            _ => Ok(server::Auth::reject()),
        }
    }
}

impl server::Handler for PasswordServer {
    type Error = russh::Error;

    async fn auth_publickey(
        &mut self,
        username: &str,
        public_key: &russh::keys::PublicKey,
    ) -> Result<server::Auth, Self::Error> {
        if username == "alice" && self.authorized_key.as_ref() == Some(public_key) {
            Ok(server::Auth::Accept)
        } else {
            Ok(server::Auth::reject())
        }
    }

    async fn auth_password(
        &mut self,
        username: &str,
        password: &str,
    ) -> Result<server::Auth, Self::Error> {
        if username == "alice" && password == "secret" {
            Ok(server::Auth::Accept)
        } else {
            Ok(server::Auth::reject())
        }
    }

    async fn channel_open_direct_tcpip(
        &mut self,
        _channel: Channel<server::Msg>,
        _host_to_connect: &str,
        _port_to_connect: u32,
        _originator_address: &str,
        _originator_port: u32,
        reply: server::ChannelOpenHandle,
        _session: &mut server::Session,
    ) -> Result<(), Self::Error> {
        reply.accept().await;
        Ok(())
    }

    async fn data(
        &mut self,
        channel: ChannelId,
        data: &[u8],
        session: &mut server::Session,
    ) -> Result<(), Self::Error> {
        session.data(channel, data.to_vec())?;
        Ok(())
    }
}

struct TestServer {
    address: SocketAddr,
    public_key: russh::keys::PublicKey,
    task: JoinHandle<()>,
}

impl TestServer {
    async fn start() -> Self {
        Self::start_with_authorized_key(None).await
    }

    async fn start_with_authorized_key(authorized_key: Option<russh::keys::PublicKey>) -> Self {
        let key =
            PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).expect("generate host key");
        let public_key = key.public_key().clone();
        let mut config = server::Config::default();
        config.keys.push(key);
        config.auth_rejection_time = Duration::from_millis(1);
        config.auth_rejection_time_initial = Some(Duration::from_millis(1));
        let config = Arc::new(config);
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("test listener");
        let address = listener.local_addr().expect("test address");
        let task = tokio::spawn(async move {
            if let Ok((socket, _)) = listener.accept().await
                && let Ok(session) =
                    server::run_stream(config, socket, PasswordServer { authorized_key }).await
            {
                let _ = session.await;
            }
        });
        Self {
            address,
            public_key,
            task,
        }
    }

    async fn start_interactive() -> Self {
        let key = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).expect("host key");
        let public_key = key.public_key().clone();
        let mut config = server::Config::default();
        config.keys.push(key);
        config.auth_rejection_time = Duration::from_millis(1);
        config.auth_rejection_time_initial = Some(Duration::from_millis(1));
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
        let address = listener.local_addr().expect("address");
        let task = tokio::spawn(async move {
            if let Ok((socket, _)) = listener.accept().await
                && let Ok(session) =
                    server::run_stream(Arc::new(config), socket, InteractiveServer::default()).await
            {
                let _ = session.await;
            }
        });
        Self {
            address,
            public_key,
            task,
        }
    }

    fn host(&self) -> SshHost {
        SshHost {
            id: HostId("test".into()),
            name: "test".into(),
            hostname: "127.0.0.1".into(),
            port: self.address.port(),
            username: "alice".into(),
            auth: AuthConfig::Password {
                credential_ref: SecretRef("test-secret".into()),
            },
            host_key_policy: HostKeyPolicy::Strict,
            connect_timeout: Duration::from_secs(5),
            keepalive_interval: Duration::from_secs(10),
            keepalive_max: 3,
            notes: String::new(),
        }
    }

    fn trust(&self, directory: &Path, key: &russh::keys::PublicKey) -> std::path::PathBuf {
        let path = directory.join("known_hosts");
        let line = format!(
            "[127.0.0.1]:{} {}\n",
            self.address.port(),
            key.to_openssh().expect("format key")
        );
        std::fs::write(&path, line).expect("write known_hosts");
        path
    }

    async fn stop(self) {
        self.task.abort();
        let _ = self.task.await;
    }
}

#[tokio::test]
async fn keyboard_interactive_routes_two_rounds_and_preserves_echo() {
    let server = TestServer::start_interactive().await;
    let directory = TempDir::new().expect("directory");
    let mut host = server.host();
    host.auth = AuthConfig::KeyboardInteractive;
    let path = server.trust(directory.path(), &server.public_key);
    let (tx, mut rx) = mpsc::channel(2);
    let provider = KeyboardInteractiveApproval::new(tx, "tunnel-one".into());
    let cancellation = CancellationToken::new();
    let task = tokio::spawn(async move {
        SshChain::connect_with_prompts(
            vec![HopSpec {
                host,
                credential: SshCredential::KeyboardInteractive,
                known_hosts_paths: vec![path],
            }],
            None,
            &cancellation,
            None,
            Some(provider),
        )
        .await
    });
    let first = timeout(Duration::from_secs(5), rx.recv())
        .await
        .expect("first prompt")
        .expect("prompt");
    assert_eq!(
        (first.tunnel_id.as_str(), first.hop, first.round),
        ("tunnel-one", 1, 1)
    );
    assert_eq!(first.name, "Login�");
    assert!(!first.questions[0].echo);
    first
        .reply
        .send(Some(Zeroizing::new(vec!["secret".into()])))
        .expect("answer");
    let second = timeout(Duration::from_secs(5), rx.recv())
        .await
        .expect("second prompt")
        .expect("prompt");
    assert_eq!(second.attempt, 1);
    assert_eq!(second.round, 2);
    assert!(second.questions[0].echo);
    second
        .reply
        .send(Some(Zeroizing::new(vec!["123456".into()])))
        .expect("answer");
    let chain = timeout(Duration::from_secs(5), task)
        .await
        .expect("connection")
        .expect("join")
        .expect("authenticated");
    assert!(!chain.is_closed());
    chain.disconnect().await;
    server.stop().await;
}

#[tokio::test]
async fn keyboard_interactive_cancel_invalidates_pending_answer() {
    let server = TestServer::start_interactive().await;
    let directory = TempDir::new().expect("directory");
    let mut host = server.host();
    host.auth = AuthConfig::KeyboardInteractive;
    let path = server.trust(directory.path(), &server.public_key);
    let (tx, mut rx) = mpsc::channel(1);
    let provider = KeyboardInteractiveApproval::new(tx, "tunnel-cancel".into());
    let cancellation = CancellationToken::new();
    let attempt_cancel = cancellation.clone();
    let task = tokio::spawn(async move {
        SshChain::connect_with_prompts(
            vec![HopSpec {
                host,
                credential: SshCredential::KeyboardInteractive,
                known_hosts_paths: vec![path],
            }],
            None,
            &attempt_cancel,
            None,
            Some(provider),
        )
        .await
    });
    let prompt = timeout(Duration::from_secs(5), rx.recv())
        .await
        .expect("prompt deadline")
        .expect("prompt");
    cancellation.cancel();
    let result = timeout(Duration::from_secs(5), task)
        .await
        .expect("cancel deadline")
        .expect("join");
    assert!(result.is_err());
    assert!(prompt.reply.is_closed());
    server.stop().await;
}

#[tokio::test]
async fn handshake_timeout_closes_transport_after_russh_task_starts() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("fake server listener");
    let address = listener.local_addr().expect("fake server address");
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.expect("fake server accept");
        socket
            .write_all(b"SSH-2.0-TestServer\r\n")
            .await
            .expect("fake server banner");
        let mut buffer = [0u8; 1024];
        loop {
            match socket.read(&mut buffer).await {
                Ok(0) => break,
                Ok(_) => {}
                Err(_) => break,
            }
        }
    });
    let host = SshHost {
        id: HostId("fake".into()),
        name: "fake".into(),
        hostname: "127.0.0.1".into(),
        port: address.port(),
        username: "alice".into(),
        auth: AuthConfig::Password {
            credential_ref: SecretRef("test-secret".into()),
        },
        host_key_policy: HostKeyPolicy::Strict,
        connect_timeout: Duration::from_millis(200),
        keepalive_interval: Duration::from_secs(10),
        keepalive_max: 3,
        notes: String::new(),
    };
    let cancellation = CancellationToken::new();
    let result = DirectSshSession::connect_password(
        &host,
        Zeroizing::new("secret".into()),
        &[],
        &cancellation,
    )
    .await;
    assert!(matches!(
        result,
        Err(SshConnectError::Timeout("SSH handshake"))
    ));
    assert!(matches!(
        timeout(Duration::from_secs(3), server).await,
        Ok(Ok(()))
    ));
}

#[tokio::test]
async fn matching_host_key_and_password_establish_real_ssh_and_ping() {
    let server = TestServer::start().await;
    let directory = TempDir::new().expect("temporary directory");
    let path = server.trust(directory.path(), &server.public_key);
    let cancellation = CancellationToken::new();
    let session = DirectSshSession::connect_password(
        &server.host(),
        Zeroizing::new("secret".into()),
        &[path],
        &cancellation,
    )
    .await
    .expect("SSH connect and authenticate");
    assert!(!session.is_closed());
    assert!(session.ping(Duration::from_secs(5)).await.is_ok());
    let disconnected = session.disconnect().await;
    assert!(disconnected.is_ok(), "{disconnected:?}");
    server.stop().await;
}

#[tokio::test]
async fn direct_tcpip_channel_carries_bytes_over_authenticated_session() {
    let server = TestServer::start().await;
    let directory = TempDir::new().expect("temporary directory");
    let path = server.trust(directory.path(), &server.public_key);
    let cancellation = CancellationToken::new();
    let session = DirectSshSession::connect_password(
        &server.host(),
        Zeroizing::new("secret".into()),
        &[path],
        &cancellation,
    )
    .await
    .expect("SSH connect and authenticate");
    let mut channel = session
        .open_direct_tcpip(
            "echo.internal",
            7000,
            "127.0.0.1:54321".parse().expect("origin address"),
            Duration::from_secs(5),
        )
        .await
        .expect("open direct-tcpip channel");
    channel
        .write_all(b"through ssh")
        .await
        .expect("channel write");
    let mut echoed = [0u8; 11];
    channel.read_exact(&mut echoed).await.expect("channel read");
    assert_eq!(&echoed, b"through ssh");
    drop(channel);
    assert!(session.disconnect().await.is_ok());
    server.stop().await;
}

#[tokio::test]
async fn unknown_host_key_is_rejected_before_authentication() {
    let server = TestServer::start().await;
    let cancellation = CancellationToken::new();
    let result = DirectSshSession::connect_password(
        &server.host(),
        Zeroizing::new("secret".into()),
        &[],
        &cancellation,
    )
    .await;
    assert!(matches!(&result, Err(SshConnectError::Handshake(_))));
    if let Err(error) = result {
        assert_eq!(error.summary().kind, TunnelErrorKind::HostKeyUnknown);
    }
    server.stop().await;
}

#[tokio::test]
async fn changed_host_key_is_rejected() {
    let server = TestServer::start().await;
    let directory = TempDir::new().expect("temporary directory");
    let stale = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).expect("stale key");
    let path = server.trust(directory.path(), stale.public_key());
    let cancellation = CancellationToken::new();
    let result = DirectSshSession::connect_password(
        &server.host(),
        Zeroizing::new("secret".into()),
        &[path],
        &cancellation,
    )
    .await;
    assert!(matches!(&result, Err(SshConnectError::Handshake(_))));
    if let Err(error) = result {
        assert_eq!(error.summary().kind, TunnelErrorKind::HostKeyMismatch);
    }
    server.stop().await;
}

#[tokio::test]
async fn wrong_password_does_not_create_authenticated_session() {
    let server = TestServer::start().await;
    let directory = TempDir::new().expect("temporary directory");
    let path = server.trust(directory.path(), &server.public_key);
    let cancellation = CancellationToken::new();
    let result = DirectSshSession::connect_password(
        &server.host(),
        Zeroizing::new("wrong".into()),
        &[path],
        &cancellation,
    )
    .await;
    assert!(matches!(
        result,
        Err(SshConnectError::AuthenticationRejected)
    ));
    server.stop().await;
}

#[tokio::test]
async fn private_key_authentication_opens_real_ssh_channel() {
    let identity =
        PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).expect("generate client identity");
    let server = TestServer::start_with_authorized_key(Some(identity.public_key().clone())).await;
    let directory = TempDir::new().expect("temporary directory");
    let known_hosts = server.trust(directory.path(), &server.public_key);
    let key_path = directory.path().join("id_ed25519");
    std::fs::write(
        &key_path,
        identity
            .to_openssh(russh::keys::ssh_key::LineEnding::LF)
            .expect("serialize identity")
            .as_bytes(),
    )
    .expect("write identity");
    let mut host = server.host();
    host.auth = AuthConfig::PrivateKey {
        key_path,
        passphrase_ref: None,
    };
    let cancellation = CancellationToken::new();
    let session = DirectSshSession::connect_private_key(&host, None, &[known_hosts], &cancellation)
        .await
        .expect("private key authentication");
    session.ping(Duration::from_secs(5)).await.expect("ping");
    let mut channel = session
        .open_direct_tcpip(
            "echo.internal",
            7000,
            "127.0.0.1:54321".parse().expect("origin"),
            Duration::from_secs(5),
        )
        .await
        .expect("channel");
    channel.write_all(b"key auth").await.expect("write");
    let mut echo = [0u8; 8];
    channel.read_exact(&mut echo).await.expect("read");
    assert_eq!(&echo, b"key auth");
    drop(channel);
    session.disconnect().await.expect("disconnect");
    server.stop().await;
}

#[cfg(windows)]
#[tokio::test]
async fn named_pipe_agent_authenticates_and_opens_channel() {
    let identity =
        PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).expect("generate client identity");
    let server = TestServer::start_with_authorized_key(Some(identity.public_key().clone())).await;
    let directory = TempDir::new().expect("temporary directory");
    let known_hosts = server.trust(directory.path(), &server.public_key);
    let pipe = format!(
        r"\\.\pipe\TunnelWarden-Agent-Test-{}-{}",
        std::process::id(),
        rand::random::<u64>()
    );
    let first = ServerOptions::new()
        .create(&pipe)
        .expect("create agent pipe");
    let listener = stream::unfold((pipe.clone(), Some(first)), |(path, first)| async move {
        let pipe = match first {
            Some(pipe) => pipe,
            None => ServerOptions::new().create(&path).ok()?,
        };
        pipe.connect().await.ok()?;
        Some((Ok::<_, std::io::Error>(pipe), (path, None)))
    });
    let agent_task = tokio::spawn(async move {
        let _ = agent_server::serve(Box::pin(listener), ()).await;
    });
    let mut agent = AgentClient::connect_named_pipe(&pipe)
        .await
        .expect("connect test agent");
    agent
        .add_identity(&identity, &[])
        .await
        .expect("load test identity");
    let mut host = server.host();
    host.auth = AuthConfig::Agent { socket: Some(pipe) };
    let cancellation = CancellationToken::new();
    let session = DirectSshSession::connect_agent(&host, &[known_hosts], &cancellation)
        .await
        .expect("agent authentication");
    session
        .ping(Duration::from_secs(5))
        .await
        .expect("agent session ping");
    let mut channel = session
        .open_direct_tcpip(
            "echo.internal",
            7000,
            "127.0.0.1:54321".parse().expect("origin"),
            Duration::from_secs(5),
        )
        .await
        .expect("agent channel");
    channel.write_all(b"agent auth").await.expect("write");
    let mut echo = [0u8; 10];
    channel.read_exact(&mut echo).await.expect("read");
    assert_eq!(&echo, b"agent auth");
    drop(channel);
    session.disconnect().await.expect("disconnect");
    drop(agent);
    agent_task.abort();
    let _ = agent_task.await;
    server.stop().await;
}

#[tokio::test]
async fn encrypted_private_key_accepts_passphrase() {
    let identity =
        PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).expect("generate client identity");
    let server = TestServer::start_with_authorized_key(Some(identity.public_key().clone())).await;
    let directory = TempDir::new().expect("temporary directory");
    let known_hosts = server.trust(directory.path(), &server.public_key);
    let key_path = directory.path().join("encrypted_id_ed25519");
    let encrypted = identity
        .encrypt(&mut rand::rng(), "passphrase")
        .expect("encrypt identity");
    std::fs::write(
        &key_path,
        encrypted
            .to_openssh(russh::keys::ssh_key::LineEnding::LF)
            .expect("serialize identity")
            .as_bytes(),
    )
    .expect("write identity");
    let mut host = server.host();
    host.auth = AuthConfig::PrivateKey {
        key_path,
        passphrase_ref: Some(SecretRef("test-passphrase".into())),
    };
    let cancellation = CancellationToken::new();
    let session = DirectSshSession::connect_private_key(
        &host,
        Some(Zeroizing::new("passphrase".into())),
        &[known_hosts],
        &cancellation,
    )
    .await
    .expect("encrypted private key authentication");
    session.ping(Duration::from_secs(5)).await.expect("ping");
    session.disconnect().await.expect("disconnect");
    server.stop().await;
}

#[tokio::test]
async fn oversized_private_key_is_rejected_before_network_connect() {
    let directory = TempDir::new().expect("temporary directory");
    let key_path = directory.path().join("oversized_key");
    std::fs::write(&key_path, vec![b'x'; 1024 * 1024 + 1]).expect("write oversized key");
    let host = TestServer::start().await;
    let mut config = host.host();
    config.auth = AuthConfig::PrivateKey {
        key_path,
        passphrase_ref: None,
    };
    let cancellation = CancellationToken::new();
    let result = DirectSshSession::connect_private_key(&config, None, &[], &cancellation).await;
    assert!(matches!(result, Err(SshConnectError::PrivateKeyTooLarge)));
    host.stop().await;
}

#[cfg(windows)]
#[tokio::test]
async fn unavailable_agent_reports_authentication_failure() {
    let server = TestServer::start().await;
    let directory = TempDir::new().expect("temporary directory");
    let known_hosts = server.trust(directory.path(), &server.public_key);
    let mut host = server.host();
    host.auth = AuthConfig::Agent {
        socket: Some(r"\\.\pipe\tunnelwarden-missing-test-agent".into()),
    };
    let cancellation = CancellationToken::new();
    let result = DirectSshSession::connect_agent(&host, &[known_hosts], &cancellation).await;
    assert!(matches!(result, Err(SshConnectError::Agent(_))));
    server.stop().await;
}
