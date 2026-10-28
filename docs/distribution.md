# Distribution: GitHub releases, standalone binaries, updates

Friends get the game from the GitHub releases of <https://github.com/SamSkinner01/velocide>
(public). Each release is a handful of single files: the game runs from one executable, with
no `assets/` folder to keep next to it, and tells the player when a newer release is out.

This file is the contract between the CI workflow, the packaging scripts, the embedded assets
and the in-game updater. Change it first if any of them needs something else.

## Releases (CI makes them, the updater reads them)

The workflow `.github/workflows/release.yml` (GitHub Actions, GitHub-hosted runners) replaces
the Forgejo one:

- every pull request and push: fmt, clippy (`-D warnings`) and the whole test suite;
- every push to `main` that passes: a release tagged `build-<run number>`;
- every `v*` tag that passes: a release under that tag;
- each new release is marked *latest*; the newest 10 `build-*` releases are kept, older ones
  (and their tags) deleted.

Every release has exactly these assets. The names carry no version, so
`https://github.com/<repo>/releases/latest/download/<name>` is a permanent link to the newest:

| asset                                 | what                                         |
| ------------------------------------- | -------------------------------------------- |
| `velocide-windows-x86_64.exe`         | the game, standalone                         |
| `velocide-linux-x86_64`               | the game, standalone (glibc 2.31 or newer)   |
| `velocide-server-windows-x86_64.exe`  | the dedicated server, standalone             |
| `velocide-server-linux-x86_64`        | the dedicated server, standalone             |
| `PLAY.txt`                            | how to play, host and join (version filled in) |

A release's `tag_name` is its version (`build-42`, `v1.0.0`) and its `target_commitish` is the
full 40-hex commit it was built from.

## Build identity

Three compile-time environment variables, read with `option_env!` (rustc tracks them, so a
change rebuilds) and exposed as `shared::build_info`:

| variable           | value                                  | unset or empty           |
| ------------------ | -------------------------------------- | ------------------------ |
| `VELOCIDE_VERSION` | the release's version, e.g. `build-42` | `dev`                    |
| `VELOCIDE_COMMIT`  | the full 40-hex commit                 | empty                    |
| `VELOCIDE_REPO`    | `owner/name` the updater checks        | `SamSkinner01/velocide`  |

`shared::build_info::{VERSION, COMMIT, REPO}` are `&'static str`; `is_release()` is true when
`COMMIT` is 40 hex digits and `VERSION` isn't `dev`. `label()` is what players compare (the
main menu's corner, Settings → Gameplay, both binaries' startup log line): `build-42
(1a2b3c4)`, or just `dev`. `scripts/build_dist.sh` sets all three
(VERSION as today, the commit from `git rev-parse HEAD`, the repo from `GITHUB_REPOSITORY`
when set), so CI builds and `scripts/package.sh` builds are release builds; a plain
`cargo build` / `cargo run` is a dev build.

## Standalone binaries (embedded assets)

`velocide` and `velocide-server` run from an otherwise empty folder: every file under
`assets/` that a binary reads at run time is compiled into it.

An `assets/` folder on disk still wins: when one is found (the existing `engine::assets_dir()`
rules: `VELOCIDE_ASSETS`, `./assets`, an `assets/` next to or above the executable), a file
there is used instead of the embedded copy, and a file missing there falls back to the
embedded one. Lists (levels, enemy definitions) are the union of both, the disk copy winning
for the same name. So development is unchanged: `cargo run` from the repo reads the working
tree's files, hot reload of `tuning.ron` and shaders, the tuning editor, `level_gen` and
`models_gen` all keep working on the files in `assets/`.

Settings stay where they are (the user's config directory, `client/src/settings.rs`).

## Update check (client)

- Only in release builds (`build_info::is_release()`), once per launch, on a background
  thread; never in dev builds, tests or `--bot` / `--quit-after` / `--capture` runs. Off with
  `VELOCIDE_NO_UPDATE_CHECK=1` or the settings toggle "Check for updates" (default on).
- `GET {api}/repos/{REPO}/releases/latest`, where `{api}` is `https://api.github.com` unless
  `VELOCIDE_UPDATE_API` (read at run time, for tests against a local server) says otherwise.
  Headers `User-Agent: velocide/<VERSION>` and `Accept: application/vnd.github+json`. The
  latest commit is `target_commitish` when that is 40 hex, else the tag's commit from
  `GET {api}/repos/{REPO}/commits/{tag_name}` (`.sha`). An update is available when it differs
  from `build_info::COMMIT`. Any failure (offline, rate limit, bad JSON) is logged and shown
  nowhere.
- The prompt appears on the main menu (if the answer comes in during a match, when the player
  is next on the main menu): the new version, the running one, and that everyone in a game
  needs the same version. Choices: **Update now**, **Later** (until the next launch; during
  the download it is **Cancel** and stops it).
- **Update now** downloads this platform's game asset (table above) into the executable's
  folder as `<exe>.new` with progress shown, checks its size, and its SHA-256 against the
  asset's `digest` (`sha256:<hex>`) when the API gives one, then swaps it in (Windows: rename
  the running exe to `<exe>.old`, then `<exe>.new` to the exe; Linux: mode 755, then rename
  over the exe), starts the new executable with the same arguments and exits. A leftover
  `<exe>.old` is deleted at startup. If anything fails (read-only folder, download error,
  checksum mismatch, a platform without an asset) the prompt says so and offers to open the
  release page (`html_url`) in the browser instead.
