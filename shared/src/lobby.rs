//! Lobby and match model shared by server and client (see `docs/lobby.md`).
//!
//! A server is always in one [`LobbyPhase`]:
//!
//! ```text
//!   Lobby ──host START (all ready) / FORCE START / autostart──▶ Countdown(3..1)
//!     ▲  ◀─────────────────── host CANCEL ───────────────────────┘ │
//!     │                                                            ▼
//!     ├──── host RETURN TO LOBBY ──────────────────────────────── InMatch ──Server::end_match──▶ Results
//!     └──── host CONTINUE (or timeout) ───────────────────────────────────────────────────────────┘
//! ```
//!
//! [`MatchSettings`] (mode, level, limits) are chosen in the lobby by the host and handed to
//! the simulation at match start (`SimWorld::match_settings`), where the game-mode rules read
//! them. [`LobbyState`] (phase, settings, players with ping / ready / host) is broadcast by the
//! server whenever it changes and once per second (pings). Clients send [`LobbyRequest`]s;
//! the server answers invalid ones with a [`LobbyUpdate::Notice`] to the sender only.

use serde::{Deserialize, Serialize};

use crate::level::{LevelDef, LevelKind};
use crate::sim::PlayerId;

/// Most players a server accepts (player colours, PvP arenas).
pub const MAX_PLAYERS: u8 = 8;
/// Co-op campaign is for up to this many players.
pub const MAX_COOP_PLAYERS: u8 = 4;
pub const DEFAULT_FRAG_LIMIT: u32 = 20;
pub const MAX_FRAG_LIMIT: u32 = 200;
/// Seconds (10 minutes).
pub const DEFAULT_TIME_LIMIT_SECS: u32 = 600;
/// Seconds (one hour).
pub const MAX_TIME_LIMIT_SECS: u32 = 3600;
/// Countdown between START and the level loading.
pub const COUNTDOWN_SECS: u8 = 3;
/// Longest level name accepted from a client.
pub const MAX_LEVEL_NAME_CHARS: usize = 64;
/// Sandbox / test levels that may be played in every mode.
pub const ANY_MODE_LEVELS: [&str; 2] = [crate::sim::BUILTIN_LEVEL, "movement_test"];

/// What kind of match is played.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum GameMode {
    /// Single-player / 1–4 player co-op through a campaign level.
    #[default]
    Campaign,
    /// PvP free-for-all on an arena, up to 8 players.
    Deathmatch,
    /// PvP free-for-all on an arena: every kill moves you to the next weapon of a fixed
    /// ladder; a kill with the last one wins (`docs/expansion.md`).
    GunGame,
    /// Co-op (1–4 players) endless waves on an arena until everyone is down.
    Grind,
}

impl GameMode {
    pub const ALL: [GameMode; 4] = [
        GameMode::Campaign,
        GameMode::Deathmatch,
        GameMode::GunGame,
        GameMode::Grind,
    ];

    /// Menu / scoreboard name.
    pub fn title(self) -> &'static str {
        match self {
            GameMode::Campaign => "CO-OP CAMPAIGN",
            GameMode::Deathmatch => "DEATHMATCH",
            GameMode::GunGame => "GUN GAME",
            GameMode::Grind => "GRIND",
        }
    }

    /// Command-line / config id.
    pub fn id(self) -> &'static str {
        match self {
            GameMode::Campaign => "campaign",
            GameMode::Deathmatch => "deathmatch",
            GameMode::GunGame => "gungame",
            GameMode::Grind => "grind",
        }
    }

    /// Parses an id ("campaign", "coop", "deathmatch", "dm", "gungame", "grind"...),
    /// case-insensitive.
    pub fn parse(text: &str) -> Option<GameMode> {
        match text.trim().to_ascii_lowercase().as_str() {
            "campaign" | "coop" | "co-op" | "c" => Some(GameMode::Campaign),
            "deathmatch" | "dm" | "pvp" | "d" => Some(GameMode::Deathmatch),
            "gungame" | "gun-game" | "gun_game" | "gg" => Some(GameMode::GunGame),
            "grind" | "endless" | "horde" | "waves" => Some(GameMode::Grind),
            _ => None,
        }
    }

    /// The kind of level this mode is played on.
    pub fn level_kind(self) -> LevelKind {
        match self {
            GameMode::Campaign => LevelKind::Campaign,
            GameMode::Deathmatch | GameMode::GunGame | GameMode::Grind => LevelKind::Arena,
        }
    }

    /// The natural mode for a level kind.
    pub fn for_level_kind(kind: LevelKind) -> GameMode {
        match kind {
            LevelKind::Campaign | LevelKind::Hub => GameMode::Campaign,
            LevelKind::Arena => GameMode::Deathmatch,
        }
    }

    /// Player cap of the mode.
    pub fn max_players(self) -> u8 {
        match self {
            GameMode::Campaign | GameMode::Grind => MAX_COOP_PLAYERS,
            GameMode::Deathmatch | GameMode::GunGame => MAX_PLAYERS,
        }
    }

    /// Players fight each other (PvP balance, no Thorns, play of the game).
    pub fn is_pvp(self) -> bool {
        matches!(self, GameMode::Deathmatch | GameMode::GunGame)
    }

    /// Players fight the Moulds together (PvE balance, difficulty applies).
    pub fn is_pve(self) -> bool {
        !self.is_pvp()
    }

    /// Whether `level` can be played in this mode: its kind matches, or it is a sandbox level
    /// ([`ANY_MODE_LEVELS`]).
    pub fn accepts(self, level: &LevelInfo) -> bool {
        level.any_mode() || level.kind == self.level_kind()
    }

    /// Frag limit / time limit apply (deathmatch).
    pub fn has_score_limits(self) -> bool {
        self == GameMode::Deathmatch
    }

    /// A time limit applies (deathmatch; gun game: most tiers wins when it runs out).
    pub fn has_time_limit(self) -> bool {
        matches!(self, GameMode::Deathmatch | GameMode::GunGame)
    }

    /// Friendly fire is a choice (co-op).
    pub fn has_friendly_fire(self) -> bool {
        self.is_pve()
    }

    /// The difficulty setting applies.
    pub fn has_difficulty(self) -> bool {
        self.is_pve()
    }
}

/// PvE difficulty (campaign co-op and grind). Scales the Moulds' health, damage and
/// aggression, on top of the scaling for the number of players (`docs/expansion.md`).
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize,
)]
pub enum Difficulty {
    Easy,
    #[default]
    Normal,
    Hard,
    Brutal,
    /// For full squads that want to be punished.
    Inferno,
}

impl Difficulty {
    pub const ALL: [Difficulty; 5] = [
        Difficulty::Easy,
        Difficulty::Normal,
        Difficulty::Hard,
        Difficulty::Brutal,
        Difficulty::Inferno,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Difficulty::Easy => "EASY",
            Difficulty::Normal => "NORMAL",
            Difficulty::Hard => "HARD",
            Difficulty::Brutal => "BRUTAL",
            Difficulty::Inferno => "INFERNO",
        }
    }

    pub fn id(self) -> &'static str {
        match self {
            Difficulty::Easy => "easy",
            Difficulty::Normal => "normal",
            Difficulty::Hard => "hard",
            Difficulty::Brutal => "brutal",
            Difficulty::Inferno => "inferno",
        }
    }

    pub fn parse(text: &str) -> Option<Difficulty> {
        let t = text.trim().to_ascii_lowercase();
        Self::ALL.into_iter().find(|d| d.id() == t)
    }

    /// Position in [`Difficulty::ALL`] (the index into the tuning's per-difficulty tables).
    pub fn index(self) -> usize {
        self as usize
    }
}

/// A level the server can run (its catalog is sent to clients so the lobby can offer levels
/// the client doesn't have installed).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LevelInfo {
    /// File stem / built-in name, e.g. `cistern_nine`.
    pub name: String,
    /// Display name from the level file, e.g. `Cistern Nine`.
    pub title: String,
    pub kind: LevelKind,
}

impl LevelInfo {
    pub fn from_def(name: &str, def: &LevelDef) -> Self {
        Self {
            name: name.to_string(),
            title: def.name.clone(),
            kind: def.kind,
        }
    }

    /// Playable in every mode (sandbox / test level).
    pub fn any_mode(&self) -> bool {
        ANY_MODE_LEVELS.contains(&self.name.as_str())
    }

    /// Upper-case display name ("CISTERN NINE"), falling back to the file name.
    pub fn display_title(&self) -> String {
        if self.title.trim().is_empty() {
            level_display_name(&self.name)
        } else {
            self.title.to_uppercase()
        }
    }
}

/// "movement_test" -> "MOVEMENT TEST".
pub fn level_display_name(name: &str) -> String {
    name.replace(['_', '-'], " ").to_uppercase()
}

/// Every level the game can load: `assets/levels/*.ron` (files that fail to parse are skipped
/// with a warning) plus the built-in level, sorted by name.
pub fn scan_levels() -> Vec<LevelInfo> {
    let mut names = LevelDef::list();
    if !names.iter().any(|n| n == crate::sim::BUILTIN_LEVEL) {
        names.push(crate::sim::BUILTIN_LEVEL.to_string());
    }
    names.sort();
    names
        .iter()
        .filter_map(|name| match crate::sim::load_level(name) {
            Ok(def) => Some(LevelInfo::from_def(name, &def)),
            Err(err) => {
                log::warn!("level '{name}' skipped: {err:#}");
                None
            }
        })
        .collect()
}

/// The rules of the next / current match. Chosen by the host in the lobby, validated by the
/// server ([`MatchSettings::clamp`] + [`MatchSettings::validate`]) and stored in
/// `SimWorld::match_settings` for the game-mode rules.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct MatchSettings {
    pub mode: GameMode,
    /// Level name (file stem / built-in).
    pub level: String,
    /// Player slots (1..=8; co-op 1..=4).
    pub max_players: u8,
    /// Deathmatch: frags that win the match (0 = no limit).
    pub frag_limit: u32,
    /// Deathmatch: match length in seconds (0 = no limit).
    pub time_limit_secs: u32,
    /// Co-op: players' weapons hurt teammates (knockback always applies).
    pub friendly_fire: bool,
    /// PvE modes: how hard the Moulds are.
    pub difficulty: Difficulty,
}

impl Default for MatchSettings {
    fn default() -> Self {
        Self {
            mode: GameMode::Campaign,
            level: crate::sim::BUILTIN_LEVEL.to_string(),
            max_players: MAX_COOP_PLAYERS,
            frag_limit: DEFAULT_FRAG_LIMIT,
            time_limit_secs: DEFAULT_TIME_LIMIT_SECS,
            friendly_fire: false,
            difficulty: Difficulty::Normal,
        }
    }
}

impl MatchSettings {
    pub fn new(mode: GameMode, level: &str) -> Self {
        Self {
            mode,
            level: level.to_string(),
            max_players: mode.max_players(),
            ..Default::default()
        }
    }

    /// Clamps every number into its range and cleans the level name. Never fails; what can't
    /// be fixed by clamping is reported by [`MatchSettings::validate`].
    pub fn clamp(&mut self) {
        self.max_players = self.max_players.clamp(1, self.mode.max_players());
        self.frag_limit = self.frag_limit.min(MAX_FRAG_LIMIT);
        self.time_limit_secs = self.time_limit_secs.min(MAX_TIME_LIMIT_SECS);
        self.level = self
            .level
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-'))
            .take(MAX_LEVEL_NAME_CHARS)
            .collect();
    }

    /// Checks that the level exists in `levels` and suits the mode. The error is a player-facing
    /// sentence.
    pub fn validate(&self, levels: &[LevelInfo]) -> Result<(), String> {
        let Some(level) = levels.iter().find(|l| l.name == self.level) else {
            return Err(format!(
                "Level '{}' doesn't exist on this server",
                self.level
            ));
        };
        if !self.mode.accepts(level) {
            return Err(match self.mode {
                GameMode::Campaign => format!(
                    "{} is not a campaign level: co-op needs a campaign level",
                    level.display_title()
                ),
                mode => format!(
                    "{} is not an arena: {} needs an arena",
                    level.display_title(),
                    mode.title().to_lowercase()
                ),
            });
        }
        if self.max_players == 0 || self.max_players > self.mode.max_players() {
            return Err(format!(
                "{} is for up to {} players",
                self.mode.title(),
                self.mode.max_players()
            ));
        }
        Ok(())
    }

    /// Makes the settings fit `levels`: clamps, keeps the mode and picks the first level of
    /// that mode if the current one doesn't fit (or the mode of the level if nothing does).
    pub fn fit_to(&mut self, levels: &[LevelInfo]) {
        self.clamp();
        if self.validate(levels).is_ok() {
            return;
        }
        let mode = self.mode;
        if let Some(l) = levels
            .iter()
            .find(|l| mode.accepts(l) && !l.any_mode())
            .or_else(|| levels.iter().find(|l| mode.accepts(l)))
        {
            self.level = l.name.clone();
        } else if let Some(l) = levels.iter().find(|l| l.name == self.level) {
            self.mode = GameMode::for_level_kind(l.kind);
        }
        self.clamp();
    }

    /// One-line summary, e.g. `DEATHMATCH // CISTERN NINE // 20 FRAGS // 10 MIN` or
    /// `GRIND // KNELL YARD // HARD`.
    pub fn summary(&self, levels: &[LevelInfo]) -> String {
        let level = levels
            .iter()
            .find(|l| l.name == self.level)
            .map_or_else(|| level_display_name(&self.level), LevelInfo::display_title);
        let mut s = format!("{} // {level}", self.mode.title());
        if self.mode.has_score_limits() {
            s.push_str(&format!(
                " // {} // {}",
                frag_limit_text(self.frag_limit),
                time_limit_text(self.time_limit_secs)
            ));
        } else if self.mode.has_time_limit() {
            s.push_str(&format!(" // {}", time_limit_text(self.time_limit_secs)));
        }
        if self.mode.has_difficulty() {
            s.push_str(&format!(" // {}", self.difficulty.title()));
        }
        s
    }
}

/// "20 FRAGS" / "NO FRAG LIMIT".
pub fn frag_limit_text(frags: u32) -> String {
    if frags == 0 {
        "NO FRAG LIMIT".into()
    } else {
        format!("{frags} FRAGS")
    }
}

/// "10 MIN" / "1:30 MIN" / "NO TIME LIMIT".
pub fn time_limit_text(secs: u32) -> String {
    match (secs / 60, secs % 60) {
        (0, 0) => "NO TIME LIMIT".into(),
        (m, 0) => format!("{m} MIN"),
        (m, s) => format!("{m}:{s:02} MIN"),
    }
}

/// Where the server is in the match flow.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum LobbyPhase {
    /// Players gather, ready up, the host picks mode / level / limits. No level is running.
    #[default]
    Lobby,
    /// Match starts in `secs` seconds (settings are locked; the host can cancel).
    Countdown { secs: u8 },
    /// The level is running; players joining now go straight in.
    InMatch,
    /// The match is over (results screen); the level keeps running behind it until the host
    /// continues (or a timeout) and everyone returns to the lobby.
    Results,
}

impl LobbyPhase {
    /// A level is loaded and simulated (players have bodies).
    pub fn level_running(self) -> bool {
        matches!(self, LobbyPhase::InMatch | LobbyPhase::Results)
    }

    /// Players are in the lobby screen.
    pub fn in_lobby(self) -> bool {
        !self.level_running()
    }
}

/// One player as the lobby / scoreboard shows them.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LobbyPlayer {
    pub id: PlayerId,
    pub name: String,
    pub ready: bool,
    /// Index into the client's player colour palette.
    pub color: u8,
    /// Round-trip time measured by the server (renet), ms. 0 for bots and the listen-server
    /// host's own client is ~0.
    pub ping_ms: u16,
    pub is_host: bool,
    /// Server-side bot (always ready, never host).
    pub bot: bool,
}

/// Everything the lobby screen shows. Broadcast on change and once per second (pings).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LobbyState {
    pub phase: LobbyPhase,
    pub settings: MatchSettings,
    /// Ascending id.
    pub players: Vec<LobbyPlayer>,
    pub host: Option<PlayerId>,
    /// The server has a lobby. `false` on single-player / no-lobby servers: the level runs
    /// from the start and "return to lobby" restarts it.
    pub lobby_enabled: bool,
    /// The server starts the match by itself once every player is ready.
    pub autostart: bool,
    /// Seconds until the results screen returns to the lobby by itself (Results phase).
    pub results_timeout: Option<u16>,
}

impl LobbyState {
    pub fn player(&self, id: PlayerId) -> Option<&LobbyPlayer> {
        self.players.iter().find(|p| p.id == id)
    }

    pub fn is_host(&self, id: PlayerId) -> bool {
        self.host == Some(id)
    }

    /// Every player is ready (and there is at least one).
    pub fn all_ready(&self) -> bool {
        !self.players.is_empty() && self.players.iter().all(|p| p.ready)
    }

    pub fn ready_count(&self) -> usize {
        self.players.iter().filter(|p| p.ready).count()
    }

    pub fn ping(&self, id: PlayerId) -> Option<u16> {
        self.player(id).map(|p| p.ping_ms)
    }
}

/// Client -> server lobby messages (inside `ClientMessage::Lobby`). Everything except
/// `ClaimHost` is host-only; the server validates and rate-limits all of them.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum LobbyRequest {
    /// The listen-server host's own client proves it is the host with the secret the session
    /// gave the in-process server (`LobbyConfig::host_token`).
    ClaimHost { token: u64 },
    /// New match settings (Lobby phase only).
    SetSettings(MatchSettings),
    /// Start the countdown: when everyone is ready, or regardless with `force`.
    StartMatch { force: bool },
    /// Abort a running countdown.
    CancelCountdown,
    /// End the running match (or leave the results screen) and bring everyone to the lobby.
    ReturnToLobby,
    /// Remove a player from the server.
    Kick { player: PlayerId },
}

/// Server -> client lobby messages (inside `ServerMessage::Lobby`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum LobbyUpdate {
    State(LobbyState),
    /// Levels the server can run (sent on connect).
    Levels(Vec<LevelInfo>),
    /// Feedback for the sender of a rejected request ("only the host can...").
    Notice(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn catalog() -> Vec<LevelInfo> {
        vec![
            LevelInfo {
                name: "cistern_nine".into(),
                title: "Cistern Nine".into(),
                kind: LevelKind::Arena,
            },
            LevelInfo {
                name: "crucible_of_veins".into(),
                title: "Crucible of Veins".into(),
                kind: LevelKind::Campaign,
            },
            LevelInfo {
                name: "movement_test".into(),
                title: "Movement Test".into(),
                kind: LevelKind::Campaign,
            },
            LevelInfo {
                name: "proving_ground".into(),
                title: "Proving Ground".into(),
                kind: LevelKind::Arena,
            },
        ]
    }

    #[test]
    fn modes_accept_matching_levels_and_sandboxes() {
        let levels = catalog();
        let ok = |mode, level: &str| MatchSettings::new(mode, level).validate(&levels);
        assert!(ok(GameMode::Deathmatch, "cistern_nine").is_ok());
        assert!(ok(GameMode::Campaign, "crucible_of_veins").is_ok());
        for sandbox in ANY_MODE_LEVELS {
            assert!(ok(GameMode::Deathmatch, sandbox).is_ok());
            assert!(ok(GameMode::Campaign, sandbox).is_ok());
        }
        let err = ok(GameMode::Deathmatch, "crucible_of_veins").unwrap_err();
        assert!(err.contains("arena"), "{err}");
        let err = ok(GameMode::Campaign, "cistern_nine").unwrap_err();
        assert!(err.contains("campaign"), "{err}");
        assert!(ok(GameMode::Campaign, "nope").is_err());
    }

    #[test]
    fn clamp_limits_values() {
        let mut s = MatchSettings {
            mode: GameMode::Campaign,
            level: "crucible_of_veins/../../etc".into(),
            max_players: 99,
            frag_limit: 100_000,
            time_limit_secs: u32::MAX,
            friendly_fire: true,
            difficulty: Difficulty::Brutal,
        };
        s.clamp();
        assert_eq!(s.max_players, MAX_COOP_PLAYERS);
        assert_eq!(s.frag_limit, MAX_FRAG_LIMIT);
        assert_eq!(s.time_limit_secs, MAX_TIME_LIMIT_SECS);
        assert_eq!(s.level, "crucible_of_veinsetc");
        s.mode = GameMode::Deathmatch;
        s.max_players = 0;
        s.clamp();
        assert_eq!(s.max_players, 1);
        s.max_players = 200;
        s.clamp();
        assert_eq!(s.max_players, MAX_PLAYERS);
    }

    #[test]
    fn fit_to_picks_a_level_of_the_mode() {
        let levels = catalog();
        let mut s = MatchSettings::new(GameMode::Deathmatch, "crucible_of_veins");
        s.fit_to(&levels);
        assert_eq!(s.level, "cistern_nine");
        let mut s = MatchSettings::new(GameMode::Campaign, "cistern_nine");
        s.max_players = 8;
        s.fit_to(&levels);
        assert_eq!(s.level, "crucible_of_veins");
        assert_eq!(s.max_players, 4);
        assert!(s.validate(&levels).is_ok());
    }

    #[test]
    fn parse_and_texts() {
        assert_eq!(GameMode::parse("DM"), Some(GameMode::Deathmatch));
        assert_eq!(GameMode::parse(" Campaign "), Some(GameMode::Campaign));
        assert_eq!(GameMode::parse("ctf"), None);
        assert_eq!(time_limit_text(600), "10 MIN");
        assert_eq!(time_limit_text(90), "1:30 MIN");
        assert_eq!(time_limit_text(0), "NO TIME LIMIT");
        assert_eq!(frag_limit_text(0), "NO FRAG LIMIT");
        let s = MatchSettings::new(GameMode::Deathmatch, "cistern_nine");
        assert_eq!(
            s.summary(&catalog()),
            "DEATHMATCH // CISTERN NINE // 20 FRAGS // 10 MIN"
        );
        let mut s = MatchSettings::new(GameMode::Grind, "cistern_nine");
        s.difficulty = Difficulty::Hard;
        assert_eq!(s.summary(&catalog()), "GRIND // CISTERN NINE // HARD");
        let s = MatchSettings::new(GameMode::GunGame, "cistern_nine");
        assert_eq!(s.summary(&catalog()), "GUN GAME // CISTERN NINE // 10 MIN");
        assert_eq!(Difficulty::parse(" Inferno"), Some(Difficulty::Inferno));
        assert_eq!(Difficulty::parse("nightmare"), None);
        for (i, d) in Difficulty::ALL.iter().enumerate() {
            assert_eq!(d.index(), i);
        }
    }

    #[test]
    fn phases() {
        assert!(LobbyPhase::Lobby.in_lobby());
        assert!(LobbyPhase::Countdown { secs: 2 }.in_lobby());
        assert!(LobbyPhase::InMatch.level_running());
        assert!(LobbyPhase::Results.level_running());
    }

    #[test]
    fn messages_round_trip() {
        let state = LobbyState {
            phase: LobbyPhase::Countdown { secs: 3 },
            settings: MatchSettings::new(GameMode::Deathmatch, "cistern_nine"),
            players: vec![LobbyPlayer {
                id: PlayerId(2),
                name: "Neo".into(),
                ready: true,
                color: 2,
                ping_ms: 42,
                is_host: true,
                bot: false,
            }],
            host: Some(PlayerId(2)),
            lobby_enabled: true,
            autostart: false,
            results_timeout: None,
        };
        let msg = crate::protocol::ServerMessage::Lobby(LobbyUpdate::State(state));
        let back: crate::protocol::ServerMessage =
            crate::protocol::decode_server_message(&crate::protocol::encode(&msg)).unwrap();
        assert_eq!(back, msg);
        let req = crate::protocol::ClientMessage::Lobby(LobbyRequest::SetSettings(
            MatchSettings::default(),
        ));
        let back: crate::protocol::ClientMessage =
            crate::protocol::decode_client_message(&crate::protocol::encode(&req)).unwrap();
        assert_eq!(back, req);
    }

    #[test]
    fn scan_finds_the_builtin_level() {
        let levels = scan_levels();
        let pg = levels
            .iter()
            .find(|l| l.name == crate::sim::BUILTIN_LEVEL)
            .expect("built-in level is always there");
        assert!(pg.any_mode());
    }
}
