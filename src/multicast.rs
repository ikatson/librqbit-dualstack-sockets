#[cfg(test)]
mod tests;

use std::{
    collections::HashSet,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4, SocketAddrV6},
    sync::Mutex,
    task::Poll,
};

use network_interface::{NetworkInterface, NetworkInterfaceConfig};
use socket2::SockRef;
use tracing::{debug, trace};

use crate::{Error, UdpSocket};

pub struct MulticastUdpSocket {
    // At least on OSX, it multicast doesn't seem to work on dualstack sockets, so we need
    // to create 2 of them.
    sock_v4: UdpSocket,
    sock_v6: UdpSocket,
    ipv4_addr: Ipv4Addr,
    ipv6_site_local: Ipv6Addr,
    ipv6_link_local: Option<Ipv6Addr>,
    nics: Vec<NetworkInterface>,
}

impl MulticastUdpSocket {
    pub fn new(
        port: u16,
        ipv4_addr: Ipv4Addr,
        ipv6_site_local: Ipv6Addr,
        ipv6_link_local: Option<Ipv6Addr>,
    ) -> crate::Result<Self> {
        if let Some(ll) = ipv6_link_local {
            if !ipv6_is_link_local_mcast(ll) {
                return Err(Error::ProvidedLinkLocalAddrIsntLinkLocal);
            }
        }
        if !ipv6_is_site_local_mcast(ipv6_site_local) {
            return Err(Error::ProvidedSiteLocalAddrIsNotSiteLocal);
        }
        let nics = network_interface::NetworkInterface::show()
            .into_iter()
            .flatten()
            .collect::<Vec<_>>();
        if nics.is_empty() {
            return Err(Error::NoNics);
        }
        let sock_v4 = UdpSocket::bind_udp((Ipv4Addr::UNSPECIFIED, port).into(), false)?;
        let sock_v6 = UdpSocket::bind_udp((Ipv6Addr::UNSPECIFIED, port).into(), false)?;
        let sock = Self {
            sock_v4,
            sock_v6,
            ipv4_addr,
            ipv6_link_local,
            ipv6_site_local,
            nics,
        };
        sock.bind_multicast()?;
        Ok(sock)
    }

    pub async fn recv_from(&self, buf: &mut [u8]) -> std::io::Result<(usize, SocketAddr)> {
        std::future::poll_fn(|cx| {
            let mut buf = tokio::io::ReadBuf::new(buf);
            if let Poll::Ready(res) = self.sock_v4.socket().poll_recv_from(cx, &mut buf) {
                return Poll::Ready(res.map(|addr| (buf.filled().len(), addr)));
            }
            if let Poll::Ready(res) = self.sock_v6.socket().poll_recv_from(cx, &mut buf) {
                return Poll::Ready(res.map(|addr| (buf.filled().len(), addr)));
            }
            Poll::Pending
        })
        .await
    }

    pub async fn send_to(&self, buf: &[u8], addr: SocketAddr) -> std::io::Result<usize> {
        let sock = if addr.is_ipv6() {
            &self.sock_v6
        } else {
            &self.sock_v4
        };
        sock.send_to(buf, addr).await
    }

    fn bind_multicast(&self) -> crate::Result<()> {
        let mut joined = try_join_v4(&self.sock_v4, self.ipv4_addr, Ipv4Addr::UNSPECIFIED);

        for nic in self.nics.iter() {
            let mut has_link_local = false;
            let mut has_site_local = false;

            for addr in nic.addr.iter() {
                match addr.ip() {
                    IpAddr::V4(iface_addr)
                        if iface_addr.is_private() && !iface_addr.is_loopback() =>
                    {
                        joined |= try_join_v4(&self.sock_v4, self.ipv4_addr, iface_addr);
                    }
                    IpAddr::V6(addr) => {
                        if addr.is_loopback() {
                            continue;
                        }
                        if ipv6_is_link_local(addr) {
                            has_link_local = true;
                        } else {
                            has_site_local = true;
                        }
                    }
                    _ => continue,
                }
            }

            if has_site_local {
                joined |= try_join_v6(&self.sock_v6, self.ipv6_site_local, nic.index);
            }

            if let Some(ll) = self.ipv6_link_local {
                if has_link_local {
                    joined |= try_join_v6(&self.sock_v6, ll, nic.index);
                }
            }
        }

        if !joined {
            return Err(Error::MulticastJoinFail);
        }

        Ok(())
    }

    async fn send_to_once(&self, buf: &[u8], opts: &MulticastOpts) -> std::io::Result<usize> {
        // This is .poll_fn() so that we call .set_multicast() immediately before sending a packet.
        // If it's repolled it'll get called again just before the send.

        std::future::poll_fn(|cx| {
            let sock;
            let mcast_addr_s: SocketAddr;

            match opts {
                MulticastOpts::V4 {
                    interface_addr,
                    mcast_addr,
                } => {
                    sock = &self.sock_v4;
                    mcast_addr_s = (*mcast_addr).into();
                    if let Err(e) = SockRef::from(sock.socket()).set_multicast_if_v4(interface_addr)
                    {
                        debug!(addr=%interface_addr, "error calling set_multicast_if_v4: {e:#}");
                        return Poll::Ready(Err(e));
                    }
                }
                MulticastOpts::V6 {
                    interface_id,
                    mcast_addr,
                    ..
                } => {
                    sock = &self.sock_v6;
                    mcast_addr_s = (*mcast_addr).into();
                    if let Err(e) = SockRef::from(sock.socket()).set_multicast_if_v6(*interface_id)
                    {
                        debug!(
                            oif_id = interface_id,
                            "error calling set_multicast_if_v6: {e:#}"
                        );
                        return Poll::Ready(Err(e));
                    }
                }
            }

            sock.poll_send_to(cx, buf, mcast_addr_s)
        })
        .await
    }

    pub async fn try_send_mcast_everywhere(
        &self,
        get_payload: &impl Fn(&MulticastOpts) -> bstr::BString,
    ) {
        // Without this it blocks for some reason. Maybe we need to do it once in new(), so that all multicast joining
        // messages are actually sent?
        //
        // It also works if we call .send_to() vs .poll_send_to() underneath. Maybe a bug in tokio/mio or I'm just
        // misusing it.
        let _ = self.sock_v6.socket().writable().await;

        let sent = Mutex::new(HashSet::new());
        let sent = &sent;

        let port = self.sock_v4.bind_addr().port();

        let futs = self
            .nics
            .iter()
            .flat_map(|ni| ni.addr.iter().map(move |a| (ni.index, a.ip())))
            .filter_map(|(ifidx, ifaddr)| {
                let ipv6_link_local = self
                    .ipv6_link_local
                    .filter(|_| matches!(ifaddr, IpAddr::V6(v6) if ipv6_is_link_local(v6)));
                let opts = match (ifaddr, ipv6_link_local) {
                    (IpAddr::V4(a), _) if !a.is_loopback() && a.is_private() => MulticastOpts::V4 {
                        interface_addr: a,
                        mcast_addr: SocketAddrV4::new(self.ipv4_addr, port),
                    },
                    (IpAddr::V6(a), Some(mlocal)) if !a.is_loopback() => MulticastOpts::V6 {
                        interface_id: ifidx,
                        interface_addr: a,
                        mcast_addr: SocketAddrV6::new(mlocal, port, 0, ifidx),
                    },
                    (IpAddr::V6(a), None) if !a.is_loopback() => MulticastOpts::V6 {
                        interface_id: ifidx,
                        interface_addr: a,
                        mcast_addr: SocketAddrV6::new(self.ipv6_site_local, port, 0, ifidx),
                    },
                    _ => {
                        trace!(oif_id=ifidx, addr=%ifaddr, "ignoring address");
                        return None;
                    }
                };
                Some(opts)
            })
            .map(|opts| async move {
                let payload = get_payload(&opts);
                if !sent
                    .lock()
                    .unwrap()
                    .insert((payload.clone(), opts.uniq_key()))
                {
                    trace!(?opts, "not sending duplicate payload");
                    return;
                }

                match self.send_to_once(payload.as_slice(), &opts).await {
                    Ok(sz) => trace!(?opts, size=sz, payload=?payload, "sent"),
                    Err(e) => {
                        debug!(?opts, payload=?payload, "error sending: {e:#}")
                    }
                }
            });

        futures::future::join_all(futs).await;
    }
}

fn try_join_v4(sock: &UdpSocket, addr: Ipv4Addr, iface: Ipv4Addr) -> bool {
    trace!(multiaddr=?addr, interface=?iface, "joining multicast v4 group");
    if let Err(e) = sock.socket().join_multicast_v4(addr, iface) {
        debug!(multiaddr=?addr, interface=?iface, "error joining multicast v4 group: {e:#}");
        return false;
    }
    true
}

fn try_join_v6(sock: &UdpSocket, addr: Ipv6Addr, ifindex: u32) -> bool {
    trace!(multiaddr=?addr, interface=?ifindex, "joining multicast v6 group");
    if let Err(e) = sock.socket().join_multicast_v6(&addr, ifindex) {
        debug!(multiaddr=?addr, interface=?ifindex, "error joining multicast v6 group: {e:#}");
        return false;
    }
    true
}

fn ipv6_is_link_local(ip: Ipv6Addr) -> bool {
    const LL: Ipv6Addr = Ipv6Addr::new(0xfe80, 0, 0, 0, 0, 0, 0, 0);
    const MASK: Ipv6Addr = Ipv6Addr::new(0xffff, 0xffff, 0xffff, 0xffff, 0, 0, 0, 0);

    ip.to_bits() & MASK.to_bits() == LL.to_bits() & MASK.to_bits()
}

fn ipv6_is_link_local_mcast(ip: Ipv6Addr) -> bool {
    const LL: Ipv6Addr = Ipv6Addr::new(0xff02, 0, 0, 0, 0, 0, 0, 0);
    const MASK: Ipv6Addr = Ipv6Addr::new(0xff0f, 0xffff, 0xffff, 0xffff, 0, 0, 0, 0);

    ip.to_bits() & MASK.to_bits() == LL.to_bits() & MASK.to_bits()
}

fn ipv6_is_site_local_mcast(ip: Ipv6Addr) -> bool {
    const LL: Ipv6Addr = Ipv6Addr::new(0xff05, 0, 0, 0, 0, 0, 0, 0);
    const MASK: Ipv6Addr = Ipv6Addr::new(0xff0f, 0xffff, 0xffff, 0xffff, 0, 0, 0, 0);

    ip.to_bits() & MASK.to_bits() == LL.to_bits() & MASK.to_bits()
}

#[derive(Debug, Hash, Clone, Copy, PartialEq, Eq)]
pub enum MulticastOpts {
    V4 {
        interface_addr: Ipv4Addr,
        mcast_addr: SocketAddrV4,
    },
    V6 {
        interface_id: u32,
        interface_addr: Ipv6Addr,
        mcast_addr: SocketAddrV6,
    },
}

impl MulticastOpts {
    pub fn iface_ip(&self) -> IpAddr {
        match self {
            MulticastOpts::V4 { interface_addr, .. } => (*interface_addr).into(),
            MulticastOpts::V6 { interface_addr, .. } => (*interface_addr).into(),
        }
    }

    pub fn mcast_addr(&self) -> SocketAddr {
        match self {
            MulticastOpts::V4 { mcast_addr, .. } => (*mcast_addr).into(),
            MulticastOpts::V6 { mcast_addr, .. } => (*mcast_addr).into(),
        }
    }

    fn uniq_key(&self) -> (Option<u32>, Option<Ipv4Addr>, SocketAddr) {
        match self {
            MulticastOpts::V4 {
                interface_addr,
                mcast_addr,
            } => (None, Some(*interface_addr), (*mcast_addr).into()),
            MulticastOpts::V6 {
                interface_id,
                mcast_addr,
                ..
            } => (Some(*interface_id), None, (*mcast_addr).into()),
        }
    }
}
