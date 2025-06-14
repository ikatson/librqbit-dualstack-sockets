use std::net::SocketAddr;

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("error creating socket: {0}")]
    SocketNew(std::io::Error),
    #[error("error binding to {addr}: {source}")]
    Bind {
        addr: SocketAddr,
        source: std::io::Error,
    },
    #[error("error setting only_v6={value}: {source}")]
    OnlyV6 { value: bool, source: std::io::Error },
    #[error("error setting SO_REUSEADDR: {0}")]
    ReuseAddress(std::io::Error),
    #[error("error getting local_addr(): {0}")]
    LocalAddr(std::io::Error),
    #[error("as_socket() returned None")]
    AsSocket,
    #[error("error setting nonblocking=true: {0}")]
    SetNonblocking(std::io::Error),
    #[error("mismatch between local_addr({local_addr:?}) and requested bind_addr({bind_addr:?})")]
    LocalBindAddrMismatch {
        bind_addr: SocketAddr,
        local_addr: SocketAddr,
    },
    #[error("error listening")]
    Listen(std::io::Error),
    #[error("error calling tokio from_std")]
    TokioFromStd(std::io::Error),
}

pub type Result<T> = core::result::Result<T, Error>;
