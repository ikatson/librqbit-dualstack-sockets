use std::net::{SocketAddr, SocketAddrV6};

pub trait TryToV4 {
    fn try_to_ipv4(&self) -> SocketAddr;

    #[cfg(test)]
    fn to_v4(&self) -> SocketAddr {
        let a = self.try_to_ipv4();
        assert!(a.is_ipv4());
        a
    }
}

pub trait ToV6Mapped {
    fn to_ipv6_mapped(&self) -> SocketAddrV6;
}

impl ToV6Mapped for SocketAddr {
    fn to_ipv6_mapped(&self) -> SocketAddrV6 {
        match self {
            SocketAddr::V4(a) => SocketAddrV6::new(a.ip().to_ipv6_mapped(), a.port(), 0, 0),
            SocketAddr::V6(a) => *a,
        }
    }
}

impl TryToV4 for SocketAddr {
    fn try_to_ipv4(&self) -> SocketAddr {
        match self {
            SocketAddr::V4(_) => *self,
            SocketAddr::V6(a) => a.try_to_ipv4(),
        }
    }
}

impl TryToV4 for SocketAddrV6 {
    fn try_to_ipv4(&self) -> SocketAddr {
        self.ip()
            .to_ipv4_mapped()
            .map(|ip| SocketAddr::new(ip.into(), self.port()))
            .unwrap_or(SocketAddr::V6(*self))
    }
}
