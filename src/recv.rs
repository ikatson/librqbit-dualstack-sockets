//! Error policy for datagram receive loops.
//!
//! A task that reads a UDP socket in a loop is usually the only reader of
//! that socket, so if it stops on a receive error the service behind it (DHT,
//! uTP, local discovery...) is gone until restart. Receive errors come in two
//! kinds, handled here in one place:
//!
//! - errors about a single datagram, or about an earlier send. The socket is
//!   fine and the next receive works. [`is_per_datagram_error`] tells them
//!   apart, and `UdpSocket::recv_from` skips them.
//! - everything else (e.g. the kernel being out of memory). Those are
//!   returned to the caller.
//!
//! On Windows, sockets are also told not to report ICMP errors in the first
//! place; see [`disable_udp_reset_errors`].

use std::io;

/// A datagram didn't fit into the buffer. Unix truncates it silently instead.
#[cfg(windows)]
const WSAEMSGSIZE: i32 = 10040;

/// Returns true if a receive error says nothing about the health of the
/// socket, so that it can be read again right away.
///
/// These are the errors caused by one incoming datagram, or by an ICMP reply
/// to a datagram we sent earlier (Windows reports those on unconnected
/// sockets too, the others only with `IP_RECVERR` or on connected sockets).
pub(crate) fn is_per_datagram_error(e: &io::Error) -> bool {
    use io::ErrorKind::*;

    if matches!(
        e.kind(),
        Interrupted | ConnectionReset | ConnectionRefused | HostUnreachable | NetworkUnreachable
    ) {
        return true;
    }

    #[cfg(windows)]
    if e.raw_os_error() == Some(WSAEMSGSIZE) {
        return true;
    }

    false
}

/// Tells a UDP socket not to fail receives because of ICMP errors caused by
/// earlier sends, as Windows does by default even on unconnected sockets:
/// "port unreachable" surfaces as `WSAECONNRESET`, and "TTL expired" as
/// `WSAENETRESET`. They say nothing useful about an unconnected socket, and
/// every other platform never reports them. Those errors are skipped on
/// receive anyway; this stops them at the source.
///
/// Failing to set this is not fatal (Wine, for one, doesn't implement all of
/// it), so this only logs.
#[cfg(windows)]
pub(crate) fn disable_udp_reset_errors(sock: &socket2::Socket) {
    use std::os::windows::io::AsRawSocket;

    use windows_sys::Win32::Networking::WinSock::{
        SIO_UDP_CONNRESET, SIO_UDP_NETRESET, SOCKET_ERROR, WSAIoctl,
    };

    for (name, code) in [
        ("SIO_UDP_CONNRESET", SIO_UDP_CONNRESET),
        ("SIO_UDP_NETRESET", SIO_UDP_NETRESET),
    ] {
        let mut enabled: u32 = 0; // a BOOL: FALSE
        let mut bytes_returned: u32 = 0;
        // SAFETY: the input buffer is a live u32 of the declared size, the
        // output buffer is empty, and no overlapped IO is requested.
        let rc = unsafe {
            WSAIoctl(
                sock.as_raw_socket() as _,
                code,
                (&raw mut enabled).cast(),
                size_of::<u32>() as u32,
                std::ptr::null_mut(),
                0,
                &raw mut bytes_returned,
                std::ptr::null_mut(),
                None,
            )
        };
        if rc == SOCKET_ERROR {
            tracing::debug!("couldn't disable {name}: {:#}", io::Error::last_os_error());
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Error, ErrorKind};

    use super::is_per_datagram_error;

    #[test]
    fn test_is_per_datagram_error() {
        for kind in [
            ErrorKind::Interrupted,
            ErrorKind::ConnectionReset,
            ErrorKind::ConnectionRefused,
            ErrorKind::HostUnreachable,
            ErrorKind::NetworkUnreachable,
        ] {
            assert!(is_per_datagram_error(&Error::from(kind)), "{kind:?}");
        }

        for kind in [
            ErrorKind::OutOfMemory,
            ErrorKind::NetworkDown,
            ErrorKind::NotConnected,
            ErrorKind::Other,
        ] {
            assert!(!is_per_datagram_error(&Error::from(kind)), "{kind:?}");
        }

        #[cfg(windows)]
        {
            assert!(is_per_datagram_error(&Error::from_raw_os_error(
                super::WSAEMSGSIZE
            )));
        }
    }
}
