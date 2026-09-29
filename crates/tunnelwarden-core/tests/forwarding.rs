// Copyright (C) 2026 Wilson Glasser and Oryxis contributors
// SPDX-License-Identifier: AGPL-3.0-or-later
// Adapted 2026-09-29 for TunnelWarden: loopback AsyncSSH fixture, all modes
// on one public ForwardConn, deterministic host-key pin, cancellation checks.
// Source: crates/oryxis-ssh/tests/ssh_integration.rs
// Oryxis 16ad2af83b424780ff60b6420a6c20a3b30c7143.
mod common;
use common::*;
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
    dto::TunnelStatus,
    runtime::{DialFuture, ForwardDialer, TunnelRuntimeManager},
};

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
