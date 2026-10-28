//! The updater end to end against a local stand-in for GitHub (a std `TcpListener` on
//! 127.0.0.1 with canned answers), found through `VELOCIDE_UPDATE_API`: check → download (via
//! a redirect, with progress) → verify → swap into a scratch folder, plus a checksum mismatch,
//! a missing file (404), a cancelled download, an up-to-date build and a repository without
//! releases. Nothing here reaches the internet.
//!
//! One test on purpose: it sets an environment variable, which is only sound while no other
//! thread can read the environment.

#![cfg(all(
    target_arch = "x86_64",
    any(target_os = "linux", target_os = "windows")
))]

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use client::update::{self, Prompt, State, UpdateConfig, Updater, install};
use sha2::{Digest, Sha256};

const RUNNING: &str = "1111111111111111111111111111111111111111";
const NEW: &str = "2222222222222222222222222222222222222222";

/// One canned answer.
#[derive(Clone)]
struct Answer {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
    /// Sent in this many pieces with a pause between them (to see progress).
    pieces: usize,
}

impl Answer {
    fn ok(body: impl Into<Vec<u8>>) -> Self {
        Self {
            status: 200,
            headers: Vec::new(),
            body: body.into(),
            pieces: 1,
        }
    }

    fn json(text: String) -> Self {
        let mut a = Self::ok(text);
        a.headers
            .push(("Content-Type".into(), "application/json".into()));
        a
    }

    fn status(status: u16) -> Self {
        Self {
            status,
            ..Self::ok(r#"{"message": "Not Found"}"#)
        }
    }

    fn redirect(to: &str) -> Self {
        let mut a = Self::status(302);
        a.body.clear();
        a.headers.push(("Location".into(), to.into()));
        a
    }
}

/// A request as the server saw it: path and headers (names lowercased).
#[derive(Clone, Debug)]
struct Seen {
    path: String,
    headers: HashMap<String, String>,
}

struct MockGitHub {
    base: String,
    seen: Arc<Mutex<Vec<Seen>>>,
}

impl MockGitHub {
    /// Serves `routes` (path → answer) on a listener bound by the caller.
    fn serve(listener: TcpListener, routes: HashMap<String, Answer>) -> Self {
        let base = format!("http://{}", listener.local_addr().unwrap());
        let seen = Arc::new(Mutex::new(Vec::new()));
        let routes = Arc::new(routes);
        {
            let seen = seen.clone();
            std::thread::spawn(move || {
                for stream in listener.incoming().flatten() {
                    let (seen, routes) = (seen.clone(), routes.clone());
                    std::thread::spawn(move || answer(stream, &routes, &seen));
                }
            });
        }
        Self { base, seen }
    }

    fn seen(&self, path: &str) -> Vec<Seen> {
        let seen = self.seen.lock().unwrap();
        seen.iter().filter(|s| s.path == path).cloned().collect()
    }
}

fn answer(stream: TcpStream, routes: &HashMap<String, Answer>, seen: &Mutex<Vec<Seen>>) {
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut line = String::new();
    if reader.read_line(&mut line).is_err() {
        return;
    }
    let path = line.split_whitespace().nth(1).unwrap_or("").to_string();
    let mut headers = HashMap::new();
    loop {
        let mut h = String::new();
        if reader.read_line(&mut h).unwrap_or(0) == 0 || h.trim().is_empty() {
            break;
        }
        if let Some((k, v)) = h.split_once(':') {
            headers.insert(k.trim().to_ascii_lowercase(), v.trim().to_string());
        }
    }
    seen.lock().unwrap().push(Seen {
        path: path.clone(),
        headers,
    });
    let a = routes
        .get(&path)
        .cloned()
        .unwrap_or_else(|| Answer::status(404));
    let mut head = format!(
        "HTTP/1.1 {} X\r\nContent-Length: {}\r\nConnection: close\r\n",
        a.status,
        a.body.len()
    );
    for (k, v) in &a.headers {
        head.push_str(&format!("{k}: {v}\r\n"));
    }
    head.push_str("\r\n");
    let mut stream = stream;
    if stream.write_all(head.as_bytes()).is_err() {
        return;
    }
    let piece = a.body.len().div_ceil(a.pieces.max(1)).max(1);
    for chunk in a.body.chunks(piece) {
        if stream
            .write_all(chunk)
            .and_then(|_| stream.flush())
            .is_err()
        {
            return;
        }
        if a.pieces > 1 {
            std::thread::sleep(Duration::from_millis(40));
        }
    }
}

fn sha256(data: &[u8]) -> String {
    format!("sha256:{}", install::hex(&Sha256::digest(data)))
}

/// A `releases/latest` answer whose game assets (both platforms) point at `url`.
fn release_json(base: &str, commitish: &str, url: &str, size: usize, digest: &str) -> String {
    let asset = |name: &str| {
        format!(
            r#"{{"name": "{name}", "size": {size}, "digest": "{digest}",
                "browser_download_url": "{base}{url}", "state": "uploaded"}}"#
        )
    };
    format!(
        r#"{{"tag_name": "build-43", "target_commitish": "{commitish}",
            "html_url": "https://github.com/test/velocide/releases/tag/build-43",
            "draft": false, "prerelease": false,
            "assets": [{}, {}, {}]}}"#,
        asset("velocide-linux-x86_64"),
        asset("velocide-windows-x86_64.exe"),
        asset("velocide-server-linux-x86_64"),
    )
}

/// Polls the updater until `done` accepts its state (fails after 20 s).
fn wait_for(u: &Updater, what: &str, mut done: impl FnMut(&State) -> bool) -> State {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let state = u.state();
        if done(&state) {
            return state;
        }
        assert!(Instant::now() < deadline, "waiting for {what}: {state:?}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn scratch_exe() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("velocide-update-it-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(if cfg!(windows) {
        "velocide.exe"
    } else {
        "velocide"
    })
}

fn old_game(exe: &Path) {
    std::fs::write(exe, b"the old game").unwrap();
    let _ = std::fs::remove_file(install::old_path(exe));
}

#[test]
fn check_download_verify_swap() {
    // Bound before anything else runs, so the variable is set while this is the only thread.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    // SAFETY: the only test of this binary, and no thread of ours has started yet.
    unsafe { std::env::set_var("VELOCIDE_UPDATE_API", &base) };

    let game: Vec<u8> = (0..300_000u32).map(|i| (i * 31 % 251) as u8).collect();
    let digest = sha256(&game);
    let mut routes = HashMap::new();
    // Made from a branch: the commit comes from the tag.
    routes.insert(
        "/repos/test/good/releases/latest".into(),
        Answer::json(release_json(
            &base,
            "main",
            "/download/game",
            game.len(),
            &digest,
        )),
    );
    routes.insert(
        "/repos/test/good/commits/build-43".into(),
        Answer::json(format!(
            r#"{{"sha": "{NEW}", "commit": {{"message": "x"}}}}"#
        )),
    );
    // Release files are served from another host behind a redirect.
    routes.insert("/download/game".into(), Answer::redirect("/cdn/game"));
    routes.insert(
        "/cdn/game".into(),
        Answer {
            pieces: 8,
            ..Answer::ok(game.clone())
        },
    );
    routes.insert(
        "/repos/test/mismatch/releases/latest".into(),
        Answer::json(release_json(
            &base,
            NEW,
            "/cdn/game",
            game.len(),
            &sha256(b"another game"),
        )),
    );
    routes.insert(
        "/repos/test/missing/releases/latest".into(),
        Answer::json(release_json(
            &base,
            NEW,
            "/download/nothing",
            game.len(),
            &digest,
        )),
    );
    routes.insert(
        "/repos/test/current/releases/latest".into(),
        Answer::json(release_json(
            &base,
            RUNNING,
            "/cdn/game",
            game.len(),
            &digest,
        )),
    );
    let mock = MockGitHub::serve(listener, routes);
    assert_eq!(mock.base, base);

    let exe = scratch_exe();
    let config = |repo: &str| UpdateConfig {
        api: update::api_from_env(),
        repo: format!("test/{repo}"),
        version: "build-42".into(),
        commit: RUNNING.into(),
        exe: exe.clone(),
    };
    assert_eq!(config("good").api, base, "VELOCIDE_UPDATE_API is read");

    // ---- a new release: check, UPDATE NOW, progress, swap ----------------------------------
    old_game(&exe);
    let mut u = Updater::start(config("good"));
    let state = wait_for(&u, "the check", |s| !matches!(s, State::Checking));
    let State::Available(latest) = state else {
        panic!("expected an update: {state:?}");
    };
    assert_eq!(latest.commit, NEW, "resolved from the tag");
    assert_eq!(
        u.prompt(),
        Some(Prompt::Available {
            new: "build-43 (2222222)".into(),
            running: "build-42 (1111111)".into(),
        })
    );
    let api_call = &mock.seen("/repos/test/good/releases/latest")[0];
    assert_eq!(api_call.headers["user-agent"], "velocide/build-42");
    assert_eq!(api_call.headers["accept"], "application/vnd.github+json");
    assert_eq!(mock.seen("/repos/test/good/commits/build-43").len(), 1);

    u.update_now();
    let mut partial = None;
    let state = wait_for(&u, "the download", |s| {
        if let State::Downloading { done, total, .. } = s
            && *done > 0
            && done < total
        {
            partial = Some((*done, *total));
        }
        !matches!(s, State::Downloading { .. })
    });
    assert!(matches!(state, State::Installed(_)), "{state:?}");
    let (done, total) = partial.expect("progress between 0 and the size");
    assert_eq!(total, game.len() as u64);
    assert!(done < total);
    assert_eq!(
        std::fs::read(&exe).unwrap(),
        game,
        "the new game is in place"
    );
    assert!(!install::new_path(&exe).exists());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&exe).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o755);
    }
    assert_eq!(mock.seen("/cdn/game").len(), 1, "followed the redirect");
    assert!(matches!(u.prompt(), Some(Prompt::Installed { .. })));
    assert!(u.restart());
    assert_eq!(update::take_relaunch(), Some(exe.clone()));
    // Windows' swap leaves `<exe>.old`; startup removes it.
    install::remove_old(&exe);
    assert!(!install::old_path(&exe).exists());

    // ---- LATER during the download: it stops, nothing changes -------------------------------
    old_game(&exe);
    let mut u = Updater::start(config("good"));
    wait_for(&u, "the check", |s| matches!(s, State::Available(_)));
    u.update_now();
    wait_for(
        &u,
        "some progress",
        |s| matches!(s, State::Downloading { done, .. } if *done > 0),
    );
    u.later();
    assert_eq!(u.prompt(), None);
    wait_for(&u, "the cancel", |s| {
        !matches!(s, State::Downloading { .. })
    });
    assert_eq!(std::fs::read(&exe).unwrap(), b"the old game");
    assert!(!install::new_path(&exe).exists());

    // ---- checksum mismatch -----------------------------------------------------------------
    old_game(&exe);
    let mut u = Updater::start(config("mismatch"));
    wait_for(&u, "the check", |s| matches!(s, State::Available(_)));
    assert!(
        mock.seen("/repos/test/mismatch/commits/build-43")
            .is_empty()
    );
    u.update_now();
    wait_for(&u, "the failure", |s| matches!(s, State::Failed { .. }));
    let Some(Prompt::Failed { reason, page, .. }) = u.prompt() else {
        panic!("{:?}", u.state());
    };
    assert!(reason.contains("checksum mismatch"), "{reason}");
    assert_eq!(
        page,
        "https://github.com/test/velocide/releases/tag/build-43"
    );
    assert_eq!(std::fs::read(&exe).unwrap(), b"the old game");
    assert!(!install::new_path(&exe).exists());

    // ---- the file is missing (404) ---------------------------------------------------------
    let mut u = Updater::start(config("missing"));
    wait_for(&u, "the check", |s| matches!(s, State::Available(_)));
    u.update_now();
    wait_for(&u, "the failure", |s| matches!(s, State::Failed { .. }));
    let Some(Prompt::Failed { reason, .. }) = u.prompt() else {
        panic!("{:?}", u.state());
    };
    assert!(reason.contains("404"), "{reason}");
    assert_eq!(std::fs::read(&exe).unwrap(), b"the old game");
    assert!(!install::new_path(&exe).exists());

    // ---- already the latest; a repository without releases --------------------------------
    let u = Updater::start(config("current"));
    let state = wait_for(&u, "the check", |s| !matches!(s, State::Checking));
    assert_eq!(state, State::UpToDate);
    assert_eq!(u.prompt(), None);
    let u = Updater::start(config("none"));
    let state = wait_for(&u, "the check", |s| !matches!(s, State::Checking));
    assert_eq!(state, State::CheckFailed, "shown nowhere");
    assert_eq!(u.prompt(), None);

    let _ = std::fs::remove_dir_all(exe.parent().unwrap());
}
