//! Transport layer: renet channel configuration, the netcode-over-UDP server transport and the
//! pieces shared with the client transport ([`UdpLink`], [`RateMeter`], [`TransportStats`]).
//!
//! We don't use `renet_netcode::NetcodeServerTransport` directly because it owns its
//! `UdpSocket` and gives no hook between the socket and netcode. Our transports are the same
//! ~100 lines of glue around `renetcode::NetcodeServer` / `NetcodeClient`, but send and receive
//! through a [`UdpLink`], which counts bytes and can run every datagram through the network
//! simulator ([`crate::netsim`]).
//!
//! **Pluggable transports (Steam, relays):** the server only talks to the
//! [`ServerTransport`] trait; a Steam networking or relay implementation would implement it
//! (renet's `renet_steam` crate has exactly this shape) and be passed to
//! [`crate::Server::with_transport`]. The client side mirrors this with
//! `client::net::ClientTransport`.

use std::collections::VecDeque;
use std::io;
use std::net::{SocketAddr, UdpSocket};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use renet::{ChannelConfig, ClientId, ConnectionConfig, RenetServer, SendType};
// Connection authentication/config types come from `renet_netcode` (which re-exports them from
// `renetcode`); the netcode.io state machine itself is driven directly so the socket glue below
// can insert the network simulator.
use renet_netcode::{NETCODE_USER_DATA_BYTES, ServerAuthentication, ServerConfig as NetcodeServerConfig};
use renetcode::{NETCODE_MAX_PACKET_BYTES, NetcodeServer, ServerResult};
use shared::protocol::{
    CHANNEL_RELIABLE, CHANNEL_UNRELIABLE, CONNECT_USER_DATA_BYTES, PROTOCOL_ID,
};

use crate::netsim::{NetSim, NetSimConfig};

const _: () = assert!(CONNECT_USER_DATA_BYTES == NETCODE_USER_DATA_BYTES);

/// Datagrams [`UdpLink::receive`] reads per call (per server / client update); the rest wait
/// in the socket buffer for the next update, so a packet flood can't stall the update loop
/// (and anything beyond the OS buffer is dropped by the OS, not queued by us).
pub const MAX_DATAGRAMS_PER_RECEIVE: usize = 4096;

/// renet channel layout shared by client and server (see `shared::protocol`).
pub fn connection_config() -> ConnectionConfig {
    let resend_time = Duration::from_millis(250);
    ConnectionConfig {
        available_bytes_per_tick: 256 * 1024,
        // Order = send priority: reliable events first, then snapshots.
        server_channels_config: vec![
            ChannelConfig {
                channel_id: CHANNEL_RELIABLE,
                // Level data can be large.
                max_memory_usage_bytes: 32 * 1024 * 1024,
                send_type: SendType::ReliableOrdered { resend_time },
            },
            ChannelConfig {
                channel_id: CHANNEL_UNRELIABLE,
                max_memory_usage_bytes: 4 * 1024 * 1024,
                send_type: SendType::Unreliable,
            },
        ],
        client_channels_config: vec![
            ChannelConfig {
                channel_id: CHANNEL_RELIABLE,
                max_memory_usage_bytes: 512 * 1024,
                send_type: SendType::ReliableOrdered { resend_time },
            },
            ChannelConfig {
                channel_id: CHANNEL_UNRELIABLE,
                max_memory_usage_bytes: 512 * 1024,
                send_type: SendType::Unreliable,
            },
        ],
    }
}

/// netcode wants wall-clock time (connect tokens carry expiry timestamps).
pub fn netcode_time() -> Duration {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
}

/// Bytes per second over a sliding window.
#[derive(Clone, Debug)]
pub struct RateMeter {
    window: Duration,
    now: Duration,
    samples: VecDeque<(Duration, u64)>,
    sum: u64,
}

impl Default for RateMeter {
    fn default() -> Self {
        Self::new(Duration::from_secs(1))
    }
}

impl RateMeter {
    pub fn new(window: Duration) -> Self {
        Self {
            window,
            now: Duration::ZERO,
            samples: VecDeque::new(),
            sum: 0,
        }
    }

    pub fn advance(&mut self, dt: Duration) {
        self.now += dt;
        while let Some(&(t, v)) = self.samples.front() {
            if self.now.saturating_sub(t) <= self.window {
                break;
            }
            self.sum -= v;
            self.samples.pop_front();
        }
    }

    pub fn add(&mut self, amount: u64) {
        self.sum += amount;
        match self.samples.back_mut() {
            Some((t, v)) if *t == self.now => *v += amount,
            _ => self.samples.push_back((self.now, amount)),
        }
    }

    /// Average per second over the window (or the elapsed time if shorter).
    pub fn per_second(&self) -> f64 {
        let span = self.window.min(self.now).as_secs_f64();
        if span <= 0.0 {
            0.0
        } else {
            self.sum as f64 / span
        }
    }
}

/// Byte/packet counters of a transport (UDP payload bytes, including netcode overhead).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TransportStats {
    pub bytes_sent: u64,
    pub bytes_received: u64,
    pub packets_sent: u64,
    pub packets_received: u64,
    pub bytes_sent_per_sec: f64,
    pub bytes_received_per_sec: f64,
    /// Packets dropped by the network simulator (outgoing + incoming).
    pub sim_dropped: u64,
    /// Packets passed through the network simulator (outgoing + incoming).
    pub sim_packets: u64,
}

/// A UDP socket with byte accounting and optional network simulation on both directions.
pub struct UdpLink {
    socket: UdpSocket,
    outgoing: Option<NetSim>,
    incoming: Option<NetSim>,
    stats: TransportStats,
    sent_rate: RateMeter,
    recv_rate: RateMeter,
    buffer: Box<[u8; NETCODE_MAX_PACKET_BYTES]>,
    /// The last receive stopped at its datagram limit.
    backlog: bool,
}

impl UdpLink {
    pub fn bind(addr: SocketAddr, netsim: Option<NetSimConfig>) -> io::Result<Self> {
        let socket = UdpSocket::bind(addr)?;
        socket.set_nonblocking(true)?;
        let netsim = netsim.filter(NetSimConfig::is_active).map(|mut c| {
            if c.seed == 0 {
                // Unseeded: derive from the clock and the socket so every link differs.
                let port = socket.local_addr().map_or(0, |a| a.port() as u64);
                c.seed = netcode_time().as_nanos() as u64 ^ (port << 48) | 1;
            }
            c
        });
        // Different (but reproducible) streams for the two directions.
        let make = |salt: u64| {
            netsim.map(|mut c| {
                c.seed ^= salt;
                NetSim::new(c)
            })
        };
        Ok(Self {
            socket,
            outgoing: make(0x0u64),
            incoming: make(0xA5A5_5A5A_1234_5678),
            stats: TransportStats::default(),
            sent_rate: RateMeter::default(),
            recv_rate: RateMeter::default(),
            buffer: Box::new([0; NETCODE_MAX_PACKET_BYTES]),
            backlog: false,
        })
    }

    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.socket.local_addr()
    }

    pub fn netsim(&self) -> Option<&NetSimConfig> {
        self.outgoing.as_ref().map(NetSim::config)
    }

    /// Advances clocks and sends outgoing packets whose simulated delay has elapsed.
    pub fn advance(&mut self, dt: Duration) {
        self.sent_rate.advance(dt);
        self.recv_rate.advance(dt);
        if let Some(sim) = &mut self.incoming {
            sim.advance(dt);
        }
        if let Some(sim) = &mut self.outgoing {
            sim.advance(dt);
        }
        self.flush();
    }

    /// Sends a datagram (through the simulator, if any).
    pub fn send(&mut self, addr: SocketAddr, data: &[u8]) {
        self.count_sent(data.len());
        match &mut self.outgoing {
            Some(sim) => sim.push(addr, data),
            None => self.raw_send(addr, data),
        }
    }

    /// Sends immediately, bypassing the simulator (disconnect packets on shutdown).
    pub fn send_now(&mut self, addr: SocketAddr, data: &[u8]) {
        self.count_sent(data.len());
        self.raw_send(addr, data);
    }

    fn count_sent(&mut self, len: usize) {
        self.stats.bytes_sent += len as u64;
        self.stats.packets_sent += 1;
        self.sent_rate.add(len as u64);
    }

    fn raw_send(&self, addr: SocketAddr, data: &[u8]) {
        if let Err(err) = self.socket.send_to(data, addr) {
            log::debug!("send to {addr} failed: {err}");
        }
    }

    /// Sends due simulated packets.
    pub fn flush(&mut self) {
        if let Some(sim) = &mut self.outgoing {
            while let Some((addr, data)) = sim.pop_due() {
                if let Err(err) = self.socket.send_to(&data, addr) {
                    log::debug!("send to {addr} failed: {err}");
                }
            }
        }
    }

    /// Reads the datagrams available on the socket (at most [`MAX_DATAGRAMS_PER_RECEIVE`];
    /// the rest stay queued for the next call) and appends those that are due (after
    /// simulation) to `out`.
    pub fn receive(&mut self, out: &mut Vec<(SocketAddr, Vec<u8>)>) -> io::Result<()> {
        self.receive_at_most(out, MAX_DATAGRAMS_PER_RECEIVE)
    }

    /// [`UdpLink::receive`] reading at most `max` datagrams from the socket: a flood can't
    /// keep the caller (the server's update) from returning.
    pub fn receive_at_most(
        &mut self,
        out: &mut Vec<(SocketAddr, Vec<u8>)>,
        max: usize,
    ) -> io::Result<()> {
        self.backlog = false;
        let mut read = 0;
        loop {
            if read >= max {
                // More may be waiting: the host loop should come back soon.
                self.backlog = true;
                break;
            }
            read += 1;
            match self.socket.recv_from(&mut self.buffer[..]) {
                Ok((len, addr)) => {
                    self.stats.bytes_received += len as u64;
                    self.stats.packets_received += 1;
                    self.recv_rate.add(len as u64);
                    match &mut self.incoming {
                        Some(sim) => sim.push(addr, &self.buffer[..len]),
                        None => out.push((addr, self.buffer[..len].to_vec())),
                    }
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => break,
                // Windows reports ICMP port unreachable from earlier sends this way.
                Err(e) if e.kind() == io::ErrorKind::ConnectionReset => continue,
                Err(e) => return Err(e),
            }
        }
        if let Some(sim) = &mut self.incoming {
            while let Some(packet) = sim.pop_due() {
                out.push(packet);
            }
        }
        Ok(())
    }

    /// True while simulated packets are in flight or the last receive stopped at its limit
    /// (callers should poll more often).
    pub fn has_pending(&self) -> bool {
        self.backlog
            || self.outgoing.as_ref().is_some_and(|s| s.pending() > 0)
            || self.incoming.as_ref().is_some_and(|s| s.pending() > 0)
    }

    pub fn stats(&self) -> TransportStats {
        let mut s = self.stats;
        s.bytes_sent_per_sec = self.sent_rate.per_second();
        s.bytes_received_per_sec = self.recv_rate.per_second();
        for sim in [&self.outgoing, &self.incoming].into_iter().flatten() {
            s.sim_dropped += sim.stats().dropped;
            s.sim_packets += sim.stats().packets;
        }
        s
    }
}

/// What the server needs from a transport. Implemented by [`NetcodeServerTransport`] (UDP);
/// other implementations (Steam networking, relays) plug in through
/// [`crate::Server::with_transport`].
pub trait ServerTransport: Send {
    /// Advances the transport clock, receives packets and feeds them to `server`.
    fn update(&mut self, dt: Duration, server: &mut RenetServer) -> anyhow::Result<()>;
    /// Sends everything renet has queued.
    fn send_packets(&mut self, server: &mut RenetServer);
    /// Disconnects everyone immediately (shutdown).
    fn disconnect_all(&mut self, server: &mut RenetServer);
    /// Address clients connect to (IP transports only).
    fn local_addr(&self) -> Option<SocketAddr>;
    /// Opaque data the client provided when connecting (see
    /// [`shared::protocol::ConnectRequest`]).
    fn user_data(&self, client_id: ClientId) -> Option<[u8; CONNECT_USER_DATA_BYTES]>;
    fn stats(&self) -> TransportStats;
    /// True while the network simulator holds packets (the host loop should poll often).
    fn has_pending(&self) -> bool {
        false
    }
}

/// netcode (Unsecure) over UDP, with optional network simulation.
pub struct NetcodeServerTransport {
    link: UdpLink,
    netcode: NetcodeServer,
    scratch: Vec<(SocketAddr, Vec<u8>)>,
}

impl NetcodeServerTransport {
    pub fn new(
        bind: SocketAddr,
        max_clients: usize,
        netsim: Option<NetSimConfig>,
    ) -> anyhow::Result<Self> {
        let link =
            UdpLink::bind(bind, netsim).map_err(|e| anyhow::anyhow!("binding UDP {bind}: {e}"))?;
        let local = link.local_addr()?;
        let netcode = NetcodeServer::new(NetcodeServerConfig {
            current_time: netcode_time(),
            max_clients: max_clients.clamp(1, 64),
            protocol_id: PROTOCOL_ID,
            public_addresses: vec![local],
            authentication: ServerAuthentication::Unsecure,
        });
        Ok(Self {
            link,
            netcode,
            scratch: Vec::new(),
        })
    }
}

fn handle_result(result: ServerResult, link: &mut UdpLink, server: &mut RenetServer) {
    match result {
        ServerResult::None => {}
        ServerResult::PacketToSend { payload, addr } => link.send(addr, payload),
        ServerResult::Payload { client_id, payload } => {
            if let Err(e) = server.process_packet_from(payload, client_id) {
                log::debug!("payload for unknown client {client_id}: {e}");
            }
        }
        ServerResult::ClientConnected {
            client_id,
            addr,
            payload,
            ..
        } => {
            server.add_connection(client_id);
            link.send(addr, payload);
        }
        ServerResult::ClientDisconnected {
            client_id,
            addr,
            payload,
        } => {
            server.remove_connection(client_id);
            if let Some(payload) = payload {
                link.send(addr, payload);
            }
        }
    }
}

impl ServerTransport for NetcodeServerTransport {
    fn update(&mut self, dt: Duration, server: &mut RenetServer) -> anyhow::Result<()> {
        self.link.advance(dt);
        self.netcode.update(dt);

        let mut packets = std::mem::take(&mut self.scratch);
        let received = self.link.receive(&mut packets);
        for (addr, data) in packets.iter_mut() {
            let result = self.netcode.process_packet(*addr, data);
            handle_result(result, &mut self.link, server);
        }
        packets.clear();
        self.scratch = packets;

        for client_id in self.netcode.clients_id() {
            let result = self.netcode.update_client(client_id);
            handle_result(result, &mut self.link, server);
        }
        for client_id in server.disconnections_id() {
            let result = self.netcode.disconnect(client_id);
            handle_result(result, &mut self.link, server);
        }
        self.link.flush();
        received.map_err(|e| anyhow::anyhow!("UDP receive: {e}"))
    }

    fn send_packets(&mut self, server: &mut RenetServer) {
        'clients: for client_id in server.clients_id() {
            let Ok(packets) = server.get_packets_to_send(client_id) else {
                continue;
            };
            for packet in packets {
                match self.netcode.generate_payload_packet(client_id, &packet) {
                    Ok((addr, payload)) => self.link.send(addr, payload),
                    Err(e) => {
                        log::warn!("failed to encrypt packet for client {client_id}: {e}");
                        continue 'clients;
                    }
                }
            }
        }
        self.link.flush();
    }

    fn disconnect_all(&mut self, server: &mut RenetServer) {
        for client_id in self.netcode.clients_id() {
            if let ServerResult::ClientDisconnected {
                client_id,
                addr,
                payload,
            } = self.netcode.disconnect(client_id)
            {
                server.remove_connection(client_id);
                if let Some(payload) = payload {
                    self.link.send_now(addr, payload);
                }
            }
        }
    }

    fn local_addr(&self) -> Option<SocketAddr> {
        self.link.local_addr().ok()
    }

    fn user_data(&self, client_id: ClientId) -> Option<[u8; CONNECT_USER_DATA_BYTES]> {
        self.netcode.user_data(client_id)
    }

    fn stats(&self) -> TransportStats {
        self.link.stats()
    }

    fn has_pending(&self) -> bool {
        self.link.has_pending()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rate_meter_window() {
        let mut m = RateMeter::new(Duration::from_secs(1));
        for _ in 0..20 {
            m.advance(Duration::from_millis(100));
            m.add(100);
        }
        // 10 samples of 100 bytes inside the 1 s window (plus the boundary one).
        let r = m.per_second();
        assert!((1000.0..=1100.0).contains(&r), "{r}");
        m.advance(Duration::from_secs(5));
        assert_eq!(m.per_second(), 0.0);
    }

    #[test]
    fn receive_reads_a_bounded_number_of_datagrams() {
        // A flood used to be drained in one go, so `Server::update` might never return.
        let mut link = UdpLink::bind("127.0.0.1:0".parse().unwrap(), None).unwrap();
        let addr = link.local_addr().unwrap();
        let sender = UdpSocket::bind("127.0.0.1:0").unwrap();
        for i in 0..20u8 {
            sender.send_to(&[i], addr).unwrap();
        }
        let mut out = Vec::new();
        let mut calls = Vec::new();
        let start = std::time::Instant::now();
        while out.len() < 20 && start.elapsed() < Duration::from_secs(2) {
            let before = out.len();
            link.receive_at_most(&mut out, 8).unwrap();
            if out.len() > before {
                calls.push((out.len() - before, link.has_pending()));
            }
        }
        // 8 + 8 + 4 (on loopback everything is there at once; allow slower delivery).
        assert_eq!(out.len(), 20);
        assert!(calls.iter().all(|&(n, _)| n <= 8), "{calls:?}");
        assert_eq!(calls[0], (8, true), "stopped at the limit, more pending");
        let data: Vec<u8> = out.iter().map(|(_, d)| d[0]).collect();
        assert_eq!(data, (0..20).collect::<Vec<u8>>(), "nothing lost or reordered");
        link.receive(&mut out).unwrap();
        assert!(!link.has_pending());
    }
}
