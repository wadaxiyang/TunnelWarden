use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener as StdTcpListener},
    sync::Arc,
    time::Duration,
};

use russh::{
    Channel, ChannelId, Disconnect,
    keys::{Algorithm, PrivateKey},
    server,
};
use ssh_engine::{HopSpec, SshChain, SshChainError, SshCredential};
use tempfile::TempDir;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::oneshot,
    time::timeout,
};
use tokio_util::sync::CancellationToken;
use tunnel_core::{ListenerRuntime, LocalForwardSupervisor, SupervisorMode, SupervisorState};
use tunnel_domain::{
    AuthConfig, HostId, HostKeyPolicy, RemoteEndpoint, RetryPolicy, SecretRef, SshHost,
};
use zeroize::Zeroizing;

struct EchoServer;

impl server::Handler for EchoServer {
    type Error = russh::Error;

    async fn auth_password(
        &mut self,
        user: &str,
        password: &str,
    ) -> Result<server::Auth, Self::Error> {
        Ok(if user == "alice" && password == "secret" {
            server::Auth::Accept
        } else {
            server::Auth::reject()
        })
    }

    async fn channel_open_direct_tcpip(
        &mut self,
        _channel: Channel<server::Msg>,
        _destination: &str,
        _port: u32,
        _originator: &str,
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

#[tokio::test]
async fn reconnect_keeps_listener_and_restores_forwarded_traffic() {
    let key = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).expect("host key");
    let public_key = key.public_key().clone();
    let mut config = server::Config::default();
    config.keys.push(key);
    let config = Arc::new(config);
    let ssh_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("SSH listener");
    let ssh_address = ssh_listener.local_addr().expect("SSH address");
    let (disconnect_first, drop_first) = oneshot::channel::<()>();
    let server_task = tokio::spawn(async move {
        let mut drop_first = Some(drop_first);
        for index in 0..2 {
            let (socket, _) = ssh_listener.accept().await.expect("SSH accept");
            let session = server::run_stream(Arc::clone(&config), socket, EchoServer)
                .await
                .expect("SSH session");
            if index == 0 {
                if let Some(signal) = drop_first.take() {
                    let _ = signal.await;
                }
                let _ = session
                    .handle()
                    .disconnect(
                        Disconnect::ByApplication,
                        "test disconnect".into(),
                        String::new(),
                    )
                    .await;
            }
            let _ = session.await;
        }
    });
    let directory = TempDir::new().expect("temporary directory");
    let known_hosts = directory.path().join("known_hosts");
    std::fs::write(
        &known_hosts,
        format!(
            "[127.0.0.1]:{} {}\n",
            ssh_address.port(),
            public_key.to_openssh().expect("public key")
        ),
    )
    .expect("known_hosts");
    let host = SshHost {
        id: HostId("echo".into()),
        name: "echo".into(),
        hostname: "127.0.0.1".into(),
        port: ssh_address.port(),
        username: "alice".into(),
        auth: AuthConfig::Password {
            credential_ref: SecretRef("test".into()),
        },
        host_key_policy: HostKeyPolicy::Strict,
        connect_timeout: Duration::from_secs(5),
        keepalive_interval: Duration::from_secs(10),
        keepalive_max: 3,
        notes: String::new(),
    };
    let mut listener = ListenerRuntime::new();
    listener
        .start_listener(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0))
        .await
        .expect("local listener");
    let root = CancellationToken::new();
    let supervisor = LocalForwardSupervisor::new(
        listener,
        SupervisorMode::Local(RemoteEndpoint {
            host: "echo.internal".into(),
            port: 7000,
        }),
        root.clone(),
        RetryPolicy::default(),
    )
    .expect("supervisor");
    let local_address = supervisor
        .local_addr()
        .expect("local address")
        .expect("bound");
    let mut state = supervisor.subscribe();
    let connector_token = root.clone();
    let supervisor_task = tokio::spawn(async move {
        supervisor
            .run(move || {
                let host = host.clone();
                let known_hosts = known_hosts.clone();
                let token = connector_token.clone();
                async move {
                    SshChain::connect(
                        vec![HopSpec {
                            host,
                            credential: SshCredential::Password(Zeroizing::new("secret".into())),
                            known_hosts_paths: vec![known_hosts],
                        }],
                        None,
                        &token,
                    )
                    .await
                }
            })
            .await
    });
    wait_for_healthy(&mut state).await;
    assert_echo(local_address, b"before reconnect").await;
    disconnect_first.send(()).expect("disconnect signal");
    wait_for_reconnecting(&mut state).await;
    assert!(
        StdTcpListener::bind(local_address).is_err(),
        "listener was released during reconnect"
    );
    wait_for_healthy(&mut state).await;
    assert_echo(local_address, b"after reconnect").await;
    root.cancel();
    timeout(Duration::from_secs(10), supervisor_task)
        .await
        .expect("supervisor stop deadline")
        .expect("supervisor join")
        .expect("supervisor stop");
    assert!(
        StdTcpListener::bind(local_address).is_ok(),
        "Stop retained listener"
    );
    server_task.abort();
    let _ = server_task.await;
}

#[tokio::test]
async fn unhealthy_socks_client_receives_failure_and_stop_releases_port() {
    let mut listener = ListenerRuntime::new();
    listener
        .start_listener(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0))
        .await
        .expect("SOCKS listener");
    let root = CancellationToken::new();
    let supervisor = LocalForwardSupervisor::new(
        listener,
        SupervisorMode::Dynamic,
        root.clone(),
        RetryPolicy::default(),
    )
    .expect("dynamic supervisor");
    let address = supervisor.local_addr().expect("address").expect("bound");
    let task = tokio::spawn(async move {
        supervisor
            .run(std::future::pending::<Result<SshChain, SshChainError>>)
            .await
    });
    let mut client = TcpStream::connect(address).await.expect("SOCKS connect");
    client.write_all(&[5, 1, 0]).await.expect("greeting");
    let mut selected = [0u8; 2];
    timeout(Duration::from_secs(5), client.read_exact(&mut selected))
        .await
        .expect("method deadline")
        .expect("method read");
    assert_eq!(selected, [5, 0]);
    client
        .write_all(&[5, 1, 0, 1, 127, 0, 0, 1, 0, 80])
        .await
        .expect("CONNECT request");
    let mut reply = [0u8; 10];
    timeout(Duration::from_secs(5), client.read_exact(&mut reply))
        .await
        .expect("failure deadline")
        .expect("failure read");
    assert_eq!(reply[1], 4, "expected SOCKS host unreachable");
    root.cancel();
    timeout(Duration::from_secs(5), task)
        .await
        .expect("stop deadline")
        .expect("supervisor join")
        .expect("supervisor stop");
    assert!(StdTcpListener::bind(address).is_ok());
}

async fn wait_for_healthy(state: &mut tokio::sync::watch::Receiver<SupervisorState>) {
    timeout(Duration::from_secs(10), async {
        loop {
            if matches!(&*state.borrow(), SupervisorState::Healthy { .. }) {
                break;
            }
            state.changed().await.expect("status sender");
        }
    })
    .await
    .expect("healthy deadline");
}

async fn wait_for_reconnecting(state: &mut tokio::sync::watch::Receiver<SupervisorState>) {
    timeout(Duration::from_secs(10), async {
        loop {
            if matches!(&*state.borrow(), SupervisorState::Reconnecting { .. }) {
                break;
            }
            state.changed().await.expect("status sender");
        }
    })
    .await
    .expect("reconnect deadline");
}

async fn assert_echo(address: SocketAddr, payload: &[u8]) {
    let mut socket = TcpStream::connect(address).await.expect("local connect");
    socket.write_all(payload).await.expect("local write");
    let mut echoed = vec![0u8; payload.len()];
    timeout(Duration::from_secs(5), socket.read_exact(&mut echoed))
        .await
        .expect("echo deadline")
        .expect("echo read");
    assert_eq!(echoed, payload);
}
