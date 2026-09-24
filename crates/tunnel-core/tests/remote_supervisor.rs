use std::{sync::Arc, time::Duration};

use russh::{
    Disconnect,
    keys::{Algorithm, PrivateKey},
    server,
};
use ssh_engine::{DirectSshSession, SshChain};
use tempfile::TempDir;
use tokio::{net::TcpListener, sync::mpsc, time::timeout};
use tokio_util::sync::CancellationToken;
use tunnel_core::{RemoteForwardSupervisor, SupervisorState};
use tunnel_domain::{
    AuthConfig, HostId, HostKeyPolicy, LocalEndpoint, RemoteEndpoint, RetryPolicy, SecretRef,
    SshHost,
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
        if address == "127.0.0.1" && *port == 8123 {
            let _ = self
                .events
                .try_send(ServerEvent::Registered(session.handle()));
            Ok(true)
        } else {
            Ok(false)
        }
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
async fn remote_registration_recovers_after_disconnect_and_stop_cancels_it() {
    let key = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).expect("host key");
    let public_key = key.public_key().clone();
    let mut config = server::Config::default();
    config.keys.push(key);
    let config = Arc::new(config);
    let ssh_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("SSH listener");
    let address = ssh_listener.local_addr().expect("SSH address");
    let (events_tx, mut events_rx) = mpsc::channel(8);
    let server_task = tokio::spawn(async move {
        let mut sessions = tokio::task::JoinSet::new();
        for _ in 0..2 {
            let (socket, _) = ssh_listener.accept().await.expect("SSH accept");
            let events = events_tx.clone();
            let config = Arc::clone(&config);
            sessions.spawn(async move {
                if let Ok(session) =
                    server::run_stream(config, socket, RemoteServer { events }).await
                {
                    let _ = session.await;
                }
            });
        }
        while sessions.join_next().await.is_some() {}
    });
    let directory = TempDir::new().expect("temporary directory");
    let known_hosts = directory.path().join("known_hosts");
    std::fs::write(
        &known_hosts,
        format!(
            "[127.0.0.1]:{} {}\n",
            address.port(),
            public_key.to_openssh().expect("public key")
        ),
    )
    .expect("known_hosts");
    let host = SshHost {
        id: HostId("remote-supervisor-test".into()),
        name: "remote-supervisor-test".into(),
        hostname: "127.0.0.1".into(),
        port: address.port(),
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
    let cancellation = CancellationToken::new();
    let supervisor = RemoteForwardSupervisor::new(
        LocalEndpoint {
            host: "127.0.0.1".into(),
            port: 9000,
        },
        cancellation.clone(),
        RetryPolicy::default(),
    );
    let mut state = supervisor.subscribe();
    let task = tokio::spawn(async move {
        supervisor
            .run(move |session_token| {
                let host = host.clone();
                let known_hosts = known_hosts.clone();
                async move {
                    DirectSshSession::connect_password_for_remote(
                        &host,
                        Zeroizing::new("secret".into()),
                        &[known_hosts],
                        &session_token,
                        RemoteEndpoint {
                            host: "127.0.0.1".into(),
                            port: 8123,
                        },
                    )
                    .await
                    .map(SshChain::single)
                    .map_err(|source| ssh_engine::SshChainError::Hop { hop: 1, source })
                }
            })
            .await;
    });
    let first = timeout(Duration::from_secs(5), events_rx.recv())
        .await
        .expect("first registration deadline")
        .expect("first registration");
    let ServerEvent::Registered(first) = first else {
        panic!("expected first registration")
    };
    first
        .disconnect(Disconnect::ByApplication, "restart".into(), "".into())
        .await
        .expect("disconnect first session");
    let second = timeout(Duration::from_secs(10), events_rx.recv())
        .await
        .expect("second registration deadline")
        .expect("second registration");
    assert!(matches!(second, ServerEvent::Registered(_)));
    timeout(Duration::from_secs(5), async {
        loop {
            if matches!(
                &*state.borrow(),
                SupervisorState::Healthy {
                    remote_port: Some(8123),
                    ..
                }
            ) {
                break;
            }
            state.changed().await.expect("state sender");
        }
    })
    .await
    .expect("healthy state deadline");
    cancellation.cancel();
    timeout(Duration::from_secs(10), task)
        .await
        .expect("supervisor stop deadline")
        .expect("supervisor join");
    assert!(matches!(
        timeout(Duration::from_secs(5), events_rx.recv())
            .await
            .expect("cancellation deadline"),
        Some(ServerEvent::Cancelled)
    ));
    assert!(matches!(&*state.borrow(), SupervisorState::Stopped));
    server_task.abort();
    let _ = server_task.await;
}
