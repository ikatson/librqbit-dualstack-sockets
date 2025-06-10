use crate::socket::MaybeDualstackSocket;

pub mod addr;
pub mod socket;

pub type TcpListener = MaybeDualstackSocket<tokio::net::TcpListener>;
pub type UdpSocket = MaybeDualstackSocket<tokio::net::UdpSocket>;
