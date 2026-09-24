use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener as StdTcpListener},
    sync::Arc,
    time::Duration,
};

use russh::{
    Channel, ChannelId,
    keys::{Algorithm, PrivateKey},
    server,
};
use ssh_engine::DirectSshSession;
use tempfile::TempDir;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    task::JoinHandle,
    time::timeout,
};
use tokio_util::sync::CancellationToken;
use tunnel_core::{ListenerRuntime, LocalForwardWorker};
use tunnel_domain::{AuthConfig, HostId, HostKeyPolicy, RemoteEndpoint, SecretRef, SshHost};
use zeroize::Zeroizing;

struct EchoServer;

impl server::Handler for EchoServer {
    type Error = russh::Error;

    async fn auth_password(
        &mut self,
        user: &str,
        password: &str,
    ) -> Result<server::Auth, Self::Error> {
        if user == "alice" && password == "secret" {
            Ok(server::Auth::Accept)
        } else {
            Ok(server::Auth::reject())
        }
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

async fn start_ssh_server() -> (SocketAddr, russh::keys::PublicKey, JoinHandle<()>) {
    let key = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).expect("host key");
    let public_key = key.public_key().clone();
    let mut config = server::Config::default();
    config.keys.push(key);
    let config = Arc::new(config);
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("SSH listener");
    let address = listener.local_addr().expect("SSH address");
    let task = tokio::spawn(async move {
        if let Ok((socket, _)) = listener.accept().await
            && let Ok(session) = server::run_stream(config, socket, EchoServer).await
        {
            let _ = session.await;
        }
    });
    (address, public_key, task)
}

#[tokio::test]
async fn local_tcp_bytes_cross_ssh_and_stop_releases_port() {
    let (ssh_address, public_key, server_task) = start_ssh_server().await;
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
    let root = CancellationToken::new();
    let mut listener = ListenerRuntime::new();
    let bind = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0);
    assert!(listener.start_listener(bind).await.is_ok());
    let session = DirectSshSession::connect_password(
        &host,
        Zeroizing::new("secret".into()),
        &[known_hosts],
        &root,
    )
    .await
    .expect("SSH connection");
    assert!(session.ping(Duration::from_secs(5)).await.is_ok());
    let mut worker = LocalForwardWorker::new(
        listener,
        session,
        RemoteEndpoint {
            host: "echo.internal".into(),
            port: 7000,
        },
        root.clone(),
    )
    .expect("local worker");
    let local_address = worker
        .local_addr()
        .expect("local address")
        .expect("bound listener");
    let stop = worker.cancellation_token();
    let worker_task = tokio::spawn(async move {
        let run = worker.run().await;
        let counts = (worker.counters().uploaded(), worker.counters().downloaded());
        let stopped = worker.stop().await;
        (run, counts, stopped)
    });

    let mut client = TcpStream::connect(local_address)
        .await
        .expect("connect local port");
    client
        .write_all(b"local through SSH")
        .await
        .expect("local write");
    let mut echoed = [0u8; 17];
    timeout(Duration::from_secs(5), client.read_exact(&mut echoed))
        .await
        .expect("echo deadline")
        .expect("echo data");
    assert_eq!(&echoed, b"local through SSH");
    stop.cancel();
    let (run, counts, stopped) = timeout(Duration::from_secs(5), worker_task)
        .await
        .expect("worker deadline")
        .expect("worker join");
    assert!(run.is_ok(), "{run:?}");
    assert!(stopped.is_ok(), "{stopped:?}");
    assert!(
        counts.0 >= 17 && counts.1 >= 17,
        "traffic counters: {counts:?}"
    );
    assert!(
        StdTcpListener::bind(local_address).is_ok(),
        "Stop retained local port"
    );
    server_task.abort();
    let _ = server_task.await;
}
