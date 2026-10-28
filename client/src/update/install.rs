//! Installing an update (docs/distribution.md "Update check"): the new game is downloaded next
//! to the running one as `<exe>.new`, its size and SHA-256 checked, then swapped in.
//!
//! Every function takes the paths it works on, so tests run them in a scratch folder instead
//! of on the test binary.

use std::fs::File;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, anyhow, bail};
use sha2::{Digest, Sha256};

use super::github::{self, Asset};

/// `<exe>.new`: where the download goes (the executable's folder, so the swap is a rename).
pub fn new_path(exe: &Path) -> PathBuf {
    with_suffix(exe, ".new")
}

/// `<exe>.old`: the replaced executable on Windows (a running exe can be renamed, not
/// deleted); the next launch deletes it ([`remove_old`]).
pub fn old_path(exe: &Path) -> PathBuf {
    with_suffix(exe, ".old")
}

fn with_suffix(exe: &Path, suffix: &str) -> PathBuf {
    let mut name = exe.file_name().unwrap_or_default().to_os_string();
    name.push(suffix);
    exe.with_file_name(name)
}

/// The SHA-256 of a `sha256:<hex>` digest. `Ok(None)`: another algorithm (not checked);
/// an error: a malformed SHA-256.
pub fn parse_digest(digest: &str) -> anyhow::Result<Option<[u8; 32]>> {
    let Some((algorithm, hex)) = digest.split_once(':') else {
        bail!("bad digest '{digest}'");
    };
    if !algorithm.eq_ignore_ascii_case("sha256") {
        return Ok(None);
    }
    if hex.len() != 64 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        bail!("bad SHA-256 '{hex}'");
    }
    let mut out = [0u8; 32];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16)?;
    }
    Ok(Some(out))
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The SHA-256 a release states for an asset: required (a download that can't be checked isn't
/// installed).
fn required_sha256(digest: Option<&str>) -> anyhow::Result<[u8; 32]> {
    match digest.map(parse_digest).transpose()?.flatten() {
        Some(sha) => Ok(sha),
        None => bail!("the release gives no SHA-256 checksum for the game: not installing it"),
    }
}

/// Writes `reader` to `dest` while hashing it and checks the result: exactly `size` bytes with
/// the SHA-256 of `digest` (required). `progress(bytes so far)` is called after every chunk;
/// it returns false to stop (cancelled). `dest` is made new (an old one, or a link there, is
/// removed first, never written through). On any failure `dest` is removed.
pub fn write_verified(
    mut reader: impl Read,
    dest: &Path,
    size: u64,
    digest: Option<&str>,
    mut progress: impl FnMut(u64) -> bool,
) -> anyhow::Result<()> {
    let expected = required_sha256(digest)?;
    let result = (|| {
        match std::fs::remove_file(dest) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => {
                return Err(anyhow!("can't replace {}: {e}", dest.display()));
            }
            _ => {}
        }
        let mut file = File::options()
            .write(true)
            .create_new(true)
            .open(dest)
            .with_context(|| format!("can't write {}", folder_of(dest).display()))?;
        let mut hasher = Sha256::new();
        let mut buf = vec![0u8; 64 * 1024];
        let mut done = 0u64;
        loop {
            let n = match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => n,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(anyhow!("download interrupted: {e}")),
            };
            done += n as u64;
            if done > size {
                bail!("the download is larger than the release says ({size} bytes)");
            }
            hasher.update(&buf[..n]);
            file.write_all(&buf[..n])
                .with_context(|| format!("can't write {}", dest.display()))?;
            if !progress(done) {
                bail!("cancelled");
            }
        }
        if done != size {
            bail!("the download stopped at {done} of {size} bytes");
        }
        file.sync_all()
            .with_context(|| format!("can't write {}", dest.display()))?;
        let actual: [u8; 32] = hasher.finalize().into();
        if actual != expected {
            log::warn!(
                "{}: sha256:{}, the release says sha256:{}",
                dest.display(),
                hex(&actual),
                hex(&expected)
            );
            bail!("checksum mismatch: the download isn't the release's file");
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(dest);
    }
    result
}

/// Checks the file at `path` again just before it is swapped in: `size` bytes with the SHA-256
/// of `digest` (nothing changed it since the download).
pub fn verify_file(path: &Path, size: u64, digest: Option<&str>) -> anyhow::Result<()> {
    let expected = required_sha256(digest)?;
    let mut file = File::open(path).with_context(|| format!("can't read {}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    let mut done = 0u64;
    loop {
        let n = match file.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(anyhow!("can't read {}: {e}", path.display())),
        };
        done += n as u64;
        hasher.update(&buf[..n]);
    }
    let actual: [u8; 32] = hasher.finalize().into();
    if done != size || actual != expected {
        bail!("{} changed after the download", path.display());
    }
    Ok(())
}

/// Downloads `asset` to `dest` ([`write_verified`]). Redirects (GitHub serves release files
/// from another host) are followed; `https_only`: never to plain HTTP.
pub fn download(
    asset: &Asset,
    dest: &Path,
    version: &str,
    https_only: bool,
    progress: impl FnMut(u64) -> bool,
) -> anyhow::Result<()> {
    // Half an hour for the whole file: tens of MB even on a slow line.
    let agent = github::agent(version, Duration::from_secs(30 * 60), https_only);
    let response = agent
        .get(&asset.browser_download_url)
        .header("Accept", "application/octet-stream")
        .call()
        .map_err(|e| anyhow!("download failed: {}", github::describe(&e)))?;
    let reader = response.into_body().into_reader();
    write_verified(reader, dest, asset.size, asset.digest.as_deref(), progress)
}

/// How [`swap_in`] replaces the executable.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Swap {
    /// Mode 755 (Unix), then rename over it: the running program keeps its open file.
    Replace,
    /// Windows can't replace or delete a running exe but can rename it: move it aside to
    /// `<exe>.old`, then the new file into its place (putting it back if that fails).
    MoveAside,
}

impl Swap {
    /// This platform's way.
    pub const NATIVE: Swap = if cfg!(windows) {
        Swap::MoveAside
    } else {
        Swap::Replace
    };
}

/// Puts the verified download `new` in place of `exe`, the way this platform needs.
pub fn swap_in(new: &Path, exe: &Path) -> anyhow::Result<()> {
    swap_in_with(Swap::NATIVE, new, exe)
}

/// [`swap_in`] with an explicit [`Swap`] (both run on any OS, so tests cover both).
pub fn swap_in_with(swap: Swap, new: &Path, exe: &Path) -> anyhow::Result<()> {
    let folder = folder_of(exe);
    match swap {
        Swap::Replace => {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(new, std::fs::Permissions::from_mode(0o755))
                    .with_context(|| format!("can't make {} executable", new.display()))?;
            }
            std::fs::rename(new, exe)
                .with_context(|| format!("can't replace the game in {}", folder.display()))?;
        }
        Swap::MoveAside => {
            let old = old_path(exe);
            // A leftover from an earlier update (gone unless that game still runs).
            let _ = std::fs::remove_file(&old);
            rename_retrying(exe, &old)
                .with_context(|| format!("can't move the game aside in {}", folder.display()))?;
            if let Err(e) = rename_retrying(new, exe) {
                let restored = rename_retrying(&old, exe);
                return Err(e).with_context(|| {
                    format!(
                        "can't put the new game in {}{}",
                        folder.display(),
                        if restored.is_err() {
                            format!(" (the old one is {})", old.display())
                        } else {
                            String::new()
                        }
                    )
                });
            }
        }
    }
    Ok(())
}

/// `fs::rename`, tried again for a second when access is denied: on Windows a virus scanner
/// often holds a freshly written exe for a moment.
fn rename_retrying(from: &Path, to: &Path) -> io::Result<()> {
    let mut tries = 0;
    loop {
        match std::fs::rename(from, to) {
            Err(e) if e.kind() == io::ErrorKind::PermissionDenied && tries < 5 => {
                tries += 1;
                std::thread::sleep(Duration::from_millis(200));
            }
            result => return result,
        }
    }
}

/// Deletes the `<exe>.old` an update left (at startup). On Windows the previous process may
/// still be exiting and hold the file: retried on a thread for a few seconds.
pub fn remove_old(exe: &Path) {
    let old = old_path(exe);
    match std::fs::remove_file(&old) {
        Ok(()) => log::info!("removed {} (left by an update)", old.display()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(_) => {
            let spawned = std::thread::Builder::new()
                .name("update-cleanup".into())
                .spawn(move || {
                    for _ in 0..20 {
                        std::thread::sleep(Duration::from_millis(500));
                        match std::fs::remove_file(&old) {
                            Ok(()) => return,
                            Err(e) if e.kind() == io::ErrorKind::NotFound => return,
                            Err(_) => {}
                        }
                    }
                    log::warn!("couldn't remove {}", old.display());
                });
            if let Err(e) = spawned {
                log::warn!("couldn't remove {}: {e}", old_path(exe).display());
            }
        }
    }
}

fn folder_of(path: &Path) -> &Path {
    match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p,
        _ => Path::new("."),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "velocide-update-test-{}-{name}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn sha256(data: &[u8]) -> String {
        format!("sha256:{}", hex(&Sha256::digest(data)))
    }

    #[test]
    fn paths_next_to_the_exe() {
        let exe = Path::new("/games/velocide/velocide.exe");
        assert_eq!(new_path(exe), Path::new("/games/velocide/velocide.exe.new"));
        assert_eq!(old_path(exe), Path::new("/games/velocide/velocide.exe.old"));
        assert_eq!(new_path(Path::new("velocide")), Path::new("velocide.new"));
    }

    #[test]
    fn digests() {
        // The SHA-256 of "abc".
        let abc = "sha256:ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
        assert_eq!(sha256(b"abc"), abc);
        let parsed = parse_digest(abc).unwrap().unwrap();
        assert_eq!(hex(&parsed), &abc[7..]);
        assert_eq!(
            parse_digest(&abc.to_uppercase()).unwrap().unwrap(),
            parsed,
            "either case"
        );
        assert_eq!(parse_digest("sha512:00ff").unwrap(), None);
        assert!(parse_digest("sha256:abcd").is_err());
        assert!(parse_digest(&format!("sha256:{}", "g".repeat(64))).is_err());
        assert!(parse_digest("ba7816bf").is_err());
    }

    #[test]
    fn verified_download() {
        let dir = scratch("verify");
        let dest = dir.join("velocide.new");
        let data: Vec<u8> = (0..200_000u32).map(|i| (i * 7) as u8).collect();
        let digest = sha256(&data);
        let mut seen = Vec::new();
        write_verified(&data[..], &dest, data.len() as u64, Some(&digest), |n| {
            seen.push(n);
            true
        })
        .unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), data);
        assert_eq!(seen.last(), Some(&(data.len() as u64)));
        assert!(seen.windows(2).all(|w| w[0] < w[1]), "progress only grows");
        // No SHA-256: refused.
        let err = write_verified(&data[..], &dest, data.len() as u64, None, |_| true).unwrap_err();
        assert!(format!("{err:#}").contains("no SHA-256"), "{err:#}");
        let other = "sha512:00ff";
        assert!(
            write_verified(&data[..], &dest, data.len() as u64, Some(other), |_| true).is_err()
        );
        // An existing file (or a link) there is replaced, not written through.
        std::fs::write(&dest, b"stale").unwrap();
        write_verified(&data[..], &dest, data.len() as u64, Some(&digest), |_| true).unwrap();
        verify_file(&dest, data.len() as u64, Some(&digest)).unwrap();
        std::fs::write(&dest, b"swapped meanwhile").unwrap();
        assert!(verify_file(&dest, data.len() as u64, Some(&digest)).is_err());
    }

    #[test]
    fn bad_downloads_leave_nothing() {
        let dir = scratch("bad");
        let dest = dir.join("velocide.new");
        let data = b"the new game".to_vec();
        let n = data.len() as u64;
        let wrong = sha256(b"another game");
        let err = write_verified(&data[..], &dest, n, Some(&wrong), |_| true).unwrap_err();
        assert!(format!("{err:#}").contains("checksum mismatch"), "{err:#}");
        assert!(!dest.exists());
        let right = sha256(&data);
        let err = write_verified(&data[..], &dest, n + 5, Some(&right), |_| true).unwrap_err();
        assert!(format!("{err:#}").contains("stopped at"), "{err:#}");
        assert!(!dest.exists());
        let err = write_verified(&data[..], &dest, n - 1, Some(&right), |_| true).unwrap_err();
        assert!(format!("{err:#}").contains("larger"), "{err:#}");
        assert!(!dest.exists());
        let err = write_verified(&data[..], &dest, n, Some(&right), |_| false).unwrap_err();
        assert!(format!("{err:#}").contains("cancelled"), "{err:#}");
        assert!(!dest.exists());
        // A folder that doesn't exist (as good as read-only).
        let nowhere = dir.join("missing").join("velocide.new");
        let err = write_verified(&data[..], &nowhere, n, Some(&right), |_| true).unwrap_err();
        assert!(format!("{err:#}").contains("can't write"), "{err:#}");
    }

    #[test]
    fn swap_replaces_the_exe() {
        for swap in [Swap::Replace, Swap::MoveAside] {
            let dir = scratch(&format!("swap-{swap:?}"));
            let exe = dir.join("velocide");
            std::fs::write(&exe, b"old game").unwrap();
            std::fs::write(new_path(&exe), b"new game").unwrap();
            swap_in_with(swap, &new_path(&exe), &exe).unwrap();
            assert_eq!(std::fs::read(&exe).unwrap(), b"new game", "{swap:?}");
            assert!(!new_path(&exe).exists());
            #[cfg(unix)]
            if swap == Swap::Replace {
                use std::os::unix::fs::PermissionsExt;
                let mode = std::fs::metadata(&exe).unwrap().permissions().mode();
                assert_eq!(mode & 0o777, 0o755);
            }
            // Windows' way leaves the old game aside; the next launch removes it.
            assert_eq!(old_path(&exe).exists(), swap == Swap::MoveAside);
            remove_old(&exe);
            assert!(!old_path(&exe).exists());
        }
    }

    #[test]
    fn move_aside_replaces_a_leftover_and_rolls_back() {
        let dir = scratch("aside");
        let exe = dir.join("velocide.exe");
        std::fs::write(&exe, b"old game").unwrap();
        std::fs::write(old_path(&exe), b"older game").unwrap();
        std::fs::write(new_path(&exe), b"new game").unwrap();
        swap_in_with(Swap::MoveAside, &new_path(&exe), &exe).unwrap();
        assert_eq!(std::fs::read(&exe).unwrap(), b"new game");
        assert_eq!(std::fs::read(old_path(&exe)).unwrap(), b"old game");

        // The new file can't be moved in (here: it is missing): the old game goes back.
        std::fs::remove_file(old_path(&exe)).unwrap();
        let err = swap_in_with(Swap::MoveAside, &new_path(&exe), &exe);
        assert!(err.is_err());
        assert_eq!(std::fs::read(&exe).unwrap(), b"new game");
        assert!(!old_path(&exe).exists());
    }
}
