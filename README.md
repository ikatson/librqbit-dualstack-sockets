# librqbit-dualstack-sockets

A library that provides dual-stack tokio sockets for use in [rqbit](https://github.com/ikatson/rqbit) torrent client.

It converts between SocketAddr addresses so that your app sees IPv4 (not IPv4-mapped IPv6) addresses.
