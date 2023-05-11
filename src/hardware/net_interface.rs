//! Smoltcp network storage and configuration

use crate::BoosterSettings;
use smoltcp_nal::smoltcp;

use super::SmoltcpDevice;

/// The number of TCP sockets supported in the network stack.
const NUM_TCP_SOCKETS: usize = 4;

/// Containers for smoltcp-related network configurations
struct NetStorage {
    // Note: There is an additional socket set item required for the DHCP socket.
    pub sockets: [smoltcp::iface::SocketStorage<'static>; NUM_TCP_SOCKETS + 1],
    pub tcp_socket_storage: [TcpSocketStorage; NUM_TCP_SOCKETS],
}

impl NetStorage {
    const fn new() -> Self {
        NetStorage {
            sockets: [smoltcp::iface::SocketStorage::EMPTY; NUM_TCP_SOCKETS + 1],
            tcp_socket_storage: [TcpSocketStorage::new(); NUM_TCP_SOCKETS],
        }
    }
}

#[derive(Copy, Clone)]
struct TcpSocketStorage {
    rx_storage: [u8; 1024],

    // Note that TX storage is set to 4096 to ensure that it is sufficient to contain full
    // telemetry messages for all 8 RF channels.
    tx_storage: [u8; 4096],
}

impl TcpSocketStorage {
    const fn new() -> Self {
        Self {
            tx_storage: [0; 4096],
            rx_storage: [0; 1024],
        }
    }
}

/// Set up the network interface.
///
/// # Note
/// This function may only be called exactly once.
///
/// # Args
/// * `device` - The smoltcp interface device.
/// * `settings` - The device settings to use.
pub fn setup(
    device: &mut SmoltcpDevice,
    settings: &BoosterSettings,
) -> (
    smoltcp::iface::Interface,
    smoltcp::iface::SocketSet<'static>,
) {
    let net_store = cortex_m::singleton!(: NetStorage = NetStorage::new()).unwrap();

    let ip_address = settings.ip_address();

    let mut config = smoltcp::iface::Config::default();
    config
        .hardware_addr
        .replace(smoltcp::wire::HardwareAddress::Ethernet(settings.mac()));

    let mut interface = smoltcp::iface::Interface::new(config, device);

    interface
        .routes_mut()
        .add_default_ipv4_route(settings.gateway())
        .unwrap();

    let mut sockets = smoltcp::iface::SocketSet::new(&mut net_store.sockets[..]);
    for storage in net_store.tcp_socket_storage[..].iter_mut() {
        let tcp_socket = {
            let rx_buffer = smoltcp::socket::tcp::SocketBuffer::new(&mut storage.rx_storage[..]);
            let tx_buffer = smoltcp::socket::tcp::SocketBuffer::new(&mut storage.tx_storage[..]);

            smoltcp::socket::tcp::Socket::new(rx_buffer, tx_buffer)
        };

        sockets.add(tcp_socket);
    }

    if ip_address.address().is_unspecified() {
        sockets.add(smoltcp::socket::dhcpv4::Socket::new());
    } else {
        interface.update_ip_addrs(|addrs| addrs.push(ip_address).unwrap());
    }

    (interface, sockets)
}
