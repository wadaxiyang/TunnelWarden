use std::{net::SocketAddr, sync::Arc, time::Duration};

use russh::{
    Channel, ChannelId,
    keys::{Algorithm, PrivateKey},
    server,
};
use ssh_engine::{HopSpec, HostKeyApproval, HostKeyDecision, SshChain, SshCredential};
use tempfile::TempDir;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::{Mutex, mpsc},
    task::JoinHandle,
    time::timeout,
};
use tokio_util::sync::CancellationToken;
use tunnel_domain::{AuthConfig, HostId, HostKeyPolicy, SecretRef, SshHost};
use zeroize::Zeroizing;

struct TargetServer;

impl server::Handler for TargetServer {
    type Error = russh::Error;

    async fn auth_password(
        &mut self,
        user: &str,
        password: &str,
    ) -> Result<server::Auth, Self::Error> {
        Ok(if user == "alice" && password == "target-secret" {
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

struct BastionServer {
    target: SocketAddr,
    relays: mpsc::Sender<JoinHandle<()>>,
}

impl server::Handler for BastionServer {
    type Error = russh::Error;

    async fn auth_password(
        &mut self,
        user: &str,
        password: &str,
    ) -> Result<server::Auth, Self::Error> {
        Ok(if user == "alice" && password == "bastion-secret" {
            server::Auth::Accept
        } else {
            server::Auth::reject()
        })
    }

    async fn channel_open_direct_tcpip(
        &mut self,
        channel: Channel<server::Msg>,
        destination: &str,
        port: u32,
        _originator: &str,
        _originator_port: u32,
        reply: server::ChannelOpenHandle,
        _session: &mut server::Session,
    ) -> Result<(), Self::Error> {
        if destination != "127.0.0.1" || port != u32::from(self.target.port()) {
            reply
                .reject(russh::ChannelOpenFailure::AdministrativelyProhibited)
                .await;
            return Ok(());
        }
        let target = self.target;
        reply.accept().await;
        let task = tokio::spawn(async move {
            if let Ok(mut stream) = TcpStream::connect(target).await {
                let mut channel = channel.into_stream();
                let _ = tokio::io::copy_bidirectional(&mut channel, &mut stream).await;
            }
        });
        if let Err(error) = self.relays.try_send(task) {
            error.into_inner().abort();
        }
        Ok(())
    }
}

async fn start_server<H: server::Handler<Error = russh::Error> + Send + 'static>(
    handler: H,
) -> (SocketAddr, russh::keys::PublicKey, JoinHandle<()>) {
    let key = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).expect("host key");
    let public_key = key.public_key().clone();
    let mut config = server::Config::default();
    config.keys.push(key);
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("SSH listener");
    let address = listener.local_addr().expect("SSH address");
    let task = tokio::spawn(async move {
        if let Ok((socket, _)) = listener.accept().await
            && let Ok(session) = server::run_stream(Arc::new(config), socket, handler).await
        {
            let _ = session.await;
        }
    });
    (address, public_key, task)
}

fn host(id: &str, address: SocketAddr) -> SshHost {
    SshHost {
        id: HostId(id.into()),
        name: id.into(),
        hostname: "127.0.0.1".into(),
        port: address.port(),
        username: "alice".into(),
        auth: AuthConfig::Password {
            credential_ref: SecretRef(format!("{id}-password")),
        },
        host_key_policy: HostKeyPolicy::Strict,
        connect_timeout: Duration::from_secs(5),
        keepalive_interval: Duration::from_secs(10),
        keepalive_max: 3,
        notes: String::new(),
    }
}

#[tokio::test]
async fn unknown_key_requires_explicit_approval_and_saves_app_known_hosts() {
    let (address, public_key, server) = start_server(TargetServer).await;
    let directory = TempDir::new().expect("temporary directory");
    let known_hosts = directory.path().join("known_hosts");
    let (prompts, mut requests) = mpsc::channel(1);
    let approval = HostKeyApproval::new(
        prompts,
        known_hosts.clone(),
        Arc::new(Mutex::new(())),
        "test-tunnel".into(),
        1,
    );
    let cancellation = CancellationToken::new();
    let task = tokio::spawn(async move {
        SshChain::connect_with_approval(
            vec![HopSpec {
                host: host("target", address),
                credential: SshCredential::Password(Zeroizing::new("target-secret".into())),
                known_hosts_paths: vec![known_hosts],
            }],
            None,
            &cancellation,
            Some(approval),
        )
        .await
    });
    let prompt = timeout(Duration::from_secs(5), requests.recv())
        .await
        .expect("host key prompt timeout")
        .expect("host key prompt");
    assert_eq!(prompt.port, address.port());
    assert_eq!(prompt.tunnel_id, "test-tunnel");
    assert_eq!(prompt.host_id, "target");
    assert_eq!((prompt.generation, prompt.attempt, prompt.hop), (1, 1, 1));
    assert!(prompt.fingerprint.starts_with("SHA256:"));
    assert!(prompt.algorithm.contains("ed25519"));
    prompt
        .reply
        .send(HostKeyDecision::TrustAndSave)
        .expect("send approval");
    let chain = timeout(Duration::from_secs(5), task)
        .await
        .expect("connect timeout")
        .expect("connect task")
        .expect("approved connection");
    let stored =
        std::fs::read_to_string(directory.path().join("known_hosts")).expect("saved known_hosts");
    assert!(stored.contains(&public_key.to_openssh().expect("public key")));
    chain.disconnect().await;
    server.await.expect("server stopped");
}

#[tokio::test]
async fn silent_server_uses_network_timeout_even_when_approval_is_available() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("silent server listener");
    let address = listener.local_addr().expect("silent server address");
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.expect("accept client");
        let mut bytes = [0u8; 256];
        while let Ok(count) = socket.read(&mut bytes).await {
            if count == 0 {
                break;
            }
        }
    });
    let directory = TempDir::new().expect("temporary directory");
    let (prompts, mut requests) = mpsc::channel(1);
    let approval = HostKeyApproval::new(
        prompts,
        directory.path().join("known_hosts"),
        Arc::new(Mutex::new(())),
        "test-tunnel".into(),
        1,
    );
    let mut target = host("silent", address);
    target.connect_timeout = Duration::from_millis(200);
    let cancellation = CancellationToken::new();
    let result = timeout(
        Duration::from_secs(2),
        SshChain::connect_with_approval(
            vec![HopSpec {
                host: target,
                credential: SshCredential::Password(Zeroizing::new("secret".into())),
                known_hosts_paths: Vec::new(),
            }],
            None,
            &cancellation,
            Some(approval),
        ),
    )
    .await
    .expect("network timeout must not become a 120-second prompt wait");
    let error = match result {
        Ok(_) => panic!("silent server cannot authenticate"),
        Err(error) => error,
    };
    assert!(error.to_string().contains("timed out during SSH handshake"));
    assert!(requests.try_recv().is_err());
    timeout(Duration::from_secs(2), server)
        .await
        .expect("server observed disconnected client")
        .expect("server task");
}

#[tokio::test]
async fn second_ssh_handshake_and_forwarding_traverse_first_hop() {
    let (target_address, target_key, target_task) = start_server(TargetServer).await;
    let (relay_tx, mut relay_rx) = mpsc::channel(1);
    let (bastion_address, bastion_key, bastion_task) = start_server(BastionServer {
        target: target_address,
        relays: relay_tx,
    })
    .await;
    let directory = TempDir::new().expect("temporary directory");
    let known_hosts = directory.path().join("known_hosts");
    std::fs::write(
        &known_hosts,
        format!(
            "[127.0.0.1]:{} {}\n[127.0.0.1]:{} {}\n",
            bastion_address.port(),
            bastion_key.to_openssh().expect("bastion public key"),
            target_address.port(),
            target_key.to_openssh().expect("target public key")
        ),
    )
    .expect("known_hosts");
    let cancellation = CancellationToken::new();
    let chain = SshChain::connect(
        vec![
            HopSpec {
                host: host("bastion", bastion_address),
                credential: SshCredential::Password(Zeroizing::new("bastion-secret".into())),
                known_hosts_paths: vec![known_hosts.clone()],
            },
            HopSpec {
                host: host("target", target_address),
                credential: SshCredential::Password(Zeroizing::new("target-secret".into())),
                known_hosts_paths: vec![known_hosts],
            },
        ],
        None,
        &cancellation,
    )
    .await
    .expect("two-hop SSH chain");
    assert!(!chain.is_closed());
    let rtts = chain
        .ping_all(Duration::from_secs(5))
        .await
        .expect("hop pings");
    assert_eq!(rtts.len(), 2);
    let mut forwarded = chain
        .final_session()
        .open_direct_tcpip(
            "echo.internal",
            7000,
            "127.0.0.1:0".parse().expect("originator"),
            Duration::from_secs(5),
        )
        .await
        .expect("final direct-tcpip");
    forwarded.write_all(b"two SSH hops").await.expect("write");
    let mut echoed = [0u8; 12];
    timeout(Duration::from_secs(5), forwarded.read_exact(&mut echoed))
        .await
        .expect("echo deadline")
        .expect("echo read");
    assert_eq!(&echoed, b"two SSH hops");
    drop(forwarded);
    chain.disconnect().await;
    if let Some(relay) = relay_rx.recv().await {
        let _ = timeout(Duration::from_secs(5), relay).await;
    }
    bastion_task.abort();
    let _ = bastion_task.await;
    target_task.abort();
    let _ = target_task.await;
}

#[tokio::test]
async fn third_ssh_handshake_and_forwarding_traverse_both_bastions() {
    let (target_address, target_key, target_task) = start_server(TargetServer).await;
    let (second_relay_tx, mut second_relay_rx) = mpsc::channel(1);
    let (second_address, second_key, second_task) = start_server(BastionServer {
        target: target_address,
        relays: second_relay_tx,
    })
    .await;
    let (first_relay_tx, mut first_relay_rx) = mpsc::channel(1);
    let (first_address, first_key, first_task) = start_server(BastionServer {
        target: second_address,
        relays: first_relay_tx,
    })
    .await;
    let directory = TempDir::new().expect("temporary directory");
    let known_hosts = directory.path().join("known_hosts");
    std::fs::write(
        &known_hosts,
        format!(
            "[127.0.0.1]:{} {}\n[127.0.0.1]:{} {}\n[127.0.0.1]:{} {}\n",
            first_address.port(),
            first_key.to_openssh().expect("first key"),
            second_address.port(),
            second_key.to_openssh().expect("second key"),
            target_address.port(),
            target_key.to_openssh().expect("target key"),
        ),
    )
    .expect("known_hosts");
    let cancellation = CancellationToken::new();
    let chain = SshChain::connect(
        vec![
            HopSpec {
                host: host("first", first_address),
                credential: SshCredential::Password(Zeroizing::new("bastion-secret".into())),
                known_hosts_paths: vec![known_hosts.clone()],
            },
            HopSpec {
                host: host("second", second_address),
                credential: SshCredential::Password(Zeroizing::new("bastion-secret".into())),
                known_hosts_paths: vec![known_hosts.clone()],
            },
            HopSpec {
                host: host("target", target_address),
                credential: SshCredential::Password(Zeroizing::new("target-secret".into())),
                known_hosts_paths: vec![known_hosts],
            },
        ],
        None,
        &cancellation,
    )
    .await
    .expect("three-hop SSH chain");
    assert_eq!(
        chain
            .ping_all(Duration::from_secs(5))
            .await
            .expect("ping each hop")
            .len(),
        3
    );
    let mut forwarded = chain
        .final_session()
        .open_direct_tcpip(
            "echo.internal",
            7000,
            "127.0.0.1:0".parse().expect("originator"),
            Duration::from_secs(5),
        )
        .await
        .expect("final direct-tcpip");
    forwarded.write_all(b"three SSH hops").await.expect("write");
    let mut echoed = [0u8; 14];
    timeout(Duration::from_secs(5), forwarded.read_exact(&mut echoed))
        .await
        .expect("echo deadline")
        .expect("echo read");
    assert_eq!(&echoed, b"three SSH hops");
    drop(forwarded);
    chain.disconnect().await;
    for rx in [&mut first_relay_rx, &mut second_relay_rx] {
        if let Some(relay) = rx.recv().await {
            let _ = timeout(Duration::from_secs(5), relay).await;
        }
    }
    for task in [first_task, second_task, target_task] {
        task.abort();
        let _ = task.await;
    }
}
