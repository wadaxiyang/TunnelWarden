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
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
};

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
