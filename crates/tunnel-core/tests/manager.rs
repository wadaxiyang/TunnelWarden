use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener},
    sync::Arc,
    time::Duration,
};

use tokio::time::timeout;
use tunnel_core::{CoreCommand, CredentialSource, SupervisorState, TunnelManager};
use tunnel_domain::{
    AuthConfig, HostId, HostKeyPolicy, LocalEndpoint, RetryPolicy, SecretRef, SshHost,
    TunnelConfig, TunnelId, TunnelMode,
};
use zeroize::Zeroizing;

struct NoSecrets;

impl CredentialSource for NoSecrets {
    fn load(&self, _: &SecretRef) -> Result<Zeroizing<String>, String> {
        Err("No secret configured".into())
    }
}

#[tokio::test]
async fn stop_during_reconnect_releases_the_owned_listener() {
    let reserved = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("reserve port");
    let port = reserved.local_addr().expect("local address").port();
    drop(reserved);
    let id = TunnelId("socks".into());
    let host = SshHost {
        id: HostId("unreachable".into()),
        name: "Unreachable".into(),
        hostname: "127.0.0.1".into(),
        port: 9,
        username: "alice".into(),
        auth: AuthConfig::Agent { socket: None },
        host_key_policy: HostKeyPolicy::Strict,
        connect_timeout: Duration::from_millis(200),
        keepalive_interval: Duration::from_secs(10),
        keepalive_max: 3,
        notes: String::new(),
    };
    let tunnel = TunnelConfig {
        id: id.clone(),
        name: "SOCKS".into(),
        group_id: None,
        mode: TunnelMode::Dynamic,
        jump_chain: vec![host.id.clone()],
        local: LocalEndpoint {
            host: "127.0.0.1".into(),
            port,
        },
        remote: None,
        auto_start: false,
        reconnect: RetryPolicy {
            base_delay: Duration::from_secs(10),
            max_delay: Duration::from_secs(30),
            ..RetryPolicy::default()
        },
        description: String::new(),
    };
    let (manager, handle) =
        TunnelManager::new(vec![host], Vec::new(), vec![tunnel], Arc::new(NoSecrets))
            .expect("manager");
    let task = tokio::spawn(manager.run());
    let mut state = handle.subscribe();
    handle
        .try_send(CoreCommand::StartTunnel(id.clone()))
        .expect("start command");
    timeout(Duration::from_secs(5), async {
        loop {
            if matches!(
                &state.borrow().get(&id).expect("tunnel snapshot").state,
                SupervisorState::Reconnecting { .. }
            ) {
                break;
            }
            state.changed().await.expect("snapshot sender");
        }
    })
    .await
    .expect("reconnecting deadline");
    assert!(TcpListener::bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port)).is_err());
    handle
        .try_send(CoreCommand::NetworkRecovered)
        .expect("network recovery command");
    timeout(Duration::from_secs(3), async {
        loop {
            if matches!(
                &state.borrow().get(&id).expect("tunnel snapshot").state,
                SupervisorState::Reconnecting { attempt: 2, .. }
            ) {
                break;
            }
            state.changed().await.expect("snapshot sender");
        }
    })
    .await
    .expect("early retry deadline");
    assert!(TcpListener::bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port)).is_err());
    handle
        .try_send(CoreCommand::StopTunnel(id.clone()))
        .expect("stop command");
    timeout(Duration::from_secs(5), async {
        loop {
            if matches!(
                &state.borrow().get(&id).expect("tunnel snapshot").state,
                SupervisorState::Stopped
            ) {
                break;
            }
            state.changed().await.expect("snapshot sender");
        }
    })
    .await
    .expect("stopped deadline");
    assert!(TcpListener::bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port)).is_ok());
    drop(handle);
    timeout(Duration::from_secs(5), task)
        .await
        .expect("manager join deadline")
        .expect("manager task");
}
