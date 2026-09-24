use std::{net::SocketAddr, path::Path, sync::Arc, time::Duration};

use russh::{
    Channel, ChannelId,
    keys::{Algorithm, PrivateKey},
    server,
};
use ssh_engine::{DirectSshSession, SshConnectError};
use tempfile::TempDir;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    task::JoinHandle,
    time::timeout,
};
use tokio_util::sync::CancellationToken;
use tunnel_domain::{AuthConfig, HostId, HostKeyPolicy, SecretRef, SshHost, TunnelErrorKind};
use zeroize::Zeroizing;

struct PasswordServer {
    authorized_key: Option<russh::keys::PublicKey>,
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
