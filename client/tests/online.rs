//! Online play end to end over real UDP sockets on loopback: a rendezvous service, a server
//! hosting online with the gate on, and game clients joining by code directly and through
//! the relay (docs/rendezvous.md). Loopback isn't trusted by the gate here, so everything
//! from 127.0.0.1 counts as "from the internet".

use std::net::{SocketAddr, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use client::net::{ClientConfig, NetClient};
use rendezvous::browse::{BrowseState, Browser};
use rendezvous::{Code, HostStatus, Service, ServiceConfig};
use server::{LobbyConfig, OnlineConfig, ServerCommand, ServerConfig, ServerHandle};
use shared::protocol::PROTOCOL_VERSION;

struct ServiceThread {
    addr: SocketAddr,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl ServiceThread {
    fn spawn() -> ServiceThread {
        let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
        socket
            .set_read_timeout(Some(Duration::from_millis(2)))
            .unwrap();
        let addr = socket.local_addr().unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let thread = std::thread::spawn({
            let stop = stop.clone();
            move || {
                let epoch = Instant::now();
                let mut service = Service::new(&[3; 32], ServiceConfig::default(), Duration::ZERO);
                let mut buf = [0u8; 2048];
                let mut out = Vec::new();
                let mut last_tick = Duration::ZERO;
                let unix = || {
                    SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap()
                        .as_secs()
                };
                while !stop.load(Ordering::Relaxed) {
                    if let Ok((len, from)) = socket.recv_from(&mut buf) {
                        service.handle(epoch.elapsed(), unix(), from, &buf[..len], &mut out);
                    }
                    let now = epoch.elapsed();
                    if now - last_tick >= Duration::from_millis(20) {
                        last_tick = now;
                        service.tick(now, unix(), &mut out);
                    }
                    for (to, data) in out.drain(..) {
                        let _ = socket.send_to(&data, to);
                    }
                }
            }
        });
        ServiceThread {
            addr,
            stop,
            thread: Some(thread),
        }
    }
}

impl Drop for ServiceThread {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// Updates the clients until `done` (or `secs` pass).
fn pump(
    clients: &mut [&mut NetClient],
    secs: f64,
    mut done: impl FnMut(&[&mut NetClient]) -> bool,
) -> bool {
    let end = Instant::now() + Duration::from_secs_f64(secs);
    let mut last = Instant::now();
    while Instant::now() < end {
        let now = Instant::now();
        for c in clients.iter_mut() {
            c.update(now - last);
            c.drain_events();
        }
        last = now;
        if done(clients) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(4));
    }
    false
}

fn online_host(service: SocketAddr) -> (ServerHandle, Arc<Mutex<HostStatus>>) {
    let status = Arc::new(Mutex::new(HostStatus::Connecting));
    let config = ServerConfig {
        bind: "127.0.0.1:0".parse().unwrap(),
        lobby: LobbyConfig::enabled(),
        server_name: "TEST GAME".into(),
        watch_tuning: false,
        online: Some(OnlineConfig {
            service: service.to_string(),
            public: true,
            relay_only: false,
            gate: true,
            trust_lan: false,
            trust_loopback: false,
            status: status.clone(),
        }),
        ..Default::default()
    };
    (ServerHandle::spawn(config).unwrap(), status)
}

fn wait_code(status: &Mutex<HostStatus>) -> Code {
    let end = Instant::now() + Duration::from_secs(10);
    while Instant::now() < end {
        if let HostStatus::Online { code } = &*status.lock().unwrap() {
            return *code;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("never online: {:?}", status.lock().unwrap());
}

#[test]
fn players_find_the_game_and_join_by_code_directly_and_through_the_relay() {
    let service = ServiceThread::spawn();
    let svc = service.addr.to_string();
    let (server, status) = online_host(service.addr);
    let code = wait_code(&status);

    // It's in the server browser.
    let mut browser = Browser::start(&svc, PROTOCOL_VERSION);
    let end = Instant::now() + Duration::from_secs(5);
    let entries = loop {
        if let BrowseState::Done(entries) = browser.poll() {
            break entries.clone();
        }
        assert!(Instant::now() < end, "browser: {:?}", browser.state());
        std::thread::sleep(Duration::from_millis(5));
    };
    assert!(
        entries
            .iter()
            .any(|e| e.code == code && e.name == "TEST GAME"),
        "{entries:?}"
    );

    // One player connects directly, one through the relay; both reach the lobby.
    let mut direct =
        NetClient::connect_online(&svc, code, false, ClientConfig::new("Direct")).unwrap();
    let mut relayed =
        NetClient::connect_online(&svc, code, true, ClientConfig::new("Relayed")).unwrap();
    let ok = pump(&mut [&mut direct, &mut relayed], 20.0, |c| {
        c.iter().all(|c| c.in_lobby() && c.players().len() == 2)
    });
    assert!(
        ok,
        "direct: {:?} {:?}, relayed: {:?} {:?}",
        direct.state(),
        direct.disconnect_reason(),
        relayed.state(),
        relayed.disconnect_reason()
    );
    assert_eq!(
        relayed.transport_progress().as_deref(),
        Some("CONNECTED THROUGH THE RELAY")
    );
    assert_eq!(direct.transport_progress().as_deref(), Some("CONNECTED"));

    // Someone who knows the address but has no introduction gets nowhere.
    let mut stranger =
        NetClient::connect(server.local_addr(), ClientConfig::new("Stranger")).unwrap();
    pump(&mut [&mut stranger, &mut direct, &mut relayed], 3.0, |_| {
        false
    });
    assert!(stranger.lobby().is_none(), "the gate let a stranger in");
    assert!(server.stats().clients <= 2, "{:?}", server.stats());

    // Kicked = banned: the same address can't get back in by code.
    let player = direct.local_player_id().unwrap();
    server.send(ServerCommand::Kick {
        player,
        reason: "bye".into(),
    });
    assert!(pump(&mut [&mut direct, &mut relayed], 5.0, |c| c[0]
        .is_disconnected()));
    std::thread::sleep(Duration::from_millis(1500)); // the ban reaches the service
    let mut again =
        NetClient::connect_online(&svc, code, false, ClientConfig::new("Again")).unwrap();
    assert!(pump(&mut [&mut again], 10.0, |c| c[0].is_disconnected()));
    let reason = again.disconnect_reason().unwrap_or_default().to_string();
    assert!(reason.contains("doesn't let you join"), "{reason}");
    // The relayed player is still there.
    assert!(relayed.in_lobby());
    drop(server);
}

#[test]
fn a_wrong_code_says_so() {
    let service = ServiceThread::spawn();
    let mut client = NetClient::connect_online(
        &service.addr.to_string(),
        Code::parse("ZZZZ-ZZZZ").unwrap(),
        false,
        ClientConfig::new("Lost"),
    )
    .unwrap();
    assert!(pump(&mut [&mut client], 10.0, |c| c[0].is_disconnected()));
    let reason = client.disconnect_reason().unwrap_or_default();
    assert!(reason.contains("no game with code ZZZZ-ZZZZ"), "{reason}");
}
