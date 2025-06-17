use std::net::{Ipv4Addr, Ipv6Addr};

pub const SSDP_PORT: u16 = 1900;
pub const SSDP_MCAST_IPV4: Ipv4Addr = Ipv4Addr::new(239, 255, 255, 250);
pub const SSDP_MCAST_IPV6_LINK_LOCAL: Ipv6Addr = Ipv6Addr::new(0xff02, 0, 0, 0, 0, 0, 0, 0xc);
pub const SSDP_MCAST_IPV6_SITE_LOCAL: Ipv6Addr = Ipv6Addr::new(0xff05, 0, 0, 0, 0, 0, 0, 0xc);
