//! The service and the host agent against abuse: reflection and amplification, guessing,
//! stolen or forged credentials, relay hijacking, and running out of resources.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::time::Duration;

use rendezvous::crypto::Keys;
use rendezvous::proto::{
    self, ErrorCode, HostInfo, Introduction, JoinFailure, Message, RelayGrant, Role, encode,
    encode_introduce, encode_relay,
};
use rendezvous::service::Outbox;
use rendezvous::{Code, HostAgent, HostConfig, HostStatus, Inbound, Service, ServiceConfig};

const KEY: [u8; 32] = [7; 32];
const UNIX: u64 = 1_900_000_000;

fn addr(s: &str) -> SocketAddr {
    s.parse().unwrap()
}

struct Svc {
    service: Service,
    now: Duration,
}

impl Svc {
    fn new(config: ServiceConfig) -> Svc {
        Svc {
            service: Service::new(&KEY, config, Duration::ZERO),
            now: Duration::from_secs(1),
        }
    }

    fn send(&mut self, from: SocketAddr, data: &[u8]) -> Outbox {
        let mut out = Vec::new();
        self.service
            .handle(self.now, UNIX + self.now.as_secs(), from, data, &mut out);
        out
    }

    fn ask(&mut self, from: SocketAddr, msg: &Message) -> Vec<Message<'static>> {
        self.send(from, &encode(msg))
            .into_iter()
            .filter(|(to, _)| *to == from)
            .map(|(_, data)| owned(proto::decode(&data).unwrap()))
            .collect()
    }

    fn cookie(&mut self, from: SocketAddr) -> [u8; 16] {
        match self.ask(from, &Message::Hello { nonce: 1 }).as_slice() {
            [Message::Cookie { cookie, .. }] => *cookie,
            other => panic!("{other:?}"),
        }
    }

    fn register(&mut self, from: SocketAddr, info: HostInfo) -> Message<'static> {
        let cookie = self.cookie(from);
        let mut replies = self.ask(
            from,
            &Message::Register {
                cookie,
                nonce: rendezvous::crypto::random_nonzero_u64(),
                resume: None,
                lan: None,
                info,
                bans: vec![],
            },
        );
        replies.remove(0)
    }

    fn join(
        &mut self,
        from: SocketAddr,
        code: Code,
        relay_only: bool,
    ) -> (Message<'static>, Outbox) {
        let cookie = self.cookie(from);
        let out = self.send(
            from,
            &encode(&Message::Join {
                cookie,
                nonce: rendezvous::crypto::random_nonzero_u64(),
                code,
                protocol: 11,
                client_ids: [1, 2, 3],
                lan: None,
                relay_only,
            }),
        );
        let reply = out
            .iter()
            .find(|(to, _)| *to == from)
            .map(|(_, d)| owned(proto::decode(d).unwrap()))
            .unwrap();
        (reply, out)
    }
}

/// Copies borrowed relay payloads so replies can outlive the outbox.
fn owned(msg: Message<'_>) -> Message<'static> {
    match msg {
        Message::Relay { session, payload } => Message::Relay {
            session,
            payload: Box::leak(payload.to_vec().into_boxed_slice()),
        },
        Message::Hello { nonce } => Message::Hello { nonce },
        // Every other variant owns its data; re-decode to change the lifetime.
        other => {
            let data = encode(&other);
            let data: &'static [u8] = Box::leak(data.into_boxed_slice());
            proto::decode(data).unwrap()
        }
    }
}

fn info() -> HostInfo {
    HostInfo {
        name: "HOST".into(),
        level: "knell_yard".into(),
        max_players: 8,
        players: 1,
        protocol: 11,
        public: true,
        ..Default::default()
    }
}

fn registered(msg: &Message) -> ([u8; 16], Code, [u8; 32]) {
    match msg {
        Message::Registered {
            host_id,
            code,
            secret,
            ..
        } => (*host_id, *code, *secret),
        other => panic!("not registered: {other:?}"),
    }
}

#[test]
fn unverified_senders_never_get_more_than_they_sent() {
    // Random and half-valid datagrams from spoofed addresses (which never see the replies,
    // so never hold a valid cookie): per address, the replies must not outweigh the requests.
    let mut svc = Svc::new(ServiceConfig::default());
    let host = addr("198.51.100.1:4000");
    let (_, code, _) = registered(&svc.register(host, info()));
    let mut state = 0x2545_f491_4f6c_dd1du64;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    let templates: Vec<Vec<u8>> = vec![
        encode(&Message::Hello { nonce: 5 }),
        encode(&Message::Probe { nonce: 5 }),
        encode(&Message::Join {
            cookie: [0; 16],
            nonce: 9,
            code,
            protocol: 11,
            client_ids: [1, 2, 3],
            lan: None,
            relay_only: false,
        }),
        encode(&Message::List {
            cookie: [1; 16],
            nonce: 9,
            protocol: 11,
            page: 0,
        }),
        encode_relay(77, &[1, 2, 3]),
        {
            let mut v = encode(&Message::Ok { nonce: 1 });
            v[4] = 9; // another version
            v
        },
    ];
    let mut sent: HashMap<SocketAddr, usize> = HashMap::new();
    let mut received: HashMap<SocketAddr, usize> = HashMap::new();
    for i in 0..50_000u32 {
        let victim = SocketAddr::from(([10, (i % 7) as u8, 0, 1], 1000 + (i % 13) as u16));
        let mut data = templates[(next() as usize) % templates.len()].clone();
        if next() % 3 == 0 && data.len() > 6 {
            let at = 6 + (next() as usize) % (data.len() - 6);
            data[at] = next() as u8;
        }
        if next() % 5 == 0 {
            data.truncate((next() as usize) % (data.len() + 1));
        }
        *sent.entry(victim).or_default() += data.len();
        for (to, reply) in svc.send(victim, &data) {
            assert_eq!(to, victim, "nothing goes to third parties");
            assert!(reply.len() <= data.len(), "a reply bigger than its request");
            *received.entry(to).or_default() += reply.len();
        }
        svc.now += Duration::from_micros(200);
    }
    for (victim, bytes) in received {
        assert!(
            bytes <= sent[&victim],
            "{victim}: {bytes} > {}",
            sent[&victim]
        );
    }
}

#[test]
fn unverified_replies_have_a_global_budget() {
    let mut svc = Svc::new(ServiceConfig {
        unverified_rate: 100,
        ..Default::default()
    });
    let hello = encode(&Message::Hello { nonce: 1 });
    let mut answered = 0;
    for i in 0..1000u32 {
        let from = SocketAddr::from(([10, 0, (i >> 8) as u8, i as u8], 9));
        answered += svc.send(from, &hello).len();
    }
    assert_eq!(answered, 200, "a burst of 2 seconds' worth, then nothing");
}

#[test]
fn cookies_belong_to_one_address_and_expire() {
    let mut svc = Svc::new(ServiceConfig::default());
    let me = addr("198.51.100.7:5000");
    let cookie = svc.cookie(me);
    let list = |cookie| Message::List {
        cookie,
        nonce: 3,
        protocol: 11,
        page: 0,
    };
    assert!(matches!(
        svc.ask(me, &list(cookie))[0],
        Message::ListPage { .. }
    ));
    // Another port of the same IP, or another IP: not valid.
    for other in [addr("198.51.100.7:5001"), addr("198.51.100.8:5000")] {
        assert!(matches!(
            svc.ask(other, &list(cookie))[..],
            [Message::Error {
                code: ErrorCode::BadCookie,
                ..
            }]
        ));
    }
    // Two minutes later: expired.
    svc.now += Duration::from_secs(121);
    assert!(matches!(
        svc.ask(me, &list(cookie))[..],
        [Message::Error {
            code: ErrorCode::BadCookie,
            ..
        }]
    ));
}

#[test]
fn listings_need_their_secret() {
    let mut svc = Svc::new(ServiceConfig::default());
    let host = addr("198.51.100.1:4000");
    let (host_id, code, secret) = registered(&svc.register(host, info()));
    let attacker = addr("203.0.113.9:6666");
    let cookie = svc.cookie(attacker);
    let mut wrong = secret;
    wrong[0] ^= 1;
    for msg in [
        Message::Heartbeat {
            cookie,
            nonce: 4,
            host_id,
            secret: wrong,
            info: HostInfo {
                name: "HIJACKED".into(),
                ..info()
            },
        },
        Message::Unregister {
            cookie,
            nonce: 5,
            host_id,
            secret: wrong,
        },
        Message::Ban {
            cookie,
            nonce: 6,
            host_id,
            secret: wrong,
            tag: [0; 16],
        },
    ] {
        assert!(matches!(
            svc.ask(attacker, &msg)[..],
            [Message::Error {
                code: ErrorCode::Refused,
                ..
            }]
        ));
    }
    // A forged resume (someone else's code, made-up secret) is refused too.
    let forged = Message::Register {
        cookie,
        nonce: 7,
        resume: Some(proto::Resume {
            host_id,
            code,
            secret: wrong,
        }),
        lan: None,
        info: info(),
        bans: vec![],
    };
    assert!(matches!(
        svc.ask(attacker, &forged)[..],
        [Message::Error {
            code: ErrorCode::Refused,
            ..
        }]
    ));
    // The listing is untouched: still joinable at the host's address.
    let (reply, _) = svc.join(addr("198.51.100.20:7000"), code, false);
    match reply {
        Message::JoinOk { host: Some(h), .. } => assert_eq!(h, host),
        other => panic!("{other:?}"),
    }
}

#[test]
fn relays_carry_only_their_two_ends() {
    let mut svc = Svc::new(ServiceConfig::default());
    let host = addr("198.51.100.1:4000");
    let (host_id, code, _) = registered(&svc.register(host, info()));
    let joiner = addr("198.51.100.2:5000");
    let (reply, _) = svc.join(joiner, code, true);
    let Message::JoinOk {
        relay: Some(grant),
        host: None,
        ..
    } = reply
    else {
        panic!("{reply:?}");
    };
    // The two ends reach each other.
    let out = svc.send(joiner, &encode_relay(grant.session, b"hello"));
    assert_eq!(out, vec![(host, encode_relay(grant.session, b"hello"))]);
    let out = svc.send(host, &encode_relay(grant.session, b"back"));
    assert_eq!(out, vec![(joiner, encode_relay(grant.session, b"back"))]);
    // A third party knowing the session id forwards nothing (and only learns "unknown").
    let attacker = addr("203.0.113.9:6666");
    let out = svc.send(attacker, &encode_relay(grant.session, b"inject"));
    assert_eq!(
        out,
        vec![(
            attacker,
            encode(&Message::RelayUnknown {
                session: grant.session
            })
        )]
    );
    // Binding needs the credential, for that role and session.
    let cookie = svc.cookie(attacker);
    let bind = |role, session, cred| Message::RelayBind {
        cookie,
        nonce: 8,
        session,
        role,
        grant_expires: grant.expires,
        host_id,
        cred,
    };
    for msg in [
        bind(Role::Joiner, grant.session, [0; 16]),
        bind(Role::Host, grant.session, grant.cred), // the joiner's credential, host role
        bind(Role::Joiner, grant.session + 1, grant.cred),
    ] {
        assert!(matches!(
            svc.ask(attacker, &msg)[..],
            [Message::Error {
                code: ErrorCode::Refused,
                ..
            }]
        ));
    }
    // An expired credential is refused even with the right MAC.
    let keys = Keys::derive(&KEY);
    let past = UNIX - 10;
    let msg = Message::RelayBind {
        cookie,
        nonce: 9,
        session: grant.session,
        role: Role::Joiner,
        grant_expires: past,
        host_id,
        cred: keys.relay_cred(grant.session, Role::Joiner as u8, past, &host_id),
    };
    assert!(matches!(
        svc.ask(attacker, &msg)[..],
        [Message::Error {
            code: ErrorCode::Refused,
            ..
        }]
    ));
    // Still only the two ends.
    let out = svc.send(joiner, &encode_relay(grant.session, b"still"));
    assert_eq!(out, vec![(host, encode_relay(grant.session, b"still"))]);
}

#[test]
fn relay_traffic_has_budgets() {
    let mut svc = Svc::new(ServiceConfig {
        relay_rate: 10_000,
        relay_burst: 20_000,
        relay_daily_bytes: 1_000_000,
        ..Default::default()
    });
    let host = addr("198.51.100.1:4000");
    let (_, code, _) = registered(&svc.register(host, info()));
    let joiner = addr("198.51.100.2:5000");
    let Message::JoinOk {
        relay: Some(grant), ..
    } = svc.join(joiner, code, true).0
    else {
        panic!()
    };
    let packet = encode_relay(grant.session, &[0; 1000]);
    let forwarded = (0..100)
        .filter(|_| !svc.send(joiner, &packet).is_empty())
        .count();
    // 20 KB of burst at ~1 KB a packet.
    assert!((18..=20).contains(&forwarded), "{forwarded}");
    // The daily budget, once used up, holds even with the rate refilled.
    let mut total = 0;
    for _ in 0..2000 {
        svc.now += Duration::from_millis(200);
        total += svc.send(joiner, &packet).len();
    }
    assert!(total <= 1_000_000 / packet.len(), "{total}");
}

#[test]
fn resources_are_bounded_per_address() {
    let mut svc = Svc::new(ServiceConfig::default());
    let ip_port = |p: u16| SocketAddr::from(([198, 51, 100, 1], p));
    for p in 0..4 {
        registered(&svc.register(ip_port(4000 + p), info()));
    }
    assert!(matches!(
        svc.register(ip_port(4100), info()),
        Message::Error {
            code: ErrorCode::Busy,
            ..
        }
    ));
    // Relays per joiner IP.
    let (_, code, _) = registered(&svc.register(addr("198.51.100.9:4000"), info()));
    let mut granted = 0;
    for p in 0..20u16 {
        svc.now += Duration::from_secs(3); // past the join rate limit
        if let (Message::JoinOk { relay: Some(_), .. }, _) = svc.join(
            SocketAddr::from(([198, 51, 100, 50], 6000 + p)),
            code,
            false,
        ) {
            granted += 1;
        }
    }
    assert_eq!(granted, ServiceConfig::default().max_relays_per_ip);
}

#[test]
fn retried_requests_get_the_same_answer() {
    let mut svc = Svc::new(ServiceConfig::default());
    let host = addr("198.51.100.1:4000");
    let cookie = svc.cookie(host);
    let register = encode(&Message::Register {
        cookie,
        nonce: 42,
        resume: None,
        lan: None,
        info: info(),
        bans: vec![],
    });
    let first = svc.send(host, &register);
    let second = svc.send(host, &register);
    assert_eq!(first, second, "one listing, not two");
    assert_eq!(svc.service.stats().listings, 1);
    // Retried joins don't use up the rate limit or open more relays.
    let (_, code, _) = registered(&proto::decode(&first[0].1).map(owned).unwrap());
    let joiner = addr("198.51.100.2:5000");
    let cookie = svc.cookie(joiner);
    let join = encode(&Message::Join {
        cookie,
        nonce: 43,
        code,
        protocol: 11,
        client_ids: [1, 2, 3],
        lan: None,
        relay_only: false,
    });
    for _ in 0..20 {
        let replies: Vec<_> = svc
            .send(joiner, &join)
            .into_iter()
            .filter(|(to, _)| *to == joiner)
            .collect();
        assert!(matches!(
            proto::decode(&replies[0].1),
            Ok(Message::JoinOk { .. })
        ));
    }
    assert_eq!(svc.service.stats().relays, 1);
}

#[test]
fn version_mismatch_and_unknown_codes() {
    let mut svc = Svc::new(ServiceConfig::default());
    let (_, code, _) = registered(&svc.register(
        addr("198.51.100.1:4000"),
        HostInfo {
            protocol: 12,
            build: "build-99".into(),
            ..info()
        },
    ));
    match svc.join(addr("198.51.100.2:5000"), code, false).0 {
        Message::JoinFailed {
            reason: JoinFailure::VersionMismatch,
            host_build,
            ..
        } => assert_eq!(host_build, "build-99"),
        other => panic!("{other:?}"),
    }
    match svc.join(addr("198.51.100.3:5000"), Code::random(), false).0 {
        Message::JoinFailed {
            reason: JoinFailure::NotFound,
            ..
        } => {}
        other => panic!("{other:?}"),
    }
}

fn agent() -> HostAgent {
    HostAgent::new(HostConfig {
        service: "203.0.113.1:47400".into(),
        info: info(),
        lan: None,
        gate: true,
        trust_loopback: true,
        trust_lan: false,
    })
}

/// Brings an agent online against a real service; returns the agent, the service and the
/// agent's (host_id, secret).
fn online_agent() -> (HostAgent, Svc, [u8; 16], [u8; 32]) {
    let service_addr = addr("203.0.113.1:47400");
    let me = addr("198.51.100.1:4000");
    let mut svc = Svc::new(ServiceConfig::default());
    let mut agent = agent();
    let mut ids = None;
    for i in 0..100 {
        let now = Duration::from_millis(10 * i);
        let mut out = Vec::new();
        agent.poll(now, &mut out);
        for (to, data) in out {
            assert_eq!(to, service_addr);
            for (back, reply) in svc.send(me, &data) {
                assert_eq!(back, me);
                if let Ok(Message::Registered {
                    host_id, secret, ..
                }) = proto::decode(&reply)
                {
                    ids = Some((host_id, secret));
                }
                agent.on_receive(now, service_addr, &reply);
            }
        }
        if matches!(agent.status(), HostStatus::Online { .. }) {
            break;
        }
    }
    let (host_id, secret) = ids.expect("registered");
    (agent, svc, host_id, secret)
}

#[test]
fn forged_introductions_admit_nobody() {
    let (mut agent, _svc, host_id, secret) = online_agent();
    let service_addr = addr("203.0.113.1:47400");
    let joiner = addr("198.51.100.66:5000");
    let intro = Introduction {
        host_id,
        join_id: 99,
        client_ids: [11, 12, 13],
        joiner: Some(joiner),
        joiner_lan: None,
        relay: Some(RelayGrant {
            session: 5,
            expires: UNIX + 100,
            host_id,
            cred: [0; 16],
        }),
        ban_tag: [0; 16],
    };
    let now = Duration::from_secs(2);
    // Signed with the wrong secret, or the right one but from somewhere else than the
    // service: nothing changes.
    let mut wrong = secret;
    wrong[3] ^= 0x40;
    agent.on_receive(now, service_addr, &encode_introduce(&intro, &wrong));
    agent.on_receive(
        now,
        addr("203.0.113.2:47400"),
        &encode_introduce(&intro, &secret),
    );
    assert_eq!(
        agent.on_receive(now, joiner, &[0x10, 1, 2, 3]),
        Inbound::Drop
    );
    assert!(!agent.admit(11, joiner));
    // Relayed packets for a session nobody granted are dropped too.
    assert_eq!(
        agent.on_receive(now, service_addr, &encode_relay(5, b"x")),
        Inbound::Drop
    );
    // The genuine one works, once, for the announced ids only.
    agent.on_receive(now, service_addr, &encode_introduce(&intro, &secret));
    assert_eq!(
        agent.on_receive(now, joiner, &[0x10, 1, 2, 3]),
        Inbound::Netcode(joiner)
    );
    assert!(!agent.admit(14, joiner), "an id it didn't announce");
    assert!(!agent.admit(12, addr("198.51.100.67:5000")), "another IP");
    assert!(agent.admit(12, joiner));
    assert!(!agent.admit(13, joiner), "the introduction is used up");
}

#[test]
fn replies_need_the_request_nonce() {
    // A spoofed `Registered` (right source address, wrong nonce) doesn't take over the agent.
    let mut agent = agent();
    let service_addr = addr("203.0.113.1:47400");
    let mut out = Vec::new();
    agent.poll(Duration::ZERO, &mut out);
    let fake = encode(&Message::Registered {
        nonce: 12345,
        host_id: [1; 16],
        code: Code::random(),
        secret: [2; 32],
    });
    agent.on_receive(Duration::ZERO, service_addr, &fake);
    assert_eq!(*agent.status(), HostStatus::Connecting);
}
