//! Client transport: netcode over UDP with optional network simulation.
//!
//! [`ClientTransport`] is the seam for other transports (Steam networking; joining by code
//! through the rendezvous service is `super::online`): a [`super::NetClient`] only calls these
//! methods. [`NetcodeClientTransport`] mirrors
//! `renet_netcode::NetcodeClientTransport` but sends/receives through a
//! [`server::UdpLink`], which counts bytes and runs the network simulator.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::time::Duration;

use renet::RenetClient;
// Authentication types from `renet_netcode`; the netcode.io client is driven directly so the
// network simulator can sit between it and the socket.
use renet_netcode::ClientAuthentication;
use renetcode::{DisconnectReason, NetcodeClient};
use server::{NetSimConfig, TransportStats, UdpLink};
use shared::protocol::{CONNECT_USER_DATA_BYTES, PROTOCOL_ID};

/// What [`super::NetClient`] needs from a transport.
pub trait ClientTransport: Send {
    /// Advances the transport clock and feeds received packets into `client`. An error means
    /// the connection is gone (the message is the reason).
    fn update(&mut self, dt: Duration, client: &mut RenetClient) -> Result<(), String>;
    /// Sends everything renet has queued.
    fn send_packets(&mut self, client: &mut RenetClient) -> Result<(), String>;
    /// Tells the server we're leaving (sent immediately).
    fn disconnect(&mut self);
    /// The transport-level connection is established.
    fn is_connected(&self) -> bool;
    fn server_addr(&self) -> Option<SocketAddr>;
    fn stats(&self) -> TransportStats;
    /// What it is doing while connecting, for the connecting screen (joining by code).
    fn progress(&self) -> Option<String> {
        None
    }
}

/// A player-facing reason for a netcode disconnect (shown after "Couldn't connect: " /
/// "Disconnected: " in the main menu).
pub fn disconnect_text(reason: DisconnectReason) -> String {
    match reason {
        DisconnectReason::ConnectionRequestTimedOut
        | DisconnectReason::ConnectionResponseTimedOut
        | DisconnectReason::ConnectTokenExpired => {
            "no answer from the server (check the address, that the game is hosted and that \
             its UDP port is open)"
                .into()
        }
        DisconnectReason::ConnectionTimedOut => "lost the connection to the server".into(),
        DisconnectReason::ConnectionDenied => "the server refused the connection".into(),
        DisconnectReason::DisconnectedByServer => "the server closed the connection".into(),
        DisconnectReason::DisconnectedByClient => "left the server".into(),
    }
}

/// netcode (Unsecure) over UDP.
pub struct NetcodeClientTransport {
    link: UdpLink,
    netcode: NetcodeClient,
    scratch: Vec<(SocketAddr, Vec<u8>)>,
}

impl NetcodeClientTransport {
    /// `client_id` must be unique per connection (random); `user_data` is the
    /// [`shared::protocol::ConnectRequest`].
    pub fn new(
        server_addr: SocketAddr,
        client_id: u64,
        user_data: [u8; CONNECT_USER_DATA_BYTES],
        netsim: Option<NetSimConfig>,
    ) -> anyhow::Result<Self> {
        let bind_ip: IpAddr = match server_addr {
            SocketAddr::V4(a) if a.ip().is_loopback() => Ipv4Addr::LOCALHOST.into(),
            SocketAddr::V4(_) => Ipv4Addr::UNSPECIFIED.into(),
            SocketAddr::V6(a) if a.ip().is_loopback() => Ipv6Addr::LOCALHOST.into(),
            SocketAddr::V6(_) => Ipv6Addr::UNSPECIFIED.into(),
        };
        let link = UdpLink::bind(SocketAddr::new(bind_ip, 0), netsim)?;
        let netcode = NetcodeClient::new(
            server::transport::netcode_time(),
            ClientAuthentication::Unsecure {
                protocol_id: PROTOCOL_ID,
                client_id,
                server_addr,
                user_data: Some(user_data),
            },
        )
        .map_err(|e| anyhow::anyhow!("netcode: {e}"))?;
        Ok(Self {
            link,
            netcode,
            scratch: Vec::new(),
        })
    }

    pub fn local_addr(&self) -> Option<SocketAddr> {
        self.link.local_addr().ok()
    }
}

impl ClientTransport for NetcodeClientTransport {
    fn update(&mut self, dt: Duration, client: &mut RenetClient) -> Result<(), String> {
        self.link.advance(dt);
        if let Some(reason) = self.netcode.disconnect_reason() {
            client.disconnect_due_to_transport();
            return Err(disconnect_text(reason));
        }
        if let Some(reason) = client.disconnect_reason() {
            if let Ok((addr, packet)) = self.netcode.disconnect() {
                self.link.send_now(addr, packet);
            }
            return Err(reason.to_string());
        }
        if self.netcode.is_connected() {
            client.set_connected();
        } else if self.netcode.is_connecting() {
            client.set_connecting();
        }

        let mut packets = std::mem::take(&mut self.scratch);
        let received = self.link.receive(&mut packets);
        let server_addr = self.netcode.server_addr();
        for (addr, data) in packets.iter_mut() {
            if *addr != server_addr {
                continue;
            }
            if let Some(payload) = self.netcode.process_packet(data) {
                client.process_packet(payload);
            }
        }
        packets.clear();
        self.scratch = packets;

        if let Some((packet, addr)) = self.netcode.update(dt) {
            self.link.send(addr, packet);
        }
        self.link.flush();
        received.map_err(|e| format!("UDP receive: {e}"))
    }

    fn send_packets(&mut self, client: &mut RenetClient) -> Result<(), String> {
        if let Some(reason) = self.netcode.disconnect_reason() {
            return Err(disconnect_text(reason));
        }
        if !self.netcode.is_connected() {
            return Ok(());
        }
        for packet in client.get_packets_to_send() {
            match self.netcode.generate_payload_packet(&packet) {
                Ok((addr, payload)) => self.link.send(addr, payload),
                Err(e) => return Err(format!("netcode: {e}")),
            }
        }
        self.link.flush();
        Ok(())
    }

    fn disconnect(&mut self) {
        if self.netcode.is_disconnected() {
            return;
        }
        if let Ok((addr, packet)) = self.netcode.disconnect() {
            self.link.send_now(addr, packet);
        }
    }

    fn is_connected(&self) -> bool {
        self.netcode.is_connected()
    }

    fn server_addr(&self) -> Option<SocketAddr> {
        Some(self.netcode.server_addr())
    }

    fn stats(&self) -> TransportStats {
        self.link.stats()
    }
}
