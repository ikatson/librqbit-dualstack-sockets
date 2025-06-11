use std::{
    net::{IpAddr, Ipv6Addr, SocketAddr, SocketAddrV4, SocketAddrV6},
    task::Poll,
};

use anyhow::Context;
use socket2::{Domain, Socket};
use tracing::{debug, trace};

use crate::addr::{ToV6Mapped, TryToV4};

#[derive(Clone, Copy, Debug)]
pub enum SocketAddrKind {
    V4(SocketAddrV4),
    V6 {
        addr: SocketAddrV6,
        is_dualstack: bool,
    },
}

impl SocketAddrKind {
    fn as_socketaddr(&self) -> SocketAddr {
        match *self {
            SocketAddrKind::V4(addr) => SocketAddr::V4(addr),
            SocketAddrKind::V6 { addr, .. } => SocketAddr::V6(addr),
        }
    }
}

pub struct MaybeDualstackSocket<S> {
    socket: S,
    addr_kind: SocketAddrKind,
}

impl<S> MaybeDualstackSocket<S> {
    pub fn socket(&self) -> &S {
        &self.socket
    }

    pub fn bind_addr(&self) -> SocketAddr {
        self.addr_kind.as_socketaddr()
    }

    pub fn is_dualstack(&self) -> bool {
        matches!(
            self.addr_kind,
            SocketAddrKind::V6 {
                is_dualstack: true,
                ..
            }
        )
    }

    fn convert_addr_for_send(&self, addr: SocketAddr) -> SocketAddr {
        if self.is_dualstack() {
            return SocketAddr::V6(addr.to_ipv6_mapped());
        }
        addr
    }
}

impl MaybeDualstackSocket<Socket> {
    fn bind(addr: SocketAddr, request_dualstack: bool, is_udp: bool) -> anyhow::Result<Self> {
        let socket = Socket::new(
            if addr.is_ipv6() {
                Domain::IPV6
            } else {
                Domain::IPV4
            },
            if is_udp {
                socket2::Type::DGRAM
            } else {
                socket2::Type::STREAM
            },
            Some(if is_udp {
                socket2::Protocol::UDP
            } else {
                socket2::Protocol::TCP
            }),
        )?;

        let mut set_dualstack = false;

        let addr_kind = match (request_dualstack, addr) {
            (request_dualstack, SocketAddr::V6(addr))
                if *addr.ip() == IpAddr::V6(Ipv6Addr::UNSPECIFIED) =>
            {
                let value = !request_dualstack;
                trace!(?addr, only_v6 = value, "setting only_v6");
                socket
                    .set_only_v6(value)
                    .with_context(|| format!("error setting only_v6={value}"))?;
                #[cfg(not(windows))] // socket.only_v6() panics on windows somehow
                trace!(?addr, only_v6=?socket.only_v6().context("error getting only_v6"));
                set_dualstack = true;
                SocketAddrKind::V6 {
                    addr,
                    is_dualstack: request_dualstack,
                }
            }
            (_, SocketAddr::V6(addr)) => SocketAddrKind::V6 {
                addr,
                is_dualstack: false,
            },
            (_, SocketAddr::V4(addr)) => SocketAddrKind::V4(addr),
        };

        if !set_dualstack {
            debug!(
                ?addr,
                "ignored dualstack request as it only applies to [::] address"
            );
        }

        #[cfg(not(windows))]
        {
            if !is_udp {
                socket
                    .set_reuse_address(true)
                    .context("error setting SO_REUSEADDR")?;
            }
        }

        socket
            .bind(&addr.into())
            .context(addr)
            .context("error binding")?;

        let local_addr: SocketAddr = socket
            .local_addr()?
            .as_socket()
            .context("as_socket returned None")?;

        let addr_kind = match (addr_kind, local_addr) {
            (SocketAddrKind::V4(..), SocketAddr::V4(received)) => SocketAddrKind::V4(received),
            (SocketAddrKind::V6 { is_dualstack, .. }, SocketAddr::V6(received)) => {
                SocketAddrKind::V6 {
                    addr: received,
                    is_dualstack,
                }
            }
            _ => anyhow::bail!(
                "mismatch between local_addr({local_addr:?}) and requested bind_addr({addr:?})"
            ),
        };

        socket
            .set_nonblocking(true)
            .context("error setting nonblocking=true")?;

        Ok(Self { socket, addr_kind })
    }
}

impl MaybeDualstackSocket<tokio::net::TcpListener> {
    pub fn bind_tcp(addr: SocketAddr, request_dualstack: bool) -> anyhow::Result<Self> {
        let sock = MaybeDualstackSocket::bind(addr, request_dualstack, false)?;

        debug!(addr=?sock.bind_addr(), requested_addr=?addr, dualstack = sock.is_dualstack(), "listening on TCP");
        sock.socket().listen(1024).context("error listening")?;

        Ok(Self {
            socket: tokio::net::TcpListener::from_std(std::net::TcpListener::from(sock.socket))?,
            addr_kind: sock.addr_kind,
        })
    }

    pub async fn accept(&self) -> std::io::Result<(tokio::net::TcpStream, SocketAddr)> {
        let (s, addr) = self.socket.accept().await?;
        Ok((s, addr.try_to_ipv4()))
    }
}

impl MaybeDualstackSocket<tokio::net::UdpSocket> {
    pub fn bind_udp(addr: SocketAddr, request_dualstack: bool) -> anyhow::Result<Self> {
        let sock = MaybeDualstackSocket::bind(addr, request_dualstack, true)?;

        debug!(addr=?sock.bind_addr(), requested_addr=?addr, dualstack = sock.is_dualstack(), "listening on UDP");

        Ok(Self {
            socket: tokio::net::UdpSocket::from_std(std::net::UdpSocket::from(sock.socket))?,
            addr_kind: sock.addr_kind,
        })
    }

    pub async fn recv_from(&self, buf: &mut [u8]) -> std::io::Result<(usize, SocketAddr)> {
        let (size, addr) = self.socket.recv_from(buf).await?;
        Ok((size, addr.try_to_ipv4()))
    }

    pub async fn send_to(&self, buf: &[u8], target: SocketAddr) -> std::io::Result<usize> {
        let target = self.convert_addr_for_send(target);
        self.socket.send_to(buf, target).await
    }

    pub fn poll_send_to(
        &self,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
        target: SocketAddr,
    ) -> Poll<std::io::Result<usize>> {
        let target = self.convert_addr_for_send(target);
        self.socket.poll_send_to(cx, buf, target)
    }
}
