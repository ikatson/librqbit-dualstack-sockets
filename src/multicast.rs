use std::{
    collections::HashSet,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV6},
    sync::Mutex,
};

use network_interface::{NetworkInterface, NetworkInterfaceConfig};
use socket2::SockRef;
use tracing::{debug, trace};

use crate::{Error, UdpSocket};

pub struct MulticastUdpSocket {
    sock: UdpSocket,
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
            if !ipv6_is_link_local(ll) {
                return Err(Error::ProvidedLinkLocalAddrIsntLinkLocal);
            }
        }
        if ipv6_is_link_local(ipv6_site_local) {
            return Err(Error::ProvidedSiteLocalAddrIsLinkLocal);
        }
        let nics = network_interface::NetworkInterface::show()
            .into_iter()
            .flatten()
            .collect::<Vec<_>>();
        if nics.is_empty() {
            return Err(Error::NoNics);
        }
        let sock = UdpSocket::bind_udp((Ipv6Addr::UNSPECIFIED, port).into(), true)?;
        let sock = Self {
            sock,
            ipv4_addr,
            ipv6_link_local,
            ipv6_site_local,
            nics,
        };
        sock.bind_multicast()?;
        Ok(sock)
    }

    fn bind_multicast(&self) -> crate::Result<()> {
        let mut joined = try_join_v4(&self.sock, self.ipv4_addr, Ipv4Addr::UNSPECIFIED);

        for nic in self.nics.iter() {
            let mut has_link_local = false;
            let mut has_site_local = false;

            for addr in nic.addr.iter() {
                match addr.ip() {
                    IpAddr::V4(iface_addr)
                        if iface_addr.is_private() && !iface_addr.is_loopback() =>
                    {
                        joined |= try_join_v4(&self.sock, self.ipv4_addr, iface_addr);
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
                joined |= try_join_v6(&self.sock, self.ipv6_site_local, nic.index);
            }

            if let Some(ll) = self.ipv6_link_local {
                if has_link_local {
                    joined |= try_join_v6(&self.sock, ll, nic.index);
                }
            }
        }

        if !joined {
            return Err(Error::MulticastJoinFail);
        }

        Ok(())
    }

    pub async fn try_send_mcast_everywhere(
        &self,
        get_payload: &impl Fn(&MulticastOpts) -> bstr::BString,
    ) {
        let sent = Mutex::new(HashSet::new());
        let sent = &sent;

        let port = self.sock.bind_addr().port();

        let futs = self.nics.iter()
            .flat_map(|ni|
                ni.addr.iter().map(move |a| (ni.index, a.ip()))
            )
            .filter_map(|(ifidx, ifaddr)| {
                let ipv6_link_local = self.ipv6_link_local.filter(|_| matches!(ifaddr, IpAddr::V6(v6) if ipv6_is_link_local(v6)));
                let mcast_addr: SocketAddr = match (ifaddr, ipv6_link_local) {
                    (IpAddr::V4(a), _) if !a.is_loopback() && a.is_private() => {
                        (self.ipv4_addr, port).into()
                    }
                    (IpAddr::V6(a), Some(mlocal)) if !a.is_loopback() => {
                        SocketAddrV6::new(mlocal, port, 0, ifidx).into()
                    },
                    (IpAddr::V6(a), None) if !a.is_loopback() => {
                        SocketAddrV6::new(self.ipv6_site_local, port, 0, ifidx).into()
                    }
                    _ => {
                        trace!(oif_id=ifidx, addr=%ifaddr, "ignoring address");
                        return None
                    }
                };
                Some(MulticastOpts{
                    interface_addr: ifaddr,
                    interface_id: ifidx,
                    mcast_addr,
                })
            })
            .map(|opts| async move {
                let payload = get_payload(&opts);
                if !sent
                    .lock().unwrap()
                    .insert((payload.clone(), opts.interface_id, opts.mcast_addr))
                {
                    trace!(oif_id=opts.interface_id, addr=%opts.mcast_addr, "not sending duplicate payload");
                    return;
                }

                let sref = SockRef::from(self.sock.socket());
                match (opts.mcast_addr.is_ipv6(), opts.interface_addr) {
                    (true, IpAddr::V6(_)) => {
                        if let Err(e) = sref.set_multicast_if_v6(opts.interface_id) {
                            debug!(oif_id=opts.interface_id, "error calling set_multicast_if_v6: {e:#}");
                        }
                    }
                    (false, IpAddr::V4(iaddr)) => {
                        if let Err(e) = sref.set_multicast_if_v4(&iaddr) {
                            debug!(addr=%iaddr, "error calling set_multicast_if_v4: {e:#}");
                        }
                    }
                    _ => unreachable!()
                }

                match self.sock.send_to(payload.as_slice(), opts.mcast_addr).await {
                    Ok(sz) => trace!(addr=%opts.mcast_addr, oif_id=opts.interface_id, oif_addr=%opts.interface_addr, size=sz, payload=?payload, "sent"),
                    Err(e) => {
                        debug!(addr=%opts.mcast_addr, oif_id=opts.interface_id, oif_addr=%opts.interface_addr, payload=?payload, "error sending: {e:#}")
                    }
                };
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

pub struct MulticastOpts {
    interface_addr: IpAddr,
    #[allow(dead_code)]
    interface_id: u32,
    mcast_addr: SocketAddr,
}
