//! Joining by code through the rendezvous service (docs/rendezvous.md): a [`ClientTransport`]
//! that lets a `rendezvous::JoinAgent` find the host, then runs netcode over the first way
//! that works (the host's LAN address, its public address with hole punching, or the relay).
//!
//! One UDP socket carries everything: the service's messages, punches, and netcode's packets
//! (wrapped for the relay when that is the way). Each attempt gets its own netcode client
//! (with its own client id, which the host admits only because the service announced it).

use std::net::SocketAddr;
use std::time::Duration;

use rendezvous::join::{Attempt, JoinStep, PathKind};
use rendezvous::{Code, Inbound, JoinAgent, JoinConfig, Outbound};
use renet::RenetClient;
use renet_netcode::ClientAuthentication;
use renetcode::NetcodeClient;
use server::{NetSimConfig, TransportStats, UdpLink};
use shared::protocol::{CONNECT_USER_DATA_BYTES, PROTOCOL_ID, PROTOCOL_VERSION};

use super::transport::{ClientTransport, disconnect_text};

/// What the connecting screen says while a code join is under way.
pub fn step_text(step: &JoinStep) -> String {
    match step {
        JoinStep::Contacting => "CONTACTING THE ONLINE SERVICE".into(),
        JoinStep::Joining => "LOOKING UP THE GAME".into(),
        JoinStep::Trying(PathKind::Lan) => "CONNECTING OVER THE LAN".into(),
        JoinStep::Trying(PathKind::Direct) => "CONNECTING DIRECTLY".into(),
        JoinStep::Trying(PathKind::Relay) => "CONNECTING THROUGH THE RELAY".into(),
        JoinStep::Connected(PathKind::Relay) => "CONNECTED THROUGH THE RELAY".into(),
        JoinStep::Connected(_) => "CONNECTED".into(),
        JoinStep::Failed(e) => e.to_uppercase(),
    }
}

pub struct OnlineClientTransport {
    link: UdpLink,
    agent: JoinAgent,
    clock: Duration,
    user_data: [u8; CONNECT_USER_DATA_BYTES],
    netcode: Option<(Attempt, NetcodeClient)>,
    connected: bool,
    scratch: Vec<(SocketAddr, Vec<u8>)>,
    out: Vec<(SocketAddr, Vec<u8>)>,
}

impl OnlineClientTransport {
    /// `client_ids`: three random netcode client ids (LAN, direct, relay attempts).
    pub fn new(
        service: &str,
        code: Code,
        relay_only: bool,
        client_ids: [u64; 3],
        user_data: [u8; CONNECT_USER_DATA_BYTES],
        netsim: Option<NetSimConfig>,
    ) -> anyhow::Result<Self> {
        let link = UdpLink::bind(SocketAddr::from(([0, 0, 0, 0], 0)), netsim)?;
        let lan = match (rendezvous::local_lan_ip(), link.local_addr()) {
            (Some(ip), Ok(local)) => Some(SocketAddr::new(ip, local.port())),
            _ => None,
        };
        let agent = JoinAgent::new(JoinConfig {
            service: service.to_string(),
            code,
            protocol: PROTOCOL_VERSION,
            client_ids,
            lan,
            relay_only,
        });
        Ok(Self {
            link,
            agent,
            clock: Duration::ZERO,
            user_data,
            netcode: None,
            connected: false,
            scratch: Vec::new(),
            out: Vec::new(),
        })
    }

    /// Where the join is at.
    pub fn step(&self) -> JoinStep {
        self.agent.step()
    }

    fn send_netcode(&mut self, addr: SocketAddr, payload: &[u8]) {
        match self.agent.outbound(addr, payload) {
            Outbound::Direct => self.link.send(addr, payload),
            Outbound::Relay(service, bytes) => self.link.send(service, &bytes),
            Outbound::Drop => {}
        }
    }

    /// A netcode client for the agent's current attempt (a new one when it changes).
    fn follow_attempt(&mut self) -> Result<(), String> {
        let want = self.agent.attempt();
        if self.netcode.as_ref().map(|(a, _)| *a) == want {
            return Ok(());
        }
        if let Some((_, mut old)) = self.netcode.take()
            && let Ok((addr, packet)) = old.disconnect()
        {
            let packet = packet.to_vec();
            self.send_netcode(addr, &packet);
        }
        if let Some(attempt) = want {
            let client = NetcodeClient::new(
                server::transport::netcode_time(),
                ClientAuthentication::Unsecure {
                    protocol_id: PROTOCOL_ID,
                    client_id: attempt.client_id,
                    server_addr: attempt.addr,
                    user_data: Some(self.user_data),
                },
            )
            .map_err(|e| format!("netcode: {e}"))?;
            self.netcode = Some((attempt, client));
        }
        Ok(())
    }
}

impl ClientTransport for OnlineClientTransport {
    fn update(&mut self, dt: Duration, client: &mut RenetClient) -> Result<(), String> {
        self.link.advance(dt);
        self.clock += dt;
        let now = self.clock;
        if let Some(reason) = self.agent.failure() {
            client.disconnect_due_to_transport();
            return Err(reason.to_string());
        }
        if let Some(reason) = self
            .netcode
            .as_ref()
            .and_then(|(_, n)| n.disconnect_reason())
        {
            if self.connected {
                client.disconnect_due_to_transport();
                return Err(disconnect_text(reason));
            }
            // This way didn't work (refused, timed out): the agent tries the next one.
            log::info!(
                "online join: {:?} attempt failed: {}",
                self.netcode.as_ref().map(|(a, _)| a.kind),
                disconnect_text(reason)
            );
            self.netcode = None;
            self.agent.attempt_failed(now);
        }
        if let Some(reason) = client.disconnect_reason() {
            self.disconnect();
            return Err(reason.to_string());
        }

        let mut packets = std::mem::take(&mut self.scratch);
        let received = self.link.receive(&mut packets);
        for (addr, data) in packets.iter_mut() {
            let from = match self.agent.on_receive(now, *addr, data) {
                Inbound::Netcode(from) => from,
                Inbound::Relayed { from, payload } => {
                    let payload = payload.to_vec();
                    *data = payload;
                    from
                }
                Inbound::Drop => continue,
            };
            if let Some((attempt, netcode)) = &mut self.netcode
                && attempt.addr == from
                && let Some(payload) = netcode.process_packet(data)
            {
                client.process_packet(payload);
            }
        }
        packets.clear();
        self.scratch = packets;

        let mut out = std::mem::take(&mut self.out);
        self.agent.poll(now, &mut out);
        for (to, data) in out.drain(..) {
            self.link.send(to, &data);
        }
        self.out = out;
        self.follow_attempt()?;

        if let Some((attempt, netcode)) = &mut self.netcode {
            if netcode.is_connected() {
                if !self.connected {
                    self.connected = true;
                    self.agent.connected();
                    log::info!("online join: connected ({:?})", attempt.kind);
                }
                client.set_connected();
            } else if netcode.is_connecting() {
                client.set_connecting();
            }
            if let Some((packet, addr)) = netcode.update(dt) {
                let packet = packet.to_vec();
                self.send_netcode(addr, &packet);
            }
        }
        self.link.flush();
        received.map_err(|e| format!("UDP receive: {e}"))
    }

    fn send_packets(&mut self, client: &mut RenetClient) -> Result<(), String> {
        let Some((_, netcode)) = &mut self.netcode else {
            return Ok(());
        };
        if let Some(reason) = netcode.disconnect_reason() {
            return if self.connected {
                Err(disconnect_text(reason))
            } else {
                Ok(())
            };
        }
        if !netcode.is_connected() {
            return Ok(());
        }
        let mut packets = Vec::new();
        for packet in client.get_packets_to_send() {
            match netcode.generate_payload_packet(&packet) {
                Ok((addr, payload)) => packets.push((addr, payload.to_vec())),
                Err(e) => return Err(format!("netcode: {e}")),
            }
        }
        for (addr, payload) in packets {
            self.send_netcode(addr, &payload);
        }
        self.link.flush();
        Ok(())
    }

    fn disconnect(&mut self) {
        let Some((_, netcode)) = &mut self.netcode else {
            return;
        };
        if netcode.is_disconnected() {
            return;
        }
        if let Ok((addr, packet)) = netcode.disconnect() {
            let packet = packet.to_vec();
            match self.agent.outbound(addr, &packet) {
                Outbound::Direct => self.link.send_now(addr, &packet),
                Outbound::Relay(service, bytes) => self.link.send_now(service, &bytes),
                Outbound::Drop => {}
            }
        }
    }

    fn is_connected(&self) -> bool {
        self.netcode.as_ref().is_some_and(|(_, n)| n.is_connected())
    }

    fn server_addr(&self) -> Option<SocketAddr> {
        self.netcode
            .as_ref()
            .filter(|(a, _)| a.kind != PathKind::Relay)
            .map(|(a, _)| a.addr)
    }

    fn stats(&self) -> TransportStats {
        self.link.stats()
    }

    fn progress(&self) -> Option<String> {
        Some(step_text(&self.agent.step()))
    }
}
