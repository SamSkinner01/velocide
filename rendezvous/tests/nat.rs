//! The service, a host and joiners on a simulated network with NAT routers (no sockets, fake
//! time). Netcode is stood in for by a CONNECT/ACCEPT/DENY exchange through the agents, so
//! the tests show which ways reach the host, that the gate keeps strangers out, and how
//! restarts, bans and lost packets are handled.

use std::collections::{HashMap, HashSet};
use std::net::{IpAddr, SocketAddr};
use std::time::Duration;

use rendezvous::join::{Attempt, JoinStep, PathKind};
use rendezvous::proto::{self, HostInfo, Message};
use rendezvous::{
    Code, HostAgent, HostConfig, HostStatus, Inbound, JoinAgent, JoinConfig, Outbound, Service,
    ServiceConfig,
};

const SERVICE: &str = "203.0.113.1:47400";
const KEY: [u8; 32] = [7; 32];
const UNIX: u64 = 1_900_000_000;
const STEP: Duration = Duration::from_millis(10);
const LATENCY: Duration = Duration::from_millis(20);
const PROTOCOL: u32 = 11;

const CONNECT: u8 = 0x01;
const ACCEPT: u8 = 0x02;
const DENY: u8 = 0x03;
/// Game traffic once connected.
const PING: u8 = 0x04;
const PONG: u8 = 0x05;

fn addr(s: &str) -> SocketAddr {
    s.parse().unwrap()
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum NatKind {
    /// A public address, no NAT.
    Open,
    /// Endpoint-independent mapping and filtering.
    FullCone,
    /// Endpoint-independent mapping, filters by remote address and port (most home routers).
    PortRestricted,
    /// A mapping per destination (some carrier and corporate NATs): punching fails.
    Symmetric,
}

struct Nat {
    public: IpAddr,
    kind: NatKind,
    map: HashMap<(SocketAddr, Option<SocketAddr>), u16>,
    back: HashMap<u16, SocketAddr>,
    allowed: HashSet<(u16, SocketAddr)>,
    next_port: u16,
}

impl Nat {
    fn new(public: &str, kind: NatKind) -> Nat {
        Nat {
            public: public.parse().unwrap(),
            kind,
            map: HashMap::new(),
            back: HashMap::new(),
            allowed: HashSet::new(),
            next_port: 40000,
        }
    }

    fn outbound(&mut self, src: SocketAddr, dst: SocketAddr) -> SocketAddr {
        let key = (src, (self.kind == NatKind::Symmetric).then_some(dst));
        let port = match self.map.get(&key) {
            Some(p) => *p,
            None => {
                self.next_port += 7;
                self.map.insert(key, self.next_port);
                self.next_port
            }
        };
        self.back.insert(port, src);
        self.allowed.insert((port, dst));
        SocketAddr::new(self.public, port)
    }

    fn inbound(&self, port: u16, from: SocketAddr) -> Option<SocketAddr> {
        let private = *self.back.get(&port)?;
        match self.kind {
            NatKind::Open => None,
            NatKind::FullCone => Some(private),
            NatKind::PortRestricted | NatKind::Symmetric => {
                self.allowed.contains(&(port, from)).then_some(private)
            }
        }
    }
}

struct HostNode {
    agent: HostAgent,
    conns: HashMap<SocketAddr, u64>,
}

impl HostNode {
    fn receive(&mut self, now: Duration, from: SocketAddr, data: &[u8], out: &mut Out) {
        let (peer, payload) = match self.agent.on_receive(now, from, data) {
            Inbound::Netcode(f) => (f, data),
            Inbound::Relayed { from, payload } => (from, payload),
            Inbound::Drop => return,
        };
        if payload.first() == Some(&PING) {
            if self.conns.contains_key(&peer) {
                self.send(peer, &[PONG], out);
            }
            return;
        }
        let Some((&CONNECT, rest)) = payload.split_first() else {
            return;
        };
        let id = u64::from_le_bytes(rest.try_into().unwrap());
        let ok = if self.conns.get(&peer) == Some(&id) {
            true
        } else if self.agent.admit(id, peer) {
            self.conns.insert(peer, id);
            true
        } else {
            false
        };
        self.send(peer, &[if ok { ACCEPT } else { DENY }], out);
    }

    fn send(&self, to: SocketAddr, payload: &[u8], out: &mut Out) {
        match self.agent.outbound(to, payload) {
            Outbound::Direct => out.push((to, payload.to_vec())),
            Outbound::Relay(service, bytes) => out.push((service, bytes)),
            Outbound::Drop => {}
        }
    }

    fn code(&self) -> Option<Code> {
        match self.agent.status() {
            HostStatus::Online { code } => Some(*code),
            _ => None,
        }
    }
}

struct JoinNode {
    agent: JoinAgent,
    attempt: Option<Attempt>,
    last_connect: Duration,
    connected: Option<PathKind>,
    kinds_tried: Vec<PathKind>,
    pongs: u32,
}

impl JoinNode {
    fn receive(&mut self, now: Duration, from: SocketAddr, data: &[u8]) {
        let payload = match self.agent.on_receive(now, from, data) {
            Inbound::Netcode(_) => data,
            Inbound::Relayed { payload, .. } => payload,
            Inbound::Drop => return,
        };
        match payload.first() {
            Some(&ACCEPT) if self.connected.is_none() => {
                self.agent.connected();
                self.connected = self.attempt.map(|a| a.kind);
            }
            Some(&DENY) => self.agent.attempt_failed(now),
            Some(&PONG) => self.pongs += 1,
            _ => {}
        }
    }

    fn poll(&mut self, now: Duration, out: &mut Out) {
        self.agent.poll(now, out);
        let attempt = self.agent.attempt();
        if attempt != self.attempt {
            self.attempt = attempt;
            self.last_connect = Duration::ZERO;
            if let Some(a) = attempt {
                self.kinds_tried.push(a.kind);
            }
        }
        if let Some(a) = attempt
            && now.saturating_sub(self.last_connect) >= Duration::from_millis(100)
        {
            self.last_connect = now;
            let payload = if self.connected.is_some() {
                vec![PING]
            } else {
                let mut payload = vec![CONNECT];
                payload.extend_from_slice(&a.client_id.to_le_bytes());
                payload
            };
            match self.agent.outbound(a.addr, &payload) {
                Outbound::Direct => out.push((a.addr, payload)),
                Outbound::Relay(service, bytes) => out.push((service, bytes)),
                Outbound::Drop => {}
            }
        }
    }
}

/// Sends raw CONNECTs at a target (someone who knows the host's address but has no code).
struct Stranger {
    target: SocketAddr,
    accepted: bool,
    replies: usize,
}

enum Role {
    Host(Box<HostNode>),
    Joiner(Box<JoinNode>),
    Stranger(Stranger),
}

struct Node {
    private: SocketAddr,
    nat: Option<usize>,
    role: Role,
}

type Out = Vec<(SocketAddr, Vec<u8>)>;

struct Packet {
    at: Duration,
    from: SocketAddr,
    to: SocketAddr,
    data: Vec<u8>,
    /// Delivered on the sender's LAN (same NAT, private destination).
    lan: Option<usize>,
}

struct Net {
    now: Duration,
    service: Service,
    service_addr: SocketAddr,
    nats: Vec<Nat>,
    nodes: Vec<Node>,
    queue: Vec<Packet>,
    loss: u64,
    rng: u64,
    /// Delivered packets: (source as seen on the wire, destination).
    log: Vec<(SocketAddr, SocketAddr)>,
}

impl Net {
    fn new() -> Net {
        Net {
            now: Duration::from_secs(1),
            service: Service::new(&KEY, ServiceConfig::default(), Duration::from_secs(1)),
            service_addr: addr(SERVICE),
            nats: Vec::new(),
            nodes: Vec::new(),
            queue: Vec::new(),
            loss: 0,
            rng: 0x9e37_79b9_7f4a_7c15,
            log: Vec::new(),
        }
    }

    fn unix(&self) -> u64 {
        UNIX + self.now.as_secs()
    }

    fn nat(&mut self, public: &str, kind: NatKind) -> usize {
        self.nats.push(Nat::new(public, kind));
        self.nats.len() - 1
    }

    fn host_config(lan: Option<SocketAddr>) -> HostConfig {
        HostConfig {
            service: SERVICE.into(),
            info: HostInfo {
                name: "SAM'S GAME".into(),
                mode: 1,
                level: "knell_yard".into(),
                players: 1,
                max_players: 8,
                protocol: PROTOCOL,
                build: "build-42".into(),
                public: true,
                ..Default::default()
            },
            lan,
            gate: true,
            trust_loopback: true,
            trust_lan: false,
        }
    }

    fn add_host(&mut self, private: &str, nat: Option<usize>, config: HostConfig) -> usize {
        self.nodes.push(Node {
            private: addr(private),
            nat,
            role: Role::Host(Box::new(HostNode {
                agent: HostAgent::new(config),
                conns: HashMap::new(),
            })),
        });
        self.nodes.len() - 1
    }

    fn add_joiner(
        &mut self,
        private: &str,
        nat: Option<usize>,
        code: Code,
        relay_only: bool,
    ) -> usize {
        let private = addr(private);
        let ids = [
            rendezvous::crypto::random_u64(),
            rendezvous::crypto::random_u64(),
            rendezvous::crypto::random_u64(),
        ];
        self.nodes.push(Node {
            private,
            nat,
            role: Role::Joiner(Box::new(JoinNode {
                agent: JoinAgent::new(JoinConfig {
                    service: SERVICE.into(),
                    code,
                    protocol: PROTOCOL,
                    client_ids: ids,
                    lan: Some(private).filter(|a| a.ip().to_string().starts_with("192.168.")),
                    relay_only,
                }),
                attempt: None,
                last_connect: Duration::ZERO,
                connected: None,
                kinds_tried: Vec::new(),
                pongs: 0,
            })),
        });
        self.nodes.len() - 1
    }

    fn host(&mut self, i: usize) -> &mut HostNode {
        match &mut self.nodes[i].role {
            Role::Host(h) => h,
            _ => panic!("not a host"),
        }
    }

    fn joiner(&mut self, i: usize) -> &mut JoinNode {
        match &mut self.nodes[i].role {
            Role::Joiner(j) => j,
            _ => panic!("not a joiner"),
        }
    }

    fn random(&mut self) -> u64 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        self.rng
    }

    fn send(&mut self, node: usize, to: SocketAddr, data: Vec<u8>) {
        let (private, nat) = (self.nodes[node].private, self.nodes[node].nat);
        let at = self.now + LATENCY;
        if let Some(n) = nat {
            let same_lan = self
                .nodes
                .iter()
                .any(|other| other.nat == Some(n) && other.private == to);
            if same_lan {
                self.queue.push(Packet {
                    at,
                    from: private,
                    to,
                    data,
                    lan: Some(n),
                });
                return;
            }
            if to.ip() == self.nats[n].public {
                return; // no hairpinning
            }
            if to.ip().to_string().starts_with("192.168.") {
                return; // someone else's LAN
            }
            let from = self.nats[n].outbound(private, to);
            self.queue.push(Packet {
                at,
                from,
                to,
                data,
                lan: None,
            });
        } else {
            self.queue.push(Packet {
                at,
                from: private,
                to,
                data,
                lan: None,
            });
        }
    }

    fn deliver(&mut self, p: Packet) {
        if self.loss > 0 && self.random() % 100 < self.loss {
            return;
        }
        if p.to == self.service_addr {
            self.log.push((p.from, p.to));
            let mut out = Vec::new();
            let (now, unix) = (self.now, self.unix());
            self.service.handle(now, unix, p.from, &p.data, &mut out);
            for (to, data) in out {
                self.queue.push(Packet {
                    at: self.now + LATENCY,
                    from: self.service_addr,
                    to,
                    data,
                    lan: None,
                });
            }
            return;
        }
        let target = match p.lan {
            Some(n) => self
                .nodes
                .iter()
                .position(|node| node.nat == Some(n) && node.private == p.to),
            None => {
                let via_nat = self
                    .nats
                    .iter()
                    .position(|nat| nat.public == p.to.ip() && nat.kind != NatKind::Open)
                    .and_then(|n| {
                        let private = self.nats[n].inbound(p.to.port(), p.from)?;
                        self.nodes
                            .iter()
                            .position(|node| node.nat == Some(n) && node.private == private)
                    });
                via_nat.or_else(|| {
                    self.nodes
                        .iter()
                        .position(|node| node.nat.is_none() && node.private == p.to)
                })
            }
        };
        let Some(i) = target else {
            return;
        };
        self.log.push((p.from, p.to));
        let now = self.now;
        let mut out = Vec::new();
        match &mut self.nodes[i].role {
            Role::Host(h) => h.receive(now, p.from, &p.data, &mut out),
            Role::Joiner(j) => j.receive(now, p.from, &p.data),
            Role::Stranger(s) => {
                s.replies += 1;
                s.accepted |= p.data.first() == Some(&ACCEPT);
            }
        }
        for (to, data) in out {
            self.send(i, to, data);
        }
    }

    fn step(&mut self) {
        self.now += STEP;
        let now = self.now;
        let (due, later): (Vec<Packet>, Vec<Packet>) = std::mem::take(&mut self.queue)
            .into_iter()
            .partition(|p| p.at <= now);
        self.queue = later;
        for p in due {
            self.deliver(p);
        }
        let mut out = Vec::new();
        let unix = self.unix();
        self.service.tick(now, unix, &mut out);
        for (to, data) in out {
            self.queue.push(Packet {
                at: now + LATENCY,
                from: self.service_addr,
                to,
                data,
                lan: None,
            });
        }
        for i in 0..self.nodes.len() {
            let mut out = Vec::new();
            match &mut self.nodes[i].role {
                Role::Host(h) => h.agent.poll(now, &mut out),
                Role::Joiner(j) => j.poll(now, &mut out),
                Role::Stranger(s) => {
                    if (now.as_millis() / 10).is_multiple_of(10) {
                        let mut payload = vec![CONNECT];
                        payload.extend_from_slice(&12345u64.to_le_bytes());
                        out.push((s.target, payload));
                    }
                }
            }
            for (to, data) in out {
                self.send(i, to, data);
            }
        }
    }

    fn run(&mut self, d: Duration) {
        let end = self.now + d;
        while self.now < end {
            self.step();
        }
    }

    fn run_until(&mut self, limit: Duration, mut done: impl FnMut(&mut Net) -> bool) -> bool {
        let end = self.now + limit;
        while self.now < end {
            if done(self) {
                return true;
            }
            self.step();
        }
        done(self)
    }

    fn wait_online(&mut self, host: usize) -> Code {
        assert!(
            self.run_until(Duration::from_secs(5), |n| n.host(host).code().is_some()),
            "host never came online: {:?}",
            self.host(host).agent.status()
        );
        self.host(host).code().unwrap()
    }

    fn wait_joined(&mut self, joiner: usize) -> PathKind {
        let ok = self.run_until(Duration::from_secs(20), |n| {
            let j = n.joiner(joiner);
            j.connected.is_some() || matches!(j.agent.step(), JoinStep::Failed(_))
        });
        let j = self.joiner(joiner);
        assert!(ok, "join still running: {:?}", j.agent.step());
        match j.connected {
            Some(kind) => kind,
            None => panic!(
                "join failed: {:?} (tried {:?})",
                j.agent.step(),
                j.kinds_tried
            ),
        }
    }

    fn join_failure(&mut self, joiner: usize) -> String {
        self.run_until(Duration::from_secs(20), |n| {
            matches!(n.joiner(joiner).agent.step(), JoinStep::Failed(_))
        });
        match self.joiner(joiner).agent.step() {
            JoinStep::Failed(e) => e,
            other => panic!("didn't fail: {other:?}"),
        }
    }
}

#[test]
fn home_routers_connect_directly() {
    let mut net = Net::new();
    let a = net.nat("198.51.100.1", NatKind::PortRestricted);
    let b = net.nat("198.51.100.2", NatKind::PortRestricted);
    let host = net.add_host("192.168.1.10:47420", Some(a), Net::host_config(None));
    let code = net.wait_online(host);
    let joiner = net.add_joiner("192.168.2.20:50000", Some(b), code, false);
    assert_eq!(net.wait_joined(joiner), PathKind::Direct);
    assert_eq!(net.joiner(joiner).kinds_tried, vec![PathKind::Direct]);
}

#[test]
fn strict_nats_fall_back_to_the_relay() {
    let mut net = Net::new();
    let a = net.nat("198.51.100.1", NatKind::Symmetric);
    let b = net.nat("198.51.100.2", NatKind::Symmetric);
    let host = net.add_host("192.168.1.10:47420", Some(a), Net::host_config(None));
    let code = net.wait_online(host);
    let joiner = net.add_joiner("192.168.2.20:50000", Some(b), code, false);
    assert_eq!(net.wait_joined(joiner), PathKind::Relay);
    assert_eq!(
        net.joiner(joiner).kinds_tried,
        vec![PathKind::Direct, PathKind::Relay]
    );
    // The game keeps flowing through the relay.
    let before = net.joiner(joiner).pongs;
    net.run(Duration::from_secs(2));
    assert!(net.joiner(joiner).pongs > before + 10);
}

#[test]
fn an_open_host_and_a_strict_joiner_connect_directly() {
    let mut net = Net::new();
    let b = net.nat("198.51.100.2", NatKind::Symmetric);
    let host = net.add_host("198.51.100.9:47420", None, Net::host_config(None));
    let code = net.wait_online(host);
    let joiner = net.add_joiner("192.168.2.20:50000", Some(b), code, false);
    assert_eq!(net.wait_joined(joiner), PathKind::Direct);
}

#[test]
fn relay_only_hides_both_addresses() {
    let mut net = Net::new();
    let a = net.nat("198.51.100.1", NatKind::PortRestricted);
    let b = net.nat("198.51.100.2", NatKind::PortRestricted);
    let mut config = Net::host_config(None);
    config.info.relay_only = true;
    let host = net.add_host("192.168.1.10:47420", Some(a), config);
    let code = net.wait_online(host);
    let joiner = net.add_joiner("192.168.2.20:50000", Some(b), code, false);
    assert_eq!(net.wait_joined(joiner), PathKind::Relay);
    assert_eq!(net.joiner(joiner).kinds_tried, vec![PathKind::Relay]);
    net.run(Duration::from_secs(1));
    // No packet ever went between the two public IPs.
    let (ha, hb): (IpAddr, IpAddr) = (
        "198.51.100.1".parse().unwrap(),
        "198.51.100.2".parse().unwrap(),
    );
    for (from, to) in &net.log {
        assert!(
            !(from.ip() == ha && to.ip() == hb || from.ip() == hb && to.ip() == ha),
            "{from} -> {to}"
        );
    }

    // The same when the joiner asks for it.
    let mut net = Net::new();
    let a = net.nat("198.51.100.1", NatKind::FullCone);
    let b = net.nat("198.51.100.2", NatKind::FullCone);
    let host = net.add_host("192.168.1.10:47420", Some(a), Net::host_config(None));
    let code = net.wait_online(host);
    let joiner = net.add_joiner("192.168.2.20:50000", Some(b), code, true);
    assert_eq!(net.wait_joined(joiner), PathKind::Relay);
    for (from, to) in &net.log {
        assert!(
            !(from.ip() == ha && to.ip() == hb),
            "host sent to the joiner: {from} -> {to}"
        );
    }
}

#[test]
fn players_behind_the_same_router_use_the_lan() {
    let mut net = Net::new();
    let a = net.nat("198.51.100.1", NatKind::PortRestricted);
    let host = net.add_host(
        "192.168.1.10:47420",
        Some(a),
        Net::host_config(Some(addr("192.168.1.10:47420"))),
    );
    let code = net.wait_online(host);
    let joiner = net.add_joiner("192.168.1.20:50000", Some(a), code, false);
    assert_eq!(net.wait_joined(joiner), PathKind::Lan);
}

#[test]
fn the_gate_keeps_out_strangers_who_know_the_address() {
    let mut net = Net::new();
    // A full-cone NAT lets anyone's packets through to the host: only the gate stops them.
    let a = net.nat("198.51.100.1", NatKind::FullCone);
    let host = net.add_host("192.168.1.10:47420", Some(a), Net::host_config(None));
    let code = net.wait_online(host);
    let host_public = net.nats[a]
        .back
        .iter()
        .find(|(_, p)| **p == addr("192.168.1.10:47420"))
        .map(|(port, _)| SocketAddr::new(net.nats[a].public, *port))
        .unwrap();
    net.nodes.push(Node {
        private: addr("203.0.113.66:5555"),
        nat: None,
        role: Role::Stranger(Stranger {
            target: host_public,
            accepted: false,
            replies: 0,
        }),
    });
    let stranger = net.nodes.len() - 1;
    // A real joiner gets in meanwhile.
    let b = net.nat("198.51.100.2", NatKind::PortRestricted);
    let joiner = net.add_joiner("192.168.2.20:50000", Some(b), code, false);
    assert_eq!(net.wait_joined(joiner), PathKind::Direct);
    net.run(Duration::from_secs(3));
    let Role::Stranger(s) = &net.nodes[stranger].role else {
        unreachable!()
    };
    assert!(!s.accepted);
    assert_eq!(s.replies, 0, "the gate drops them before netcode sees them");
    assert!(net.host(host).conns.len() == 1);
}

#[test]
fn a_wrong_code_finds_nothing_and_guessing_is_limited() {
    let mut net = Net::new();
    let a = net.nat("198.51.100.1", NatKind::PortRestricted);
    let host = net.add_host("192.168.1.10:47420", Some(a), Net::host_config(None));
    let code = net.wait_online(host);
    let b = net.nat("198.51.100.2", NatKind::Open);
    let mut guesses = Vec::new();
    for i in 0..8 {
        let mut wrong = *code.as_bytes();
        wrong[7] = if wrong[7] == b'Z' { b'Y' } else { b'Z' };
        wrong[6] = b"0123456789"[i];
        let wrong = Code::from_bytes(wrong).unwrap();
        if wrong == code {
            continue;
        }
        guesses.push(net.add_joiner(&format!("198.51.100.2:{}", 50000 + i), None, wrong, false));
        let _ = b;
    }
    let mut failures = Vec::new();
    for g in guesses {
        failures.push(net.join_failure(g));
    }
    assert!(failures[0].contains("no game with code"), "{failures:?}");
    assert!(
        failures
            .iter()
            .any(|f| f.contains("too many join attempts")),
        "one IP can't keep guessing: {failures:?}"
    );
}

#[test]
fn a_banned_joiner_cannot_come_back() {
    let mut net = Net::new();
    let a = net.nat("198.51.100.1", NatKind::PortRestricted);
    let b = net.nat("198.51.100.2", NatKind::PortRestricted);
    let host = net.add_host("192.168.1.10:47420", Some(a), Net::host_config(None));
    let code = net.wait_online(host);
    let joiner = net.add_joiner("192.168.2.20:50000", Some(b), code, false);
    net.wait_joined(joiner);
    let (peer, id) = net
        .host(host)
        .conns
        .iter()
        .next()
        .map(|(p, i)| (*p, *i))
        .unwrap();
    net.host(host).agent.ban(id);
    net.host(host).conns.remove(&peer);
    net.run(Duration::from_secs(2));
    let again = net.add_joiner("192.168.2.21:50001", Some(b), code, false);
    let failure = net.join_failure(again);
    assert!(failure.contains("doesn't let you join"), "{failure}");
}

#[test]
fn versions_must_match() {
    let mut net = Net::new();
    let host = net.add_host("198.51.100.9:47420", None, Net::host_config(None));
    let code = net.wait_online(host);
    let joiner = net.add_joiner("198.51.100.10:5000", None, code, false);
    if let Role::Joiner(j) = &mut net.nodes[joiner].role {
        j.agent = JoinAgent::new(JoinConfig {
            service: SERVICE.into(),
            code,
            protocol: PROTOCOL + 1,
            client_ids: [1, 2, 3],
            lan: None,
            relay_only: false,
        });
    }
    let failure = net.join_failure(joiner);
    assert!(failure.contains("another version (build-42)"), "{failure}");
}

#[test]
fn a_restarted_service_keeps_codes_and_relayed_games() {
    let mut net = Net::new();
    let a = net.nat("198.51.100.1", NatKind::Symmetric);
    let b = net.nat("198.51.100.2", NatKind::Symmetric);
    let host = net.add_host("192.168.1.10:47420", Some(a), Net::host_config(None));
    let code = net.wait_online(host);
    let joiner = net.add_joiner("192.168.2.20:50000", Some(b), code, false);
    assert_eq!(net.wait_joined(joiner), PathKind::Relay);

    // A deploy: a new process with the same key, nothing else.
    net.service = Service::new(&KEY, ServiceConfig::default(), net.now);
    net.run(Duration::from_secs(4));
    assert_eq!(
        net.host(host).code(),
        Some(code),
        "same code after the restart"
    );
    assert_eq!(net.service.stats().listings, 1);
    // Both ends bound the relay again: the game's traffic flows.
    let before = net.joiner(joiner).pongs;
    net.run(Duration::from_secs(2));
    let after = net.joiner(joiner).pongs;
    assert!(
        after > before + 10,
        "relay resumed: {before} -> {after} ({:?})",
        net.service.stats()
    );

    // The same after a restart with a different key: the host gets a new code.
    net.service = Service::new(&[9; 32], ServiceConfig::default(), net.now);
    net.run(Duration::from_secs(4));
    let new_code = net.host(host).code();
    assert!(new_code.is_some() && new_code != Some(code), "{new_code:?}");
}

#[test]
fn lossy_links_still_connect() {
    for (kinds, expect) in [
        (
            (NatKind::PortRestricted, NatKind::PortRestricted),
            PathKind::Direct,
        ),
        ((NatKind::Symmetric, NatKind::Symmetric), PathKind::Relay),
    ] {
        let mut net = Net::new();
        net.loss = 20;
        let a = net.nat("198.51.100.1", kinds.0);
        let b = net.nat("198.51.100.2", kinds.1);
        let host = net.add_host("192.168.1.10:47420", Some(a), Net::host_config(None));
        let code = net.wait_online(host);
        let joiner = net.add_joiner("192.168.2.20:50000", Some(b), code, false);
        assert_eq!(net.wait_joined(joiner), expect);
    }
}

#[test]
fn listings_show_public_games_of_our_version() {
    let mut net = Net::new();
    let h1 = net.add_host("198.51.100.9:47420", None, Net::host_config(None));
    let mut private = Net::host_config(None);
    private.info.public = false;
    let h2 = net.add_host("198.51.100.10:47420", None, private);
    net.wait_online(h1);
    net.wait_online(h2);
    // Ask by hand.
    let me = addr("198.51.100.50:6000");
    let mut out = Vec::new();
    let (now, unix) = (net.now, net.unix());
    net.service.handle(
        now,
        unix,
        me,
        &proto::encode(&Message::Hello { nonce: 1 }),
        &mut out,
    );
    let Ok(Message::Cookie { cookie, .. }) = proto::decode(&out[0].1) else {
        panic!()
    };
    let list = |net: &mut Net, protocol| {
        let mut out = Vec::new();
        net.service.handle(
            now,
            unix,
            me,
            &proto::encode(&Message::List {
                cookie,
                nonce: 2,
                protocol,
                page: 0,
            }),
            &mut out,
        );
        match proto::decode(&out[0].1) {
            Ok(Message::ListPage { entries, .. }) => entries,
            other => panic!("{other:?}"),
        }
    };
    let entries = list(&mut net, PROTOCOL);
    assert_eq!(entries.len(), 1, "only the public one");
    assert_eq!(Some(entries[0].code), net.host(h1).code());
    assert_eq!(entries[0].name, "SAM'S GAME");
    assert!(list(&mut net, PROTOCOL + 1).is_empty());
}
