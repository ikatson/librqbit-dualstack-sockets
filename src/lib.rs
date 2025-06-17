#[cfg(test)]
mod tests;

mod constants;
mod error;
mod multicast;
pub use error::{Error, Result};

use crate::socket::MaybeDualstackSocket;

pub mod addr;
pub mod socket;

pub type TcpListener = MaybeDualstackSocket<tokio::net::TcpListener>;
pub type UdpSocket = MaybeDualstackSocket<tokio::net::UdpSocket>;
pub use multicast::{
    HandlerDropGuard, MulticastOpts, MulticastUdpSocket, SharedMulticastUdpSocket,
    create_ssdp_socket,
};
pub use socket::BindOpts;

#[cfg(feature = "axum")]
pub use socket::axum::WrappedSocketAddr;
