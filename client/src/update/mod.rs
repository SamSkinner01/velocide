//! Update check and self-update (docs/distribution.md "Update check").
//!
//! Release builds ask GitHub once per launch, on a background thread, which commit the latest
//! release was built from; when it isn't this build's, the main menu offers UPDATE NOW / LATER
//! (`menus::update_prompt`). UPDATE NOW downloads this platform's game next to the running
//! one, verifies it, swaps it in and restarts: the menu quits the app the normal way and
//! `main` starts the new executable with the same arguments once the window is closed
//! ([`take_relaunch`]). If anything fails the prompt says why and offers the release page.
//!
//! - [`github`]: the API (release JSON, the release's commit, the asset per platform).
//! - [`install`]: download with size / SHA-256 check, the swap, removing `<exe>.old`.
//!
//! The main thread never waits: the workers write the [`State`] into a mutex that the menu
//! reads each frame. Dev builds, `--bot` / `--quit-after` / `--capture` runs,
//! `VELOCIDE_NO_UPDATE_CHECK=1` and the "Check for updates" setting never touch the network
//! ([`skip_reason`]).

pub mod github;
pub mod install;

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use anyhow::{Context, anyhow};
use shared::build_info;

pub use github::{Asset, Latest, Release};

use crate::cli::LaunchOptions;
use crate::settings::Settings;

/// Where this launch's update check stands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum State {
    /// Not checking this launch ([`skip_reason`]).
    Off,
    Checking,
    UpToDate,
    /// Offline, rate limited, a bad answer...: logged, shown nowhere.
    CheckFailed,
    Available(Latest),
    /// UPDATE NOW: `done` of `total` bytes so far.
    Downloading {
        latest: Latest,
        done: u64,
        total: u64,
    },
    /// Swapped in: the game restarts into it.
    Installed(Latest),
    /// UPDATE NOW failed; the prompt offers the release page.
    Failed {
        latest: Latest,
        reason: String,
    },
}

/// What the main menu shows (from [`Updater::prompt`]). Versions are labels like
/// `build-43 (1a2b3c4)`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Prompt {
    Available {
        new: String,
        running: String,
    },
    Downloading {
        new: String,
        done: u64,
        total: u64,
    },
    Failed {
        new: String,
        reason: String,
        /// The release page, for players to download it themselves.
        page: String,
    },
    Installed {
        new: String,
    },
}

/// What a check compares against and where it asks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UpdateConfig {
    /// API base URL: `https://api.github.com`, or `VELOCIDE_UPDATE_API` (tests).
    pub api: String,
    /// `owner/name`.
    pub repo: String,
    /// The running build.
    pub version: String,
    pub commit: String,
    /// The running executable: replaced by the update, `<exe>.new` next to it.
    pub exe: PathBuf,
}

impl UpdateConfig {
    /// This build ([`build_info`]), the executable it runs from and the API from the
    /// environment. `None` if the executable's path is unknown.
    pub fn from_build() -> Option<Self> {
        let exe = match std::env::current_exe() {
            Ok(exe) => exe,
            Err(e) => {
                log::warn!("no update check: can't find the executable ({e})");
                return None;
            }
        };
        Some(Self {
            api: api_from_env(),
            repo: build_info::REPO.to_string(),
            version: build_info::VERSION.to_string(),
            commit: build_info::COMMIT.to_string(),
            exe,
        })
    }

    fn running_label(&self) -> String {
        build_info::version_label(&self.version, &self.commit)
    }
}

/// `VELOCIDE_UPDATE_API` (read at run time), else GitHub's API.
pub fn api_from_env() -> String {
    std::env::var("VELOCIDE_UPDATE_API")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| github::DEFAULT_API.to_string())
}

/// Why this launch doesn't check for updates (`None`: it does). The settings toggle comes
/// last: only then can turning it on start the check ([`Updater::set_enabled`]).
pub fn skip_reason(
    release_build: bool,
    env_off: bool,
    launch: &LaunchOptions,
    enabled: bool,
) -> Option<&'static str> {
    if !release_build {
        Some("dev build")
    } else if env_off {
        Some("VELOCIDE_NO_UPDATE_CHECK is set")
    } else if launch.bot || launch.quit_after.is_some() || launch.capture.is_some() {
        Some("test run (--bot / --quit-after / --capture)")
    } else if !enabled {
        Some("turned off in settings")
    } else {
        None
    }
}

/// `VELOCIDE_NO_UPDATE_CHECK` set to anything but empty or `0`.
fn env_off() -> bool {
    std::env::var_os("VELOCIDE_NO_UPDATE_CHECK").is_some_and(|v| !v.is_empty() && v != "0")
}

struct Shared {
    state: Mutex<State>,
    /// LATER during the download: the worker stops at its next chunk.
    cancel: AtomicBool,
}

impl Shared {
    fn state(&self) -> MutexGuard<'_, State> {
        // A panicking worker can't leave the state half-written: keep using it.
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn set(&self, state: State) {
        *self.state() = state;
    }
}

/// The update check of this launch and its prompt (`Common::update`).
pub struct Updater {
    config: Option<UpdateConfig>,
    shared: Arc<Shared>,
    /// LATER: no prompt until the next launch.
    dismissed: bool,
    /// The "Check for updates" setting: off hides a new release (not a running download).
    enabled: bool,
    /// Everything but the setting allowed a check: turning it on starts one.
    deferred: bool,
    /// The browser couldn't be opened (shown with the failure).
    open_error: Option<String>,
}

impl Updater {
    /// Never checks (tools, tests, dev builds).
    pub fn off() -> Self {
        Self::showing(None, State::Off)
    }

    /// A fixed state without a check, e.g. to capture the prompt in screenshots. UPDATE NOW
    /// from it really downloads, from the release it names, to `config.exe`.
    pub fn showing(config: Option<UpdateConfig>, state: State) -> Self {
        Self {
            config,
            shared: Arc::new(Shared {
                state: Mutex::new(state),
                cancel: AtomicBool::new(false),
            }),
            dismissed: false,
            enabled: true,
            deferred: false,
            open_error: None,
        }
    }

    /// This launch's check: started now unless [`skip_reason`] says no.
    pub fn for_launch(launch: &LaunchOptions, settings: &Settings) -> Self {
        let reason = skip_reason(
            build_info::is_release(),
            env_off(),
            launch,
            settings.check_for_updates,
        );
        match (reason, UpdateConfig::from_build()) {
            (None, Some(config)) => Self::start(config),
            (Some(reason), config) => {
                log::info!("not checking for updates: {reason}");
                let mut updater = Self::showing(config, State::Off);
                updater.enabled = settings.check_for_updates;
                updater.deferred = !settings.check_for_updates
                    && skip_reason(build_info::is_release(), env_off(), launch, true).is_none();
                updater
            }
            (None, None) => Self::off(),
        }
    }

    /// Checks now, on a background thread.
    pub fn start(config: UpdateConfig) -> Self {
        let mut updater = Self::showing(Some(config), State::Off);
        updater.spawn_check();
        updater
    }

    fn spawn_check(&mut self) {
        let Some(config) = self.config.clone() else {
            return;
        };
        self.shared.set(State::Checking);
        let shared = self.shared.clone();
        log::info!(
            "checking {} for a newer release than {}",
            config.repo,
            config.running_label()
        );
        let spawned = std::thread::Builder::new()
            .name("update-check".into())
            .spawn(move || shared.set(check(&config)));
        if let Err(e) = spawned {
            log::warn!("update check: no thread ({e})");
            self.shared.set(State::CheckFailed);
        }
    }

    /// The "Check for updates" setting (applied with every settings change): off hides a new
    /// release; on shows it again, or starts the check if only the setting held it back.
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
        if enabled && self.deferred {
            self.deferred = false;
            self.spawn_check();
        }
    }

    pub fn state(&self) -> State {
        self.shared.state().clone()
    }

    /// What the main menu should show now (`None`: nothing, e.g. after LATER).
    pub fn prompt(&self) -> Option<Prompt> {
        let state = self.shared.state();
        let new = |l: &Latest| l.label();
        let prompt = match &*state {
            State::Off | State::Checking | State::UpToDate | State::CheckFailed => None,
            State::Available(_) if !self.enabled => None,
            State::Available(latest) => Some(Prompt::Available {
                new: new(latest),
                running: self
                    .config
                    .as_ref()
                    .map_or_else(build_info::label, UpdateConfig::running_label),
            }),
            State::Downloading {
                latest,
                done,
                total,
            } => Some(Prompt::Downloading {
                new: new(latest),
                done: *done,
                total: *total,
            }),
            State::Failed { latest, reason } => {
                let mut reason = reason.clone();
                if let Some(e) = &self.open_error {
                    reason = format!("{reason}. {e}");
                }
                Some(Prompt::Failed {
                    new: new(latest),
                    reason,
                    page: self.release_page(&latest.release),
                })
            }
            // Shown even after a late LATER: the new game is in place, so restart into it.
            State::Installed(latest) => return Some(Prompt::Installed { new: new(latest) }),
        };
        prompt.filter(|_| !self.dismissed)
    }

    /// UPDATE NOW: download, verify and swap in on a background thread.
    pub fn update_now(&mut self) {
        let Some(config) = self.config.clone() else {
            return;
        };
        let latest = match &*self.shared.state() {
            State::Available(l) | State::Failed { latest: l, .. } => l.clone(),
            _ => return,
        };
        self.open_error = None;
        self.shared.cancel.store(false, Ordering::SeqCst);
        let total = latest.release.game_asset().map_or(0, |a| a.size);
        self.shared.set(State::Downloading {
            latest: latest.clone(),
            done: 0,
            total,
        });
        let shared = self.shared.clone();
        let spawned = std::thread::Builder::new()
            .name("update-download".into())
            .spawn({
                let latest = latest.clone();
                move || install_update(&shared, &config, latest)
            });
        if let Err(e) = spawned {
            self.shared.set(State::Failed {
                latest,
                reason: format!("no thread for the download ({e})"),
            });
        }
    }

    /// LATER: no prompt until the next launch (a running download stops).
    pub fn later(&mut self) {
        self.dismissed = true;
        self.shared.cancel.store(true, Ordering::SeqCst);
    }

    /// Opens the latest release's page in the browser (after a failure). If that fails too,
    /// the prompt says so next to the page's address.
    pub fn open_release_page(&mut self) {
        let page = match &*self.shared.state() {
            State::Available(l) | State::Failed { latest: l, .. } => self.release_page(&l.release),
            _ => return,
        };
        match open_in_browser(&page) {
            Ok(()) => {
                log::info!("opened {page}");
                self.dismissed = true;
            }
            Err(e) => {
                log::warn!("couldn't open {page}: {e:#}");
                self.open_error = Some(format!("Couldn't open a browser: {e:#}"));
            }
        }
    }

    /// The new game is in place: ask `main` to start it once the app has shut down. True if
    /// the app should quit now.
    pub fn restart(&mut self) -> bool {
        let installed = matches!(*self.shared.state(), State::Installed(_));
        match (&self.config, installed) {
            (Some(config), true) => {
                request_relaunch(config.exe.clone());
                true
            }
            _ => false,
        }
    }

    /// The release's own page when it is a plain https link, else the repository's newest
    /// release (`html_url` comes from the network and is handed to the OS).
    fn release_page(&self, release: &Release) -> String {
        if release.html_url.starts_with("https://")
            && !release.html_url.contains(char::is_whitespace)
        {
            release.html_url.clone()
        } else {
            let repo = self.config.as_ref().map_or(build_info::REPO, |c| &c.repo);
            format!("https://github.com/{repo}/releases/latest")
        }
    }
}

/// One check: the latest release and whether it is another build.
fn check(config: &UpdateConfig) -> State {
    let api = github::Api::new(&config.api, &config.repo, &config.version);
    match api.latest() {
        Ok(latest) if github::is_update(&latest.commit, &config.commit) => {
            log::info!(
                "update available: {} (running {})",
                latest.label(),
                config.running_label()
            );
            State::Available(latest)
        }
        Ok(latest) => {
            log::info!("up to date ({})", latest.label());
            State::UpToDate
        }
        Err(e) => {
            log::warn!("update check failed: {e:#}");
            State::CheckFailed
        }
    }
}

/// The download worker: `<exe>.new` → verified → swapped in → [`State::Installed`].
fn install_update(shared: &Shared, config: &UpdateConfig, latest: Latest) {
    let new = install::new_path(&config.exe);
    let result = (|| {
        let asset = latest.release.game_asset().ok_or_else(|| {
            anyhow!(
                "{} has no download for {} {}",
                latest.release.tag_name,
                std::env::consts::OS,
                std::env::consts::ARCH
            )
        })?;
        log::info!(
            "downloading {} ({} bytes) to {}",
            asset.browser_download_url,
            asset.size,
            new.display()
        );
        install::download(asset, &new, &config.version, |done| {
            if shared.cancel.load(Ordering::SeqCst) {
                return false;
            }
            if let State::Downloading { done: d, .. } = &mut *shared.state() {
                *d = done;
            }
            true
        })?;
        if shared.cancel.load(Ordering::SeqCst) {
            let _ = std::fs::remove_file(&new);
            anyhow::bail!("cancelled");
        }
        install::swap_in(&new, &config.exe).inspect_err(|_| {
            // Keep the download only if it is now the one copy of the game.
            if config.exe.exists() {
                let _ = std::fs::remove_file(&new);
            }
        })
    })();
    match result {
        Ok(()) => {
            log::info!("installed {} at {}", latest.label(), config.exe.display());
            shared.set(State::Installed(latest));
        }
        Err(_) if shared.cancel.load(Ordering::SeqCst) => {
            log::info!("update download cancelled");
            shared.set(State::Available(latest));
        }
        Err(e) => {
            log::warn!("update failed: {e:#}");
            shared.set(State::Failed {
                latest,
                reason: format!("{e:#}"),
            });
        }
    }
}

static RELAUNCH: Mutex<Option<PathBuf>> = Mutex::new(None);

/// Asks `main` to start `exe` (with this run's arguments) after the app has shut down.
pub fn request_relaunch(exe: PathBuf) {
    if let Ok(mut slot) = RELAUNCH.lock() {
        *slot = Some(exe);
    }
}

/// The executable to start now that the app has shut down, if an update asked for it.
pub fn take_relaunch() -> Option<PathBuf> {
    RELAUNCH.lock().ok().and_then(|mut slot| slot.take())
}

/// Starts `exe` with this process's arguments (it outlives us).
pub fn relaunch(exe: &Path) -> anyhow::Result<()> {
    Command::new(exe)
        .args(std::env::args_os().skip(1))
        .spawn()
        .with_context(|| format!("can't start {}", exe.display()))?;
    Ok(())
}

/// Opens `url` in the default browser. No shell is involved, so `&` and `?` in the URL are
/// safe: Windows hands it to `url.dll`'s protocol handler (as `start` would, minus `cmd`'s
/// quoting), macOS to `open`, other systems to `xdg-open`.
pub fn open_in_browser(url: &str) -> anyhow::Result<()> {
    let mut command = if cfg!(windows) {
        // From System32: the game's own folder comes first in Windows' search order.
        let rundll = std::env::var_os("SystemRoot")
            .map(|root| PathBuf::from(root).join("System32").join("rundll32.exe"))
            .filter(|p| p.exists())
            .unwrap_or_else(|| PathBuf::from("rundll32.exe"));
        let mut c = Command::new(rundll);
        c.arg("url.dll,FileProtocolHandler");
        c
    } else if cfg!(target_os = "macos") {
        Command::new("open")
    } else {
        Command::new("xdg-open")
    };
    let mut child = command
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("no browser launcher")?;
    // Reap it when it exits (xdg-open returns quickly; the browser lives on).
    let _ = std::thread::Builder::new()
        .name("open-browser".into())
        .spawn(move || child.wait());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const RUNNING: &str = "1111111111111111111111111111111111111111";
    const NEW: &str = "2222222222222222222222222222222222222222";

    fn config() -> UpdateConfig {
        UpdateConfig {
            // Nothing listens there: a test that reached the network would fail.
            api: "http://127.0.0.1:9".into(),
            repo: "someone/velocide".into(),
            version: "build-42".into(),
            commit: RUNNING.into(),
            exe: std::env::temp_dir().join("velocide-update-unit-test-exe"),
        }
    }

    fn latest() -> Latest {
        Latest {
            release: Release {
                tag_name: "build-43".into(),
                target_commitish: NEW.into(),
                html_url: "https://github.com/someone/velocide/releases/tag/build-43".into(),
                assets: Vec::new(),
            },
            commit: NEW.into(),
        }
    }

    #[test]
    fn who_checks() {
        let menu = LaunchOptions::default();
        assert_eq!(skip_reason(true, false, &menu, true), None);
        assert_eq!(skip_reason(false, false, &menu, true), Some("dev build"));
        assert!(skip_reason(true, true, &menu, true).is_some());
        assert!(skip_reason(true, false, &menu, false).is_some());
        for test_run in [
            LaunchOptions {
                bot: true,
                ..Default::default()
            },
            LaunchOptions {
                quit_after: Some(5.0),
                ..Default::default()
            },
            LaunchOptions {
                capture: Some("shot.png".into()),
                ..Default::default()
            },
        ] {
            assert!(skip_reason(true, false, &test_run, true).is_some());
        }
        // `cargo test` builds are dev builds: the real launch never checks.
        if !build_info::is_release() {
            let u = Updater::for_launch(&menu, &Settings::default());
            assert_eq!(u.state(), State::Off);
        }
    }

    #[test]
    fn prompt_follows_the_state() {
        let mut u = Updater::showing(Some(config()), State::UpToDate);
        assert_eq!(u.prompt(), None);
        u.shared.set(State::Available(latest()));
        assert_eq!(
            u.prompt(),
            Some(Prompt::Available {
                new: "build-43 (2222222)".into(),
                running: "build-42 (1111111)".into(),
            })
        );
        u.later();
        assert_eq!(u.prompt(), None, "LATER: until the next launch");

        let u = Updater::showing(
            Some(config()),
            State::Failed {
                latest: latest(),
                reason: "checksum mismatch".into(),
            },
        );
        match u.prompt() {
            Some(Prompt::Failed { reason, page, .. }) => {
                assert_eq!(reason, "checksum mismatch");
                assert_eq!(page, latest().release.html_url);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn release_page_is_https_or_the_repo() {
        let u = Updater::showing(Some(config()), State::Off);
        let mut r = latest().release;
        assert_eq!(u.release_page(&r), r.html_url);
        for bad in [
            "file:///C:/Windows/System32/calc.exe",
            "",
            "http://example.com",
        ] {
            r.html_url = bad.into();
            assert_eq!(
                u.release_page(&r),
                "https://github.com/someone/velocide/releases/latest"
            );
        }
    }

    #[test]
    fn no_asset_for_this_platform_fails_with_a_reason() {
        // The release has no assets at all: UPDATE NOW fails without any request.
        let mut u = Updater::showing(Some(config()), State::Available(latest()));
        u.update_now();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            if let Some(Prompt::Failed { reason, .. }) = u.prompt() {
                assert!(reason.contains("no download for"), "{reason}");
                break;
            }
            assert!(std::time::Instant::now() < deadline, "{:?}", u.state());
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(!u.restart(), "nothing to restart into");
        assert!(!install::new_path(&config().exe).exists());
    }

    #[test]
    fn setting_hides_a_new_release_but_not_a_download() {
        let mut u = Updater::showing(Some(config()), State::Available(latest()));
        u.set_enabled(false);
        assert_eq!(u.prompt(), None);
        // On again: the same answer (no second check: it isn't deferred).
        u.set_enabled(true);
        assert!(matches!(u.prompt(), Some(Prompt::Available { .. })));
        assert!(matches!(u.state(), State::Available(_)));
        let mut u = Updater::showing(
            Some(config()),
            State::Downloading {
                latest: latest(),
                done: 1,
                total: 2,
            },
        );
        u.set_enabled(false);
        assert!(matches!(u.prompt(), Some(Prompt::Downloading { .. })));
    }

    #[test]
    fn relaunch_request_is_taken_once() {
        request_relaunch(PathBuf::from("/games/velocide"));
        assert_eq!(take_relaunch(), Some(PathBuf::from("/games/velocide")));
        assert_eq!(take_relaunch(), None);
    }
}
