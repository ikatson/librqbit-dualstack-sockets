#[cfg(test)]
mod tests;

mod error;
mod multicast;
mod traits;
pub use error::{Error, Result};

use crate::socket::MaybeDualstackSocket;

pub mod addr;
pub mod socket;

pub type TcpListener = MaybeDualstackSocket<tokio::net::TcpListener>;
pub type UdpSocket = MaybeDualstackSocket<tokio::net::UdpSocket>;
pub use multicast::{MulticastOpts, MulticastUdpSocket};
pub use socket::BindOpts;
pub use traits::PollSendToVectored;

#[cfg(feature = "axum")]
pub use socket::axum::WrappedSocketAddr;
