use std::{fs, io, path::PathBuf};

use russh::{
    client,
    keys::{HashAlg, PublicKeyOrCertificate, known_hosts},
};
use thiserror::Error;
use tunnel_domain::HostKeyPolicy;

const MAX_KNOWN_HOSTS_BYTES: u64 = 1024 * 1024;

#[derive(Debug, Error)]
pub enum HostKeyError {
    #[error(transparent)]
    Protocol(#[from] russh::Error),
    #[error("host key for {host}:{port} is unknown ({fingerprint})")]
    Unknown {
        host: String,
        port: u16,
        fingerprint: String,
    },
    #[error("host key changed for {host}:{port} in {path}, line {line}")]
    Changed {
        host: String,
        port: u16,
        path: PathBuf,
        line: usize,
    },
    #[error("SSH host certificates are not supported yet for {host}:{port}")]
    UnsupportedCertificate { host: String, port: u16 },
    #[error("cannot read known_hosts at {path}: {source}")]
    KnownHostsIo {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("known_hosts at {path} exceeds {MAX_KNOWN_HOSTS_BYTES} bytes")]
    KnownHostsTooLarge { path: PathBuf },
    #[error("invalid known_hosts at {path}: {source}")]
    InvalidKnownHosts {
        path: PathBuf,
        #[source]
        source: russh::keys::Error,
    },
}

pub(crate) struct HostKeyVerifier {
    pub(crate) host: String,
    pub(crate) port: u16,
    pub(crate) paths: Vec<PathBuf>,
    pub(crate) policy: HostKeyPolicy,
}

impl client::Handler for HostKeyVerifier {
    type Error = HostKeyError;

    async fn check_server_key(
        &mut self,
        server_public_key: &PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        if self.policy == HostKeyPolicy::Bypass {
            return Ok(true);
        }
        let PublicKeyOrCertificate::PublicKey { key, .. } = server_public_key else {
            return Err(HostKeyError::UnsupportedCertificate {
                host: self.host.clone(),
                port: self.port,
            });
        };

        let mut found = false;
        for path in &self.paths {
            let metadata = match fs::metadata(path) {
                Ok(metadata) => metadata,
                Err(source) if source.kind() == io::ErrorKind::NotFound => continue,
                Err(source) => {
                    return Err(HostKeyError::KnownHostsIo {
                        path: path.clone(),
                        source,
                    });
                }
            };
            if !metadata.is_file() {
                return Err(HostKeyError::KnownHostsIo {
                    path: path.clone(),
                    source: io::Error::new(io::ErrorKind::InvalidData, "expected a regular file"),
                });
            }
            if metadata.len() > MAX_KNOWN_HOSTS_BYTES {
                return Err(HostKeyError::KnownHostsTooLarge { path: path.clone() });
            }
            match known_hosts::check_known_hosts_path(&self.host, self.port, key, path) {
                Ok(matched) => found |= matched,
                Err(russh::keys::Error::KeyChanged { line }) => {
                    return Err(HostKeyError::Changed {
                        host: self.host.clone(),
                        port: self.port,
                        path: path.clone(),
                        line,
                    });
                }
                Err(source) => {
                    return Err(HostKeyError::InvalidKnownHosts {
                        path: path.clone(),
                        source,
                    });
                }
            }
        }
        if found {
            Ok(true)
        } else {
            Err(HostKeyError::Unknown {
                host: self.host.clone(),
                port: self.port,
                fingerprint: key.fingerprint(HashAlg::Sha256).to_string(),
            })
        }
    }
}
