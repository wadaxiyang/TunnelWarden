use std::{
    future::Future,
    io,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
};
use thiserror::Error;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

const VERSION: u8 = 5;
const CONNECT: u8 = 1;
const NO_AUTH: u8 = 0;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Destination {
    Ip(SocketAddr),
    Domain { host: String, port: u16 },
}

impl Destination {
    pub fn host(&self) -> String {
        match self {
            Self::Ip(address) => address.ip().to_string(),
            Self::Domain { host, .. } => host.clone(),
        }
    }

    pub fn port(&self) -> u16 {
        match self {
            Self::Ip(address) => address.port(),
            Self::Domain { port, .. } => *port,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SocksConnectRequest {
    pub destination: Destination,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum SocksReply {
    Succeeded = 0,
    GeneralFailure = 1,
    NotAllowed = 2,
    NetworkUnreachable = 3,
    HostUnreachable = 4,
    ConnectionRefused = 5,
    TtlExpired = 6,
    CommandNotSupported = 7,
    AddressTypeNotSupported = 8,
}

#[derive(Debug, Error)]
pub enum SocksError {
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error("SOCKS5 client did not offer NO AUTH")]
    NoAcceptedMethod,
    #[error("invalid SOCKS5 request: {0}")]
    InvalidRequest(&'static str),
}

/// A narrow parser boundary. Business logic only receives a typed destination
/// and remains independent of the wire representation.
pub trait SocksFrontend {
    fn read_connect<S>(
        &self,
        stream: &mut S,
    ) -> impl Future<Output = Result<SocksConnectRequest, SocksError>> + Send
    where
        S: AsyncRead + AsyncWrite + Unpin + Send;

    fn reply<S>(
        &self,
        stream: &mut S,
        reply: SocksReply,
    ) -> impl Future<Output = Result<(), SocksError>> + Send
    where
        S: AsyncWrite + Unpin + Send;
}

#[derive(Clone, Copy, Default)]
pub struct Socks5Frontend;

impl SocksFrontend for Socks5Frontend {
    async fn read_connect<S>(&self, stream: &mut S) -> Result<SocksConnectRequest, SocksError>
    where
        S: AsyncRead + AsyncWrite + Unpin + Send,
    {
        let mut greeting = [0u8; 2];
        stream.read_exact(&mut greeting).await?;
        if greeting[0] != VERSION || greeting[1] == 0 {
            return Err(SocksError::InvalidRequest("invalid greeting"));
        }
        let mut methods = [0u8; 255];
        let count = usize::from(greeting[1]);
        stream.read_exact(&mut methods[..count]).await?;
        if !methods[..count].contains(&NO_AUTH) {
            stream.write_all(&[VERSION, 0xff]).await?;
            return Err(SocksError::NoAcceptedMethod);
        }
        stream.write_all(&[VERSION, NO_AUTH]).await?;

        let mut header = [0u8; 4];
        stream.read_exact(&mut header).await?;
        if header[0] != VERSION || header[2] != 0 {
            self.reply(stream, SocksReply::GeneralFailure).await?;
            return Err(SocksError::InvalidRequest("invalid request header"));
        }
        if header[1] != CONNECT {
            self.reply(stream, SocksReply::CommandNotSupported).await?;
            return Err(SocksError::InvalidRequest("only CONNECT is supported"));
        }
        let destination = match header[3] {
            1 => {
                let mut address = [0u8; 4];
                stream.read_exact(&mut address).await?;
                let port = read_port(stream).await?;
                Destination::Ip(SocketAddr::new(IpAddr::V4(Ipv4Addr::from(address)), port))
            }
            3 => {
                let length = stream.read_u8().await?;
                if length == 0 {
                    self.reply(stream, SocksReply::AddressTypeNotSupported)
                        .await?;
                    return Err(SocksError::InvalidRequest("empty domain"));
                }
                let mut bytes = [0u8; 255];
                stream.read_exact(&mut bytes[..usize::from(length)]).await?;
                let host = match std::str::from_utf8(&bytes[..usize::from(length)]) {
                    Ok(host) => host.to_owned(),
                    Err(_) => {
                        self.reply(stream, SocksReply::AddressTypeNotSupported)
                            .await?;
                        return Err(SocksError::InvalidRequest("domain is not UTF-8"));
                    }
                };
                let port = read_port(stream).await?;
                Destination::Domain { host, port }
            }
            4 => {
                let mut address = [0u8; 16];
                stream.read_exact(&mut address).await?;
                let port = read_port(stream).await?;
                Destination::Ip(SocketAddr::new(IpAddr::V6(Ipv6Addr::from(address)), port))
            }
            _ => {
                self.reply(stream, SocksReply::AddressTypeNotSupported)
                    .await?;
                return Err(SocksError::InvalidRequest("unsupported address type"));
            }
        };
        if destination.port() == 0 {
            self.reply(stream, SocksReply::GeneralFailure).await?;
            return Err(SocksError::InvalidRequest("port is zero"));
        }
        Ok(SocksConnectRequest { destination })
    }

    async fn reply<S>(&self, stream: &mut S, reply: SocksReply) -> Result<(), SocksError>
    where
        S: AsyncWrite + Unpin + Send,
    {
        stream
            .write_all(&[VERSION, reply as u8, 0, 1, 0, 0, 0, 0, 0, 0])
            .await?;
        Ok(())
    }
}

async fn read_port<S: AsyncRead + Unpin>(stream: &mut S) -> io::Result<u16> {
    let mut bytes = [0u8; 2];
    stream.read_exact(&mut bytes).await?;
    Ok(u16::from_be_bytes(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::duplex;

    #[tokio::test]
    async fn domain_is_preserved_for_remote_dns() {
        let (mut client, mut server) = duplex(512);
        let task = tokio::spawn(async move { Socks5Frontend.read_connect(&mut server).await });
        client.write_all(&[5, 1, 0]).await.expect("greeting");
        let mut selected = [0u8; 2];
        client.read_exact(&mut selected).await.expect("method");
        assert_eq!(selected, [5, 0]);
        client
            .write_all(&[5, 1, 0, 3, 11])
            .await
            .expect("request header");
        client.write_all(b"example.com").await.expect("domain");
        client.write_all(&443u16.to_be_bytes()).await.expect("port");
        let request = task.await.expect("parser task").expect("request");
        assert_eq!(
            request.destination,
            Destination::Domain {
                host: "example.com".into(),
                port: 443
            }
        );
    }

    #[tokio::test]
    async fn bind_command_is_rejected_with_standard_reply() {
        let (mut client, mut server) = duplex(512);
        let task = tokio::spawn(async move { Socks5Frontend.read_connect(&mut server).await });
        client.write_all(&[5, 1, 0]).await.expect("greeting");
        let mut selected = [0u8; 2];
        client.read_exact(&mut selected).await.expect("method");
        client.write_all(&[5, 2, 0, 1]).await.expect("BIND request");
        let mut reply = [0u8; 10];
        client.read_exact(&mut reply).await.expect("failure reply");
        assert_eq!(reply[1], SocksReply::CommandNotSupported as u8);
        assert!(task.await.expect("parser task").is_err());
    }

    #[tokio::test]
    async fn ipv6_connect_address_is_parsed() {
        let (mut client, mut server) = duplex(512);
        let task = tokio::spawn(async move { Socks5Frontend.read_connect(&mut server).await });
        client.write_all(&[5, 1, 0]).await.expect("greeting");
        let mut selected = [0u8; 2];
        client.read_exact(&mut selected).await.expect("method");
        let mut request = vec![5, 1, 0, 4];
        request.extend_from_slice(&Ipv6Addr::LOCALHOST.octets());
        request.extend_from_slice(&80u16.to_be_bytes());
        client.write_all(&request).await.expect("IPv6 request");
        let parsed = task.await.expect("parser task").expect("request");
        assert_eq!(
            parsed.destination,
            Destination::Ip(SocketAddr::new(IpAddr::V6(Ipv6Addr::LOCALHOST), 80))
        );
    }
}
