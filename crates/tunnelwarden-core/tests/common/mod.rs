use oryxis_core::models::{
    connection::{AuthMethod, Connection},
    port_forward_rule::{ForwardKind, PortForwardRule},
};
use oryxis_ssh::{HostKeyStatus, SshEngine};
use serde::Deserialize;
use std::{
    io::{BufRead, BufReader},
    process::{Child, Command, Stdio},
    sync::Arc,
    time::Duration,
};
use tokio::{
    io::AsyncReadExt,
    net::{TcpListener, TcpStream},
};

#[derive(Deserialize)]
pub struct FixtureInfo {
    pub port: u16,
    pub fingerprint: String,
    pub private_key: String,
    pub encrypted_private_key: String,
}

pub struct Fixture {
    pub child: Child,
    pub info: FixtureInfo,
}

impl Fixture {
    pub fn start() -> Self {
        let python = std::env::var("TUNNELWARDEN_TEST_PYTHON").unwrap_or_else(|_| "python".into());
        let mut child = Command::new(python)
            .arg(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../tests/ssh_fixture.py"
            ))
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("Python fixture; install tests/requirements.txt");
        let mut line = String::new();
        BufReader::new(child.stdout.take().unwrap())
            .read_line(&mut line)
            .unwrap();
        let info = serde_json::from_str(&line).expect("SSH fixture ready JSON");
        Self { child, info }
    }

    pub fn connection(&self) -> Connection {
        let mut c = Connection::new("fixture", "127.0.0.1");
        c.port = self.info.port;
        c.username = Some("tester".into());
        c.auth_method = AuthMethod::Password;
        c
    }

    pub fn engine(&self) -> SshEngine {
        let expected = self.info.fingerprint.clone();
        SshEngine::new()
            .with_strict_host_key(true)
            .with_host_key_check(Arc::new(move |_, _, _, fp| {
                if fp == expected {
                    HostKeyStatus::Known
                } else {
                    HostKeyStatus::Unknown
                }
            }))
            .with_connect_timeout(Duration::from_secs(5))
            .with_auth_timeout(Duration::from_secs(5))
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

pub const PASSWORD: &str = "fixture-only-password";

pub async fn rule(c: &Connection, kind: ForwardKind) -> PortForwardRule {
    let probe = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let mut r = PortForwardRule::new("fixture tunnel", kind, c.id);
    r.listen_port = probe.local_addr().unwrap().port();
    r.target_host = "127.0.0.1".into();
    r.target_port = c.port;
    r
}

pub async fn banner(stream: &mut TcpStream) {
    let mut buf = [0; 7];
    tokio::time::timeout(Duration::from_secs(5), stream.read_exact(&mut buf))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(&buf, b"SSH-2.0");
}

pub async fn assert_released(port: u16) {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if TcpListener::bind(("127.0.0.1", port)).await.is_ok() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("listener released after explicit cancel");
}
