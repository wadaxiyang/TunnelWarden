// Copyright (C) 2026 Wilson Glasser and Oryxis contributors
// SPDX-License-Identifier: AGPL-3.0-or-later
// Adapted 2026-09-29 for TunnelWarden: loopback AsyncSSH fixture, all modes
// on one public ForwardConn, deterministic host-key pin, cancellation checks.
// Source: crates/oryxis-ssh/tests/ssh_integration.rs
// Oryxis 16ad2af83b424780ff60b6420a6c20a3b30c7143.
mod common;
#[cfg(windows)]
#[path = "../../../tests/windows_agent.rs"]
mod windows_agent;
use common::*;
use oryxis_core::models::known_host::KnownHost;
use oryxis_core::models::{connection::AuthMethod, port_forward_rule::ForwardKind};
use oryxis_ssh::KeyMaterial;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Duration;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
};
use tunnelwarden_core::{
    credentials::SecretStore,
    dial::{ApplicationDialer, PromptBroker},
    storage::ConfigStore,
};
use tunnelwarden_core::{
    dto::TunnelStatus,
    runtime::{DialFuture, ForwardDialer, TunnelRuntimeManager},
};
use tunnelwarden_core::{
    dto::{HostDto, SaveHost, TunnelDto},
    service::AppService,
};

#[derive(Default)]
struct TestSecrets(std::sync::Mutex<std::collections::HashMap<uuid::Uuid, String>>);
impl SecretStore for TestSecrets {
    fn get(&self, id: uuid::Uuid) -> Result<Option<String>, String> {
        Ok(self.0.lock().unwrap().get(&id).cloned())
    }
    fn set(&self, id: uuid::Uuid, value: &str) -> Result<(), String> {
        self.0.lock().unwrap().insert(id, value.into());
        Ok(())
    }
    fn delete(&self, id: uuid::Uuid) -> Result<(), String> {
        self.0.lock().unwrap().remove(&id);
        Ok(())
    }
}

struct TestDialer {
    connection: oryxis_core::models::connection::Connection,
    engine: oryxis_ssh::SshEngine,
    calls: AtomicUsize,
}
impl ForwardDialer for TestDialer {
    fn connect(&self, _: uuid::Uuid, _: bool) -> DialFuture {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let c = self.connection.clone();
        let engine = self.engine.clone();
        Box::pin(async move {
            tokio::time::sleep(Duration::from_millis(150)).await;
            engine
                .connect_forward_conn(&c, Some(PASSWORD), None, None)
                .await
                .map_err(|e| e.to_string())
        })
    }
}
async fn wait_status(runtime: &TunnelRuntimeManager, id: uuid::Uuid, status: TunnelStatus) {
    let mut updates = runtime.subscribe();
    tokio::time::timeout(Duration::from_secs(12), async {
        loop {
            if updates
                .borrow()
                .tunnels
                .iter()
                .any(|r| r.id == id && r.status == status)
            {
                return;
            }
            updates.changed().await.unwrap();
        }
    })
    .await
    .unwrap_or_else(|_| panic!("wanted {status:?}, got {:?}", runtime.snapshot()));
}

#[tokio::test]
async fn public_api_all_modes_share_a_connection_and_explicitly_cancel() {
    let fixture = Fixture::start();
    let c = fixture.connection();
    let conn = fixture
        .engine()
        .connect_forward_conn(&c, Some(PASSWORD), None, None)
        .await
        .unwrap();
    let mut sessions = Vec::new();
    for kind in ForwardKind::ALL {
        let r = rule(&c, kind).await;
        let session = conn.attach(&r).await.unwrap();
        assert!(session.is_alive());
        let mut stream = TcpStream::connect(("127.0.0.1", r.listen_port))
            .await
            .unwrap();
        if kind == ForwardKind::Dynamic {
            // Upstream test's minimal SOCKS5 CLIENT: the server remains Oryxis.
            stream.write_all(&[5, 1, 0]).await.unwrap();
            let mut method = [0; 2];
            stream.read_exact(&mut method).await.unwrap();
            assert_eq!(method, [5, 0]);
            let [hi, lo] = c.port.to_be_bytes();
            stream
                .write_all(&[5, 1, 0, 1, 127, 0, 0, 1, hi, lo])
                .await
                .unwrap();
            let mut reply = [0; 10];
            stream.read_exact(&mut reply).await.unwrap();
            assert_eq!(reply[1], 0);
        }
        banner(&mut stream).await;
        sessions.push((r, session));
    }
    for (r, session) in sessions {
        session.cancel().await;
        assert!(session.is_cancelled());
        drop(session);
        assert_released(r.listen_port).await;
        // Reattaching -R on the SAME connection proves the server bind was
        // cancelled, rather than merely released by dropping its transport.
        let again = conn.attach(&r).await.unwrap();
        again.cancel().await;
    }
}

#[tokio::test]
async fn payload_counters_are_live_isolated_and_reset_for_all_modes() {
    let fixture = Fixture::start();
    let c = fixture.connection();
    let conn = fixture
        .engine()
        .connect_forward_conn(&c, Some(PASSWORD), None, None)
        .await
        .unwrap();
    let echo = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let target = echo.local_addr().unwrap().port();
    let echo_task = tokio::spawn(async move {
        loop {
            let (mut stream, _) = echo.accept().await.unwrap();
            tokio::spawn(async move {
                let (mut read, mut write) = stream.split();
                let _ = tokio::io::copy(&mut read, &mut write).await;
            });
        }
    });
    let idle_rule = rule(&c, ForwardKind::Local).await;
    let idle = conn.attach(&idle_rule).await.unwrap();
    for kind in ForwardKind::ALL {
        let mut r = rule(&c, kind).await;
        r.target_port = target;
        let session = conn.attach(&r).await.unwrap();
        let mut stream = TcpStream::connect(("127.0.0.1", r.listen_port))
            .await
            .unwrap();
        if kind == ForwardKind::Dynamic {
            stream.write_all(&[5, 1, 0]).await.unwrap();
            let mut method = [0; 2];
            stream.read_exact(&mut method).await.unwrap();
            let [hi, lo] = target.to_be_bytes();
            stream
                .write_all(&[5, 1, 0, 1, 127, 0, 0, 1, hi, lo])
                .await
                .unwrap();
            let mut reply = [0; 10];
            stream.read_exact(&mut reply).await.unwrap();
            assert_eq!(reply[1], 0);
        }
        // Keep the socket open: counters must update before EOF and exclude SOCKS framing.
        let payload = vec![42u8; 128 * 1024];
        stream.write_all(&payload).await.unwrap();
        let mut received = vec![0; payload.len()];
        tokio::time::timeout(Duration::from_secs(5), stream.read_exact(&mut received))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(received, payload);
        assert_eq!(
            session.traffic_bytes(),
            (payload.len() as u64, payload.len() as u64)
        );
        assert_eq!(idle.traffic_bytes(), (0, 0));
        let started = session.traffic_started_at();
        session.cancel().await;
        assert_released(r.listen_port).await;
        let again = conn.attach(&r).await.unwrap();
        assert_eq!(again.traffic_bytes(), (0, 0));
        assert!(again.traffic_started_at() > started);
        again.cancel().await;
    }
    idle.cancel().await;
    echo_task.abort();
}

#[tokio::test]
async fn private_key_auth_uses_upstream_engine() {
    let fixture = Fixture::start();
    let mut c = fixture.connection();
    c.auth_method = AuthMethod::Key;
    let r = rule(&c, ForwardKind::Local).await;
    let session = fixture
        .engine()
        .connect_forward(
            &c,
            None,
            Some(KeyMaterial::plain(&fixture.info.private_key)),
            &r,
            None,
        )
        .await
        .unwrap();
    let mut stream = TcpStream::connect(("127.0.0.1", r.listen_port))
        .await
        .unwrap();
    banner(&mut stream).await;
    session.cancel().await;
}

#[cfg(windows)]
#[tokio::test]
async fn windows_agent_auth_uses_upstream_discovery_and_signing() {
    let fixture = Fixture::start();
    let _agent = windows_agent::AgentFixture::start(&fixture.info.private_key).await;
    let mut c = fixture.connection();
    c.auth_method = AuthMethod::Agent;
    let r = rule(&c, ForwardKind::Local).await;
    let session = fixture
        .engine()
        .connect_forward(&c, None, None, &r, None)
        .await
        .unwrap();
    let mut stream = TcpStream::connect(("127.0.0.1", r.listen_port))
        .await
        .unwrap();
    banner(&mut stream).await;
    session.cancel().await;
}

#[tokio::test]
async fn rejected_auth_unknown_host_key_and_occupied_port_fail_closed() {
    let fixture = Fixture::start();
    let c = fixture.connection();
    assert!(
        fixture
            .engine()
            .connect_forward_conn(&c, Some("wrong"), None, None)
            .await
            .is_err()
    );
    assert!(
        oryxis_ssh::SshEngine::new()
            .with_strict_host_key(true)
            .connect_forward_conn(&c, Some(PASSWORD), None, None)
            .await
            .is_err()
    );
    let conn = fixture
        .engine()
        .connect_forward_conn(&c, Some(PASSWORD), None, None)
        .await
        .unwrap();
    let occupied = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let mut r = rule(&c, ForwardKind::Local).await;
    r.listen_port = occupied.local_addr().unwrap().port();
    let err = conn
        .attach(&r)
        .await
        .expect_err("occupied port must not report running")
        .to_string();
    assert!(
        err.contains(&r.listen_port.to_string()),
        "actionable bind error: {err}"
    );
}

#[tokio::test]
async fn runtime_shares_connections_honors_late_stop_and_releases_deleted_remote_bind() {
    let fixture = Fixture::start();
    let c = fixture.connection();
    let dialer = Arc::new(TestDialer {
        connection: c.clone(),
        engine: fixture.engine(),
        calls: AtomicUsize::new(0),
    });
    let a = rule(&c, ForwardKind::Local).await;
    let b = rule(&c, ForwardKind::Remote).await;
    let runtime = TunnelRuntimeManager::new(dialer.clone(), vec![a.clone(), b.clone()], true);
    runtime.start(a.id).await.unwrap();
    runtime.start(b.id).await.unwrap();
    runtime.stop(a.id).await.unwrap();
    wait_status(&runtime, b.id, TunnelStatus::Running).await;
    assert_eq!(runtime.snapshot().tunnels[0].status, TunnelStatus::Stopped);
    assert_eq!(dialer.calls.load(Ordering::SeqCst), 1);
    runtime.start(a.id).await.unwrap();
    wait_status(&runtime, a.id, TunnelStatus::Running).await;
    assert_eq!(dialer.calls.load(Ordering::SeqCst), 1);
    runtime.stop(b.id).await.unwrap();
    runtime.configure(vec![a.clone()], true).await.unwrap();
    assert_released(b.listen_port).await;
    let mut stream = TcpStream::connect(("127.0.0.1", a.listen_port))
        .await
        .unwrap();
    banner(&mut stream).await;
    runtime.shutdown().await.unwrap();
    assert_released(a.listen_port).await;
}

#[tokio::test]
async fn runtime_auto_start_liveness_and_manual_stop_during_retry() {
    let mut fixture = Fixture::start();
    let c = fixture.connection();
    let dialer = Arc::new(TestDialer {
        connection: c.clone(),
        engine: fixture.engine(),
        calls: AtomicUsize::new(0),
    });
    let mut a = rule(&c, ForwardKind::Local).await;
    a.auto_start = true;
    let runtime = TunnelRuntimeManager::new(dialer.clone(), vec![a.clone()], true);
    runtime.auto_start().await.unwrap();
    wait_status(&runtime, a.id, TunnelStatus::Running).await;
    fixture.child.kill().unwrap();
    fixture.child.wait().unwrap();
    wait_status(&runtime, a.id, TunnelStatus::Retrying).await;
    assert!(runtime.snapshot().tunnels[0].next_retry_at.is_some());
    runtime.stop(a.id).await.unwrap();
    assert_eq!(runtime.snapshot().tunnels[0].status, TunnelStatus::Stopped);
    assert!(runtime.snapshot().tunnels[0].next_retry_at.is_none());
    runtime.shutdown().await.unwrap();
}

#[tokio::test]
async fn credential_adapter_and_upstream_nested_jump_routes_work_end_to_end() {
    let fixtures: Vec<_> = (0..3).map(|_| Fixture::start()).collect();
    let mut hosts: Vec<_> = fixtures.iter().map(Fixture::connection).collect();
    hosts[1].jump_chain = vec![hosts[0].id];
    hosts[2].jump_chain = vec![hosts[1].id];
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(ConfigStore::open(dir.path().join("config.json")).unwrap());
    let secrets = Arc::new(TestSecrets::default());
    for h in &hosts {
        secrets.set(h.id, PASSWORD).unwrap();
    }
    store
        .update(|c| {
            c.hosts = hosts.clone();
            c.known_hosts = fixtures
                .iter()
                .map(|f| {
                    KnownHost::new("127.0.0.1", f.info.port, "ssh-ed25519", &f.info.fingerprint)
                })
                .collect();
            Ok(())
        })
        .unwrap();
    let prompts = Arc::new(PromptBroker::new(store.clone()));
    let dialer = ApplicationDialer {
        store,
        secrets,
        prompts,
    };
    for index in [1, 2] {
        let conn = dialer.connect(hosts[index].id, true).await.unwrap();
        let r = rule(&hosts[index], ForwardKind::Local).await;
        let session = conn.attach(&r).await.unwrap();
        let mut stream = TcpStream::connect(("127.0.0.1", r.listen_port))
            .await
            .unwrap();
        banner(&mut stream).await;
        session.cancel().await;
    }
}

#[tokio::test]
async fn manual_host_key_consent_persists_trust_and_unattended_unknown_keys_fail() {
    let fixture = Fixture::start();
    let host = fixture.connection();
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(ConfigStore::open(dir.path().join("config.json")).unwrap());
    store
        .update(|c| {
            c.hosts.push(host.clone());
            Ok(())
        })
        .unwrap();
    let secrets = Arc::new(TestSecrets::default());
    secrets.set(host.id, PASSWORD).unwrap();
    let prompts = Arc::new(PromptBroker::new(store.clone()));
    let dialer = ApplicationDialer {
        store: store.clone(),
        secrets,
        prompts: prompts.clone(),
    };
    assert!(dialer.connect(host.id, true).await.is_err());
    assert!(prompts.snapshot().is_empty());
    let mut questions = prompts.subscribe();
    let connect = tokio::spawn(dialer.connect(host.id, false));
    tokio::time::timeout(
        Duration::from_secs(5),
        questions.wait_for(|q| !q.is_empty()),
    )
    .await
    .unwrap()
    .unwrap();
    let question = prompts.snapshot()[0].clone();
    assert_eq!(question.fingerprint, fixture.info.fingerprint);
    prompts.respond(question.id, true).unwrap();
    assert!(connect.await.unwrap().is_ok());
    assert_eq!(
        ConfigStore::open(store.path())
            .unwrap()
            .snapshot()
            .known_hosts
            .len(),
        1
    );
    assert!(dialer.connect(host.id, true).await.is_ok());
}

#[tokio::test]
async fn tauri_facing_service_crud_start_stop_restart_and_config_reload() {
    let fixture = Fixture::start();
    let c = fixture.connection();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.json");
    let store = Arc::new(ConfigStore::open(&path).unwrap());
    let secrets = Arc::new(TestSecrets::default());
    let service = AppService::new(store.clone(), secrets.clone());
    let h = HostDto {
        id: c.id,
        name: "SSH fixture".into(),
        hostname: c.hostname.clone(),
        ssh_port: c.port,
        username: "tester".into(),
        auth_method: AuthMethod::Password,
        identity_file: String::new(),
        jump_chain: vec![],
        keepalive_seconds: 30,
        has_password: false,
    };
    service
        .save_host(SaveHost {
            profile: h.clone(),
            password: Some(PASSWORD.into()),
            clear_password: false,
        })
        .await
        .unwrap();
    assert!(service.list_hosts().unwrap()[0].has_password);
    let mut cycle = h.clone();
    cycle.jump_chain.push(h.id);
    assert!(
        service
            .save_host(SaveHost {
                profile: cycle,
                password: None,
                clear_password: false
            })
            .await
            .unwrap_err()
            .contains("cycle")
    );
    store
        .update(|c| {
            c.known_hosts.push(KnownHost::new(
                "127.0.0.1",
                fixture.info.port,
                "ssh-ed25519",
                &fixture.info.fingerprint,
            ));
            Ok(())
        })
        .unwrap();
    let r = rule(&c, ForwardKind::Remote).await;
    let mut dto = TunnelDto {
        id: r.id,
        name: "remote".into(),
        kind: r.kind,
        host_id: c.id,
        listen_address: r.listen_host.clone(),
        listen_port: 0,
        target_host: r.target_host.clone(),
        target_port: r.target_port,
        auto_start: false,
    };
    assert!(service.save_tunnel(dto.clone()).await.is_err());
    dto.listen_port = r.listen_port;
    service.save_tunnel(dto).await.unwrap();
    assert!(service.delete_host(h.id).await.is_err());
    service.start_tunnel(r.id).await.unwrap();
    assert_ne!(
        service.get_runtime_snapshot().tunnels[0].status,
        TunnelStatus::Running
    );
    wait_status(&service.runtime, r.id, TunnelStatus::Running).await;
    service.restart_tunnel(r.id).await.unwrap();
    wait_status(&service.runtime, r.id, TunnelStatus::Running).await;
    service.stop_tunnel(r.id).await.unwrap();
    assert_released(r.listen_port).await;
    service.runtime.shutdown().await.unwrap();
    let saved = std::fs::read_to_string(&path).unwrap();
    assert!(!saved.contains(PASSWORD));
    let restored = AppService::new(Arc::new(ConfigStore::open(&path).unwrap()), secrets);
    assert_eq!(restored.list_hosts().unwrap().len(), 1);
    assert_eq!(
        restored.get_runtime_snapshot().tunnels[0].status,
        TunnelStatus::Stopped
    );
    restored.start_tunnel(r.id).await.unwrap();
    wait_status(&restored.runtime, r.id, TunnelStatus::Running).await;
    restored.delete_tunnel(r.id).await.unwrap();
    assert_released(r.listen_port).await;
    assert!(restored.list_tunnels().is_empty());
    restored.delete_host(h.id).await.unwrap();
    assert!(restored.list_hosts().unwrap().is_empty());
    restored.runtime.shutdown().await.unwrap();
}

#[tokio::test]
async fn encrypted_key_host_test_cancel_retry_cache_and_report_are_isolated() {
    let fixture = Fixture::start();
    let mut host = fixture.connection();
    let dir = tempfile::tempdir().unwrap();
    let key_path = dir.path().join("encrypted.key");
    std::fs::write(&key_path, &fixture.info.encrypted_private_key).unwrap();
    host.auth_method = AuthMethod::Key;
    host.identity_file = Some(key_path.to_string_lossy().into_owned());
    let r = rule(&host, ForwardKind::Local).await;
    let store = Arc::new(ConfigStore::open(dir.path().join("config.json")).unwrap());
    store
        .update(|c| {
            c.hosts.push(host.clone());
            c.tunnels.push(r.clone());
            c.known_hosts.push(KnownHost::new(
                "127.0.0.1",
                fixture.info.port,
                "ssh-ed25519",
                &fixture.info.fingerprint,
            ));
            Ok(())
        })
        .unwrap();
    let service = AppService::new(store.clone(), Arc::new(TestSecrets::default()));
    let mut questions = service.prompts.key_subscribe();
    let run_test = || {
        let s = service.clone();
        let id = host.id;
        tokio::spawn(async move { s.test_host(id).await })
    };
    let cancelled = run_test();
    tokio::time::timeout(
        Duration::from_secs(10),
        questions.wait_for(|q| !q.is_empty()),
    )
    .await
    .unwrap()
    .unwrap();
    service.cancel_host_test(host.id);
    assert!(cancelled.await.unwrap().unwrap_err().contains("cancelled"));
    assert!(service.prompts.key_snapshot().is_empty());
    let test = run_test();
    tokio::time::timeout(
        Duration::from_secs(10),
        questions.wait_for(|q| !q.is_empty()),
    )
    .await
    .unwrap()
    .unwrap();
    let first = service.prompts.key_snapshot()[0].id;
    service
        .prompts
        .respond_key(first, Some("wrong passphrase".into()))
        .unwrap();
    tokio::time::timeout(
        Duration::from_secs(10),
        questions.wait_for(|q| q.iter().any(|p| p.error.is_some())),
    )
    .await
    .unwrap()
    .unwrap();
    let retry = service.prompts.key_snapshot()[0].clone();
    assert_ne!(first, retry.id);
    service
        .prompts
        .respond_key(retry.id, Some("fixture-key-passphrase".into()))
        .unwrap();
    assert!(test.await.unwrap().unwrap().message.contains("not tested"));
    assert_eq!(
        service.get_runtime_snapshot().tunnels[0].status,
        TunnelStatus::Stopped
    );
    assert_released(r.listen_port).await;
    service.start_tunnel(r.id).await.unwrap();
    wait_status(&service.runtime, r.id, TunnelStatus::Running).await;
    assert!(service.test_host(host.id).await.is_ok());
    service.cancel_ui_questions();
    let mut stream = TcpStream::connect(("127.0.0.1", r.listen_port))
        .await
        .unwrap();
    banner(&mut stream).await;
    service
        .logs
        .push("error", None, "password=never-export-this-value");
    service
        .logs
        .push("error", None, fixture.info.private_key.clone());
    let report = service.diagnostic_report().unwrap();
    let json: serde_json::Value = serde_json::from_str(&report).unwrap();
    assert!(json["errors"].as_array().unwrap().len() >= 2);
    assert!(!report.contains("never-export-this-value"));
    assert!(!report.contains("fixture-key-passphrase"));
    assert!(!report.contains("BEGIN OPENSSH PRIVATE KEY"));
    let saved = std::fs::read_to_string(store.path()).unwrap();
    assert!(!saved.contains("fixture-key-passphrase"));
    assert!(!saved.contains("PRIVATE KEY"));
    service.runtime.shutdown().await.unwrap();
    // A fresh process/session must ask again; unattended startup never prompts.
    let fresh = ApplicationDialer {
        store,
        secrets: Arc::new(TestSecrets::default()),
        prompts: Arc::new(PromptBroker::new(Arc::new(
            ConfigStore::open(dir.path().join("config.json")).unwrap(),
        ))),
    };
    assert!(
        fresh
            .connect(host.id, true)
            .await
            .err()
            .unwrap()
            .contains("locked")
    );
    assert!(fresh.prompts.key_snapshot().is_empty());
}

#[tokio::test]
async fn remote_policy_refusal_preserves_engine_error_and_explains_server_requirements() {
    let fixture = Fixture::start();
    let host = fixture.connection();
    let mut r = rule(&host, ForwardKind::Remote).await;
    r.listen_host = "0.0.0.0".into(); // fixture deliberately refuses external listeners
    let dialer = Arc::new(TestDialer {
        connection: host,
        engine: fixture.engine(),
        calls: AtomicUsize::new(0),
    });
    let runtime = TunnelRuntimeManager::new(dialer, vec![r.clone()], false);
    runtime.start(r.id).await.unwrap();
    wait_status(&runtime, r.id, TunnelStatus::Error).await;
    let error = runtime.snapshot().tunnels[0].last_error.clone().unwrap();
    assert!(error.contains("GatewayPorts"));
    assert!(error.contains("0.0.0.0"));
    assert!(error.contains("not established"));
    runtime.shutdown().await.unwrap();
}
