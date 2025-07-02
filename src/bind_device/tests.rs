use std::net::SocketAddr;

use network_interface::{NetworkInterface, NetworkInterfaceConfig};

use crate::{BindDevice, ConnectOpts, tcp_connect};

fn find_localhost_name() -> String {
    let nics = NetworkInterface::show().unwrap();
    nics.into_iter()
        .find(|nic| nic.addr.iter().any(|a| a.ip().is_loopback()))
        .map(|nic| nic.name)
        .expect("expected to find loopback interface")
}

#[tokio::test]
async fn test_bind_to_device() {
    let bd_name = find_localhost_name();
    println!("localhost interface name: {bd_name}");
    let bd = BindDevice::new_from_name(&bd_name).expect("expected to create BindDevice");
    println!("bd: {bd:?}");
    let test_addr: SocketAddr = "1.1.1.1:80".parse().unwrap();
    drop(
        tcp_connect(test_addr, ConnectOpts::default())
            .await
            .expect("expected to connect without BD"),
    );

    let res = tcp_connect(
        test_addr,
        ConnectOpts {
            bind_device: Some(&bd),
            ..Default::default()
        },
    )
    .await;

    match &res {
        Ok(_) => panic!("expected an error"),
        Err(e) => {
            println!("error: {e:#}");
        }
    }
}
