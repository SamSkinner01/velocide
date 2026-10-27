//! The arsenal over the real network stack (UDP loopback, virtual clock): matches run with
//! the balance profile of their mode and clients get that effective tuning, hot reloads
//! recompute it, PvP has no Thorns (the knack applies as Surge), and
//! `ClientMessage::SetLoadout` chosen in the lobby is what the player spawns with (sanitised,
//! knack included; while alive in a match it waits for the respawn).

use std::path::PathBuf;
use std::time::Duration;

use client::net::{ClientConfig, NetClient};
use engine::time::FixedTimestep;
use server::{LobbyConfig, Server, ServerConfig};
use shared::input::PlayerInput;
use shared::loadout::{Ability, Knack, Loadout, WeaponBuild, WeaponKind};
use shared::lobby::LobbyPhase;
use shared::protocol::{self, ClientMessage};
use shared::sim::Attack;

const STEP: Duration = Duration::from_millis(2);

fn config(level: &str) -> ServerConfig {
    ServerConfig {
        bind: "127.0.0.1:0".parse().unwrap(),
        level: level.to_string(),
        // Defaults only: tests must not depend on the repo's assets/tuning.ron.
        tuning_path: Some(PathBuf::from("/nonexistent/velocide-test/tuning.ron")),
        enemies_dir: Some(PathBuf::from("/nonexistent/velocide-test/enemies")),
        watch_tuning: false,
        ..Default::default()
    }
}

struct Net {
    server: Server,
    clients: Vec<(NetClient, FixedTimestep)>,
}

impl Net {
    fn new(config: ServerConfig) -> Self {
        let _ = env_logger::builder().is_test(true).try_init();
        Self {
            server: Server::new(config).expect("server starts"),
            clients: Vec::new(),
        }
    }

    fn add(&mut self) -> usize {
        let net = NetClient::connect(self.server.local_addr(), ClientConfig::new("c"))
            .expect("client starts");
        self.clients.push((net, FixedTimestep::new(60.0)));
        self.clients.len() - 1
    }

    fn run(&mut self, secs: f64) {
        for _ in 0..(secs / STEP.as_secs_f64()).round() as u32 {
            self.server.update(STEP);
            for (c, ts) in &mut self.clients {
                c.update(STEP);
                c.drain_events();
                ts.set_hz(c.tick_rate() as f64);
                for _ in 0..ts.advance(STEP.as_secs_f64()) {
                    if c.is_in_game() {
                        c.tick(PlayerInput::default());
                    }
                }
            }
        }
    }

    fn send_loadout(&mut self, client: usize, loadout: Loadout) {
        let msg = protocol::encode(&ClientMessage::SetLoadout(loadout));
        self.clients[client].0.send_raw(true, msg);
    }
}

fn bow_and_kiln() -> Loadout {
    Loadout {
        slots: [
            Some(WeaponBuild {
                weapon: WeaponKind::Longwick,
                abilities: [Ability::EmberArrow, Ability::PinningArrow],
            }),
            None,
            Some(WeaponKind::Kiln.default_build()),
            None,
        ],
        knack: Knack::KindleFlask,
    }
}

#[test]
fn pvp_clients_get_surge_for_the_thorn_coop_clients_keep_it() {
    // proving_ground is an arena: deathmatch, the PvP profile.
    let mut pvp = Net::new(config(shared::sim::BUILTIN_LEVEL));
    let c = pvp.add();
    pvp.run(1.5);
    assert!(pvp.clients[c].0.is_in_game());
    assert_eq!(pvp.clients[c].0.tuning(), pvp.server.tuning());
    // No Thorns in PvP: the default loadout's Thorn applies as Surge, in the world and in the
    // owner's own state (what its prediction runs).
    let id = pvp.clients[c].0.local_player_id().expect("joined");
    assert_eq!(
        pvp.server.world().players[&id].combat.weapons.knack(),
        Knack::Surge
    );
    let own = pvp.clients[c].0.own_combat().expect("own state");
    assert_eq!(own.combat.weapons.loadout.knack, Knack::Surge);

    // A campaign level: co-op, the PvE profile (the reference): the Thorn, three charges.
    let mut coop = Net::new(config("crucible_of_veins"));
    let c = coop.add();
    coop.run(1.5);
    assert!(coop.clients[c].0.is_in_game());
    let own = coop.clients[c].0.own_combat().expect("own state");
    assert_eq!(own.combat.weapons.knack(), Knack::Thorn);
    let t = coop.clients[c].0.tuning();
    assert_eq!(own.combat.weapons.knack_charges(&t.combat), 3);
    assert_eq!(
        coop.clients[c].0.tuning().combat,
        coop.server.base_tuning().combat
    );

    // A reload recomputes the effective tuning and sends it.
    let mut t = pvp.server.base_tuning().clone();
    t.balance.pvp.overrides.insert("thorn.charges".into(), 1.0);
    t.combat.quill.damage = 30.0;
    pvp.server.set_tuning(t);
    pvp.run(0.5);
    let got = pvp.clients[0].0.tuning();
    assert_eq!(got.combat.thorn.charges, 1);
    assert_eq!(got.combat.quill.damage, 30.0);
}

#[test]
fn a_loadout_chosen_in_the_lobby_is_what_the_match_spawns() {
    let mut cfg = config(shared::sim::BUILTIN_LEVEL);
    cfg.lobby = LobbyConfig::enabled();
    let mut n = Net::new(cfg);
    let c = n.add();
    n.run(1.0);
    assert_eq!(n.server.lobby_phase(), LobbyPhase::Lobby);
    let id = n.clients[c].0.local_player_id().expect("joined");
    // Repeated weapon and a foreign ability: sanitised by the server.
    let mut sent = bow_and_kiln();
    sent.slots[3] = Some(WeaponBuild {
        weapon: WeaponKind::Kiln,
        abilities: [Ability::Gust, Ability::Gust],
    });
    n.send_loadout(c, sent);
    n.run(0.3);
    assert_eq!(n.server.world().loadout(id), bow_and_kiln());

    n.server.start_match().expect("match starts");
    n.run(1.5);
    let p = &n.server.world().players[&id];
    assert_eq!(p.combat.weapons.loadout, bow_and_kiln());
    assert_eq!(p.combat.weapons.current(), WeaponKind::Longwick);
    let own = n.clients[c].0.own_combat().expect("own state");
    assert_eq!(
        own.combat.weapons.loadout,
        bow_and_kiln(),
        "the owner sees it"
    );

    // In play a change waits for the respawn.
    let kiln = Loadout::single(WeaponKind::Kiln.default_build());
    n.send_loadout(c, kiln);
    n.run(0.3);
    assert_eq!(n.server.world().loadout(id), bow_and_kiln());
    let t = n.server.tuning().clone();
    n.server.world_mut().kill_player(&t, id, Attack::Dev);
    n.run(3.0);
    let p = &n.server.world().players[&id];
    assert!(!p.combat.dead, "respawned");
    assert_eq!(p.combat.weapons.loadout, kiln);
    let own = n.clients[c].0.own_combat().expect("own state");
    assert_eq!(own.combat.weapons.loadout, kiln);
}
