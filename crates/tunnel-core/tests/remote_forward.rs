use std::{sync::Arc, time::Duration};

use russh::{
    keys::{Algorithm, PrivateKey},
    server,
};
use ssh_engine::DirectSshSession;
use tempfile::TempDir;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::mpsc,
    time::timeout,
};
use tokio_util::sync::CancellationToken;
use tunnel_core::RemoteForwardWorker;
use tunnel_domain::{
    AuthConfig, HostId, HostKeyPolicy, LocalEndpoint, RemoteEndpoint, SecretRef, SshHost,
};
use zeroize::Zeroizing;

enum ServerEvent {
    Registered(server::Handle),
    Cancelled,
}

struct RemoteServer {
    events: mpsc::Sender<ServerEvent>,
}

impl server::Handler for RemoteServer {
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

    async fn tcpip_forward(
        &mut self,
        address: &str,
        port: &mut u32,
        session: &mut server::Session,
    ) -> Result<bool, Self::Error> {
        if address != "127.0.0.1" || *port != 8123 {
            return Ok(false);
        }
        let _ = self
            .events
            .try_send(ServerEvent::Registered(session.handle()));
        Ok(true)
    }

    async fn cancel_tcpip_forward(
        &mut self,
        address: &str,
        port: u32,
        _session: &mut server::Session,
    ) -> Result<bool, Self::Error> {
        if address == "127.0.0.1" && port == 8123 {
            let _ = self.events.try_send(ServerEvent::Cancelled);
            Ok(true)
        } else {
            Ok(false)
        }
    }
}

#[tokio::test]
async fn reverse_channel_reaches_local_target_and_stop_cancels_registration() {
    let echo_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("echo listener");
    let echo_address = echo_listener.local_addr().expect("echo address");
    let echo_task = tokio::spawn(async move {
        let (mut stream, _) = echo_listener.accept().await.expect("echo accept");
        let mut buffer = [0u8; 128];
        loop {
            let count = stream.read(&mut buffer).await.expect("echo read");
            if count == 0 {
                break;
            }
            stream
                .write_all(&buffer[..count])
                .await
                .expect("echo write");
        }
    });

    let key = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).expect("host key");
    let public_key = key.public_key().clone();
    let mut config = server::Config::default();
    config.keys.push(key);
    let ssh_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("SSH listener");
    let ssh_address = ssh_listener.local_addr().expect("SSH address");
    let (events_tx, mut events_rx) = mpsc::channel(4);
    let ssh_task = tokio::spawn(async move {
        if let Ok((socket, _)) = ssh_listener.accept().await
            && let Ok(session) =
                server::run_stream(Arc::new(config), socket, RemoteServer { events: events_tx })
                    .await
        {
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
        id: HostId("remote-test".into()),
        name: "remote-test".into(),
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
        inactivity_timeout: None,
        notes: String::new(),
    };
    let root = CancellationToken::new();
    let session = DirectSshSession::connect_password_for_remote(
        &host,
        Zeroizing::new("secret".into()),
        &[known_hosts],
        &root,
        RemoteEndpoint {
            host: "127.0.0.1".into(),
            port: 8123,
        },
    )
    .await
    .expect("SSH connection");
    let worker_cancel = CancellationToken::new();
    let mut worker = RemoteForwardWorker::start(
        session,
        LocalEndpoint {
            host: "127.0.0.1".into(),
            port: echo_address.port(),
        },
        worker_cancel.clone(),
    )
    .await
    .expect("remote registration");
    assert_eq!(worker.bound_port(), 8123);
    let ServerEvent::Registered(handle) = timeout(Duration::from_secs(5), events_rx.recv())
        .await
        .expect("registration deadline")
        .expect("registration event")
    else {
        panic!("expected registration");
    };
    let worker_task = tokio::spawn(async move {
        let run = worker.run().await;
        let stopped = worker.stop().await;
        (run, stopped)
    });
    let wrong_port = handle
        .channel_open_forwarded_tcpip("127.0.0.1", 8124, "192.0.2.1", 12345)
        .await;
    assert!(wrong_port.is_err(), "unregistered remote port was accepted");
    let mut remote = handle
        .channel_open_forwarded_tcpip("127.0.0.1", 8123, "192.0.2.1", 12345)
        .await
        .expect("forwarded-tcpip channel")
        .into_stream();
    remote.write_all(b"reverse over SSH").await.expect("write");
    let mut echoed = [0u8; 16];
    timeout(Duration::from_secs(5), remote.read_exact(&mut echoed))
        .await
        .expect("echo deadline")
        .expect("echo read");
    assert_eq!(&echoed, b"reverse over SSH");
    drop(remote);
    // Stop owns the cancellation sequence; worker cancellation is separate
    // from SSH transport cancellation so cancel-tcpip-forward can complete.
    worker_cancel.cancel();
    let (run, stopped) = timeout(Duration::from_secs(5), worker_task)
        .await
        .expect("worker deadline")
        .expect("worker join");
    assert!(run.is_ok(), "{run:?}");
    assert!(stopped.is_ok(), "{stopped:?}");
    assert!(matches!(
        events_rx.recv().await,
        Some(ServerEvent::Cancelled)
    ));
    echo_task.abort();
    let _ = echo_task.await;
    ssh_task.abort();
    let _ = ssh_task.await;
}
