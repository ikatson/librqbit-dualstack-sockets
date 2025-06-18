use std::{
    net::{Ipv4Addr, Ipv6Addr, SocketAddrV4, SocketAddrV6},
    time::Duration,
};

use bstr::BStr;
use tracing::trace;

use crate::MulticastUdpSocket;

const SSDP_PORT: u16 = 1900;
const SSDP_MCAST_IPV4: SocketAddrV4 =
    SocketAddrV4::new(Ipv4Addr::new(239, 255, 255, 250), SSDP_PORT);
const SSDP_MCAST_IPV6_LINK_LOCAL: SocketAddrV6 = SocketAddrV6::new(
    Ipv6Addr::new(0xff02, 0, 0, 0, 0, 0, 0, 0xc),
    SSDP_PORT,
    0,
    0,
);
const SSDP_MCAST_IPV6_SITE_LOCAL: SocketAddrV6 = SocketAddrV6::new(
    Ipv6Addr::new(0xff05, 0, 0, 0, 0, 0, 0, 0xc),
    SSDP_PORT,
    0,
    0,
);

pub fn setup_test_logging() {
    unsafe { std::env::set_var("RUST_BACKTRACE", "1") };
    if std::env::var("RUST_LOG").is_err() {
        unsafe { std::env::set_var("RUST_LOG", "trace") };
    }
    let _ = tracing_subscriber::fmt::try_init();
}

#[tokio::test]
async fn multicast_example() {
    setup_test_logging();
    let sock = MulticastUdpSocket::new(
        (Ipv6Addr::UNSPECIFIED, SSDP_PORT).into(),
        SSDP_MCAST_IPV4,
        SSDP_MCAST_IPV6_SITE_LOCAL,
        Some(SSDP_MCAST_IPV6_LINK_LOCAL),
    )
    .await
    .unwrap();

    let recv = async {
        let mut buf = [0u8; 256];
        while let Ok(()) = tokio::time::timeout(Duration::from_millis(100), async {
            let (payload, addr) = sock.recv_from(&mut buf).await.unwrap();
            let payload = BStr::new(&buf[..payload]);
            println!("received from {addr:?}: {payload}");
        })
        .await
        {}

        trace!("recv timed out")
    };

    let send = sock.try_send_mcast_everywhere(&|mopts| format!("{mopts:?}").into());

    tokio::join!(recv, send);
}
