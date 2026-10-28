//! The GitHub releases API as the updater uses it (docs/distribution.md "Update check"): the
//! latest release, the commit it was built from, this platform's game asset.

use std::time::Duration;

use anyhow::{Context, anyhow, bail};
use serde::Deserialize;
use shared::build_info;

/// GitHub's API, unless `VELOCIDE_UPDATE_API` says otherwise.
pub const DEFAULT_API: &str = "https://api.github.com";

/// A release as `GET /repos/{repo}/releases/latest` describes it (only what the updater reads;
/// other fields are ignored).
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
pub struct Release {
    /// The version: `build-42`, `v1.0.0`.
    pub tag_name: String,
    /// The full commit the release was built from (CI's releases), or a branch name.
    #[serde(default)]
    pub target_commitish: String,
    /// The release's page (where players can download it by hand).
    #[serde(default)]
    pub html_url: String,
    #[serde(default)]
    pub assets: Vec<Asset>,
}

/// One file of a release.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
pub struct Asset {
    pub name: String,
    /// Bytes.
    pub size: u64,
    pub browser_download_url: String,
    /// `sha256:<hex>` (GitHub computes it on upload; older releases have none).
    #[serde(default)]
    pub digest: Option<String>,
}

/// The newest release and the commit it was built from (resolved: always 40 hex).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Latest {
    pub release: Release,
    pub commit: String,
}

impl Latest {
    /// How the prompt names it: `build-43 (1a2b3c4)`.
    pub fn label(&self) -> String {
        build_info::version_label(&self.release.tag_name, &self.commit)
    }
}

/// The game asset's name for an OS and architecture (`std::env::consts` values), as in the
/// table of docs/distribution.md. `None`: no build for that platform.
pub fn game_asset_name(os: &str, arch: &str) -> Option<&'static str> {
    match (os, arch) {
        ("windows", "x86_64") => Some("velocide-windows-x86_64.exe"),
        ("linux", "x86_64") => Some("velocide-linux-x86_64"),
        _ => None,
    }
}

impl Release {
    /// The game asset for an OS and architecture, if this release has one.
    pub fn game_asset_for(&self, os: &str, arch: &str) -> Option<&Asset> {
        let name = game_asset_name(os, arch)?;
        self.assets.iter().find(|a| a.name == name)
    }

    /// The game asset for the platform this binary runs on.
    pub fn game_asset(&self) -> Option<&Asset> {
        self.game_asset_for(std::env::consts::OS, std::env::consts::ARCH)
    }
}

pub fn parse_release(json: &str) -> anyhow::Result<Release> {
    let release: Release = serde_json::from_str(json).context("bad release JSON")?;
    if release.tag_name.is_empty() {
        bail!("release without a tag");
    }
    Ok(release)
}

/// The `.sha` of a `GET /repos/{repo}/commits/{ref}` answer.
pub fn parse_commit_sha(json: &str) -> anyhow::Result<String> {
    #[derive(Deserialize)]
    struct Commit {
        sha: String,
    }
    let commit: Commit = serde_json::from_str(json).context("bad commit JSON")?;
    if !build_info::is_full_commit(&commit.sha) {
        bail!("'{}' is not a commit hash", commit.sha);
    }
    Ok(commit.sha.to_ascii_lowercase())
}

/// The release's commit: `target_commitish` when that is a full hash (CI sets it), else the
/// tag's commit from `tag_commit` (a release made from a branch names the branch there).
pub fn resolve_commit(
    release: &Release,
    tag_commit: impl FnOnce(&str) -> anyhow::Result<String>,
) -> anyhow::Result<String> {
    if build_info::is_full_commit(&release.target_commitish) {
        return Ok(release.target_commitish.to_ascii_lowercase());
    }
    tag_commit(&release.tag_name)
}

/// Whether the latest release is another build than the running one. Any difference counts:
/// releases replace each other, and everyone in a game needs the same one.
pub fn is_update(latest_commit: &str, running_commit: &str) -> bool {
    !latest_commit.eq_ignore_ascii_case(running_commit)
}

/// A path segment with everything but unreserved characters percent-encoded (tags are
/// user-chosen; `build-42` and `v1.0.0` pass unchanged).
fn encode_segment(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// A short reason for a failed request (shown in the prompt when a download fails).
pub fn describe(e: &ureq::Error) -> String {
    match e {
        ureq::Error::StatusCode(403 | 429) => "rate limited by GitHub, try later".into(),
        ureq::Error::StatusCode(code) => format!("the server answered HTTP {code}"),
        ureq::Error::HostNotFound => "host not found (offline?)".into(),
        ureq::Error::Timeout(_) => "timed out".into(),
        ureq::Error::Io(e) => e.to_string(),
        other => other.to_string(),
    }
}

/// HTTP agent for the API and downloads: `User-Agent: velocide/<version>`, connect and
/// response timeouts. `body_timeout` bounds reading a whole body (the game download is tens of
/// MB on any connection). `https_only`: no plain HTTP, not even through a redirect (always,
/// against GitHub; tests run a local HTTP server).
pub fn agent(version: &str, body_timeout: Duration, https_only: bool) -> ureq::Agent {
    ureq::Agent::config_builder()
        .https_only(https_only)
        .user_agent(format!("velocide/{version}"))
        .timeout_connect(Some(Duration::from_secs(15)))
        .timeout_recv_response(Some(Duration::from_secs(30)))
        .timeout_recv_body(Some(body_timeout))
        .build()
        .new_agent()
}

/// The releases API of one repository.
pub struct Api {
    agent: ureq::Agent,
    /// `https://api.github.com` (no trailing slash).
    base: String,
    /// `owner/name`.
    repo: String,
}

impl Api {
    pub fn new(base: &str, repo: &str, version: &str) -> Self {
        Self {
            agent: agent(
                version,
                Duration::from_secs(60),
                base.starts_with("https://"),
            ),
            base: base.trim_end_matches('/').to_string(),
            repo: repo.to_string(),
        }
    }

    fn get(&self, path: &str) -> anyhow::Result<String> {
        let url = format!("{}/repos/{}/{path}", self.base, self.repo);
        let mut response = self
            .agent
            .get(&url)
            .header("Accept", "application/vnd.github+json")
            .call()
            .map_err(|e| anyhow!("GET {url}: {}", describe(&e)))?;
        response
            .body_mut()
            .read_to_string()
            .with_context(|| format!("GET {url}"))
    }

    /// `GET /repos/{repo}/releases/latest`.
    pub fn latest_release(&self) -> anyhow::Result<Release> {
        parse_release(&self.get("releases/latest")?)
    }

    /// `GET /repos/{repo}/commits/{tag}` → `.sha`.
    pub fn tag_commit(&self, tag: &str) -> anyhow::Result<String> {
        parse_commit_sha(&self.get(&format!("commits/{}", encode_segment(tag)))?)
    }

    /// The latest release with its commit resolved.
    pub fn latest(&self) -> anyhow::Result<Latest> {
        let release = self.latest_release()?;
        let commit = resolve_commit(&release, |tag| self.tag_commit(tag))?;
        Ok(Latest { release, commit })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHA: &str = "63837fb778d6a39f02be0ce76ff89f2bc8d74bdc";
    const OTHER: &str = "0123456789abcdef0123456789abcdef01234567";

    /// Trimmed from a real `releases/latest` answer.
    const RELEASE_JSON: &str = r#"{
        "url": "https://api.github.com/repos/SamSkinner01/velocide/releases/1",
        "html_url": "https://github.com/SamSkinner01/velocide/releases/tag/build-43",
        "id": 1,
        "tag_name": "build-43",
        "target_commitish": "63837fb778d6a39f02be0ce76ff89f2bc8d74bdc",
        "name": "build-43",
        "draft": false,
        "prerelease": false,
        "assets": [
            {
                "name": "velocide-linux-x86_64",
                "content_type": "application/octet-stream",
                "state": "uploaded",
                "size": 61234567,
                "digest": "sha256:f756e7859d90dbb5e85aae5194206c37da220bec9bac23d3d659078a8e766fdc",
                "download_count": 3,
                "browser_download_url": "https://github.com/SamSkinner01/velocide/releases/download/build-43/velocide-linux-x86_64"
            },
            {
                "name": "velocide-windows-x86_64.exe",
                "size": 62000000,
                "digest": null,
                "browser_download_url": "https://github.com/SamSkinner01/velocide/releases/download/build-43/velocide-windows-x86_64.exe"
            },
            {
                "name": "velocide-server-linux-x86_64",
                "size": 20000000,
                "browser_download_url": "https://github.com/SamSkinner01/velocide/releases/download/build-43/velocide-server-linux-x86_64"
            }
        ],
        "body": "Automatic build."
    }"#;

    #[test]
    fn parses_a_release() {
        let r = parse_release(RELEASE_JSON).unwrap();
        assert_eq!(r.tag_name, "build-43");
        assert_eq!(r.target_commitish, SHA);
        assert!(r.html_url.ends_with("/releases/tag/build-43"));
        assert_eq!(r.assets.len(), 3);
        let linux = &r.assets[0];
        assert_eq!(linux.size, 61234567);
        assert!(linux.digest.as_deref().unwrap().starts_with("sha256:f756"));
        // `null` and a missing digest are both "none".
        assert_eq!(r.assets[1].digest, None);
        assert_eq!(r.assets[2].digest, None);
    }

    #[test]
    fn rejects_bad_json() {
        assert!(parse_release("").is_err());
        assert!(parse_release("{\"message\": \"Not Found\"}").is_err());
        assert!(parse_release("{\"tag_name\": \"\"}").is_err());
        assert!(parse_release("[1, 2]").is_err());
        // Assets missing their size or URL are a broken answer, not a release.
        assert!(parse_release(r#"{"tag_name": "b", "assets": [{"name": "x"}]}"#).is_err());
        // Only the tag is required.
        let r = parse_release(r#"{"tag_name": "v1.0.0"}"#).unwrap();
        assert_eq!(r.target_commitish, "");
        assert!(r.assets.is_empty());
    }

    #[test]
    fn commit_comparison() {
        assert!(!is_update(SHA, SHA));
        assert!(!is_update(&SHA.to_uppercase(), SHA));
        assert!(is_update(OTHER, SHA));
        // A build without a commit (shouldn't check at all) sees every release as new.
        assert!(is_update(SHA, ""));
    }

    #[test]
    fn commit_from_target_commitish_or_the_tag() {
        let mut r = parse_release(RELEASE_JSON).unwrap();
        let no_lookup = |_: &str| -> anyhow::Result<String> { panic!("needs no lookup") };
        assert_eq!(resolve_commit(&r, no_lookup).unwrap(), SHA);
        r.target_commitish = SHA.to_uppercase();
        assert_eq!(resolve_commit(&r, no_lookup).unwrap(), SHA);

        // Made from a branch: the tag's commit.
        r.target_commitish = "main".into();
        let looked_up = resolve_commit(&r, |tag| {
            assert_eq!(tag, "build-43");
            parse_commit_sha(&format!(
                r#"{{"sha": "{OTHER}", "commit": {{"message": "x"}}}}"#
            ))
        });
        assert_eq!(looked_up.unwrap(), OTHER);
        let failed = resolve_commit(&r, |_| Err(anyhow!("offline")));
        assert!(failed.is_err());
    }

    #[test]
    fn commit_answers() {
        assert_eq!(
            parse_commit_sha(&format!(r#"{{"sha": "{}"}}"#, SHA.to_uppercase())).unwrap(),
            SHA
        );
        assert!(parse_commit_sha(r#"{"sha": "main"}"#).is_err());
        assert!(parse_commit_sha(r#"{"message": "No commit found"}"#).is_err());
    }

    #[test]
    fn asset_per_platform() {
        let r = parse_release(RELEASE_JSON).unwrap();
        let name = |os, arch| r.game_asset_for(os, arch).map(|a| a.name.as_str());
        assert_eq!(name("linux", "x86_64"), Some("velocide-linux-x86_64"));
        assert_eq!(
            name("windows", "x86_64"),
            Some("velocide-windows-x86_64.exe")
        );
        // Never the server, never another platform.
        assert_eq!(name("macos", "aarch64"), None);
        assert_eq!(name("linux", "aarch64"), None);
        // A release missing this platform's file.
        let mut partial = r.clone();
        partial.assets.retain(|a| !a.name.contains("windows"));
        assert!(partial.game_asset_for("windows", "x86_64").is_none());
        assert!(partial.game_asset_for("linux", "x86_64").is_some());
    }

    #[test]
    fn tags_in_paths() {
        assert_eq!(encode_segment("build-42"), "build-42");
        assert_eq!(encode_segment("v1.0.0"), "v1.0.0");
        assert_eq!(encode_segment("a/b c"), "a%2Fb%20c");
    }

    #[test]
    fn labels() {
        let latest = Latest {
            release: parse_release(RELEASE_JSON).unwrap(),
            commit: SHA.into(),
        };
        assert_eq!(latest.label(), "build-43 (63837fb)");
    }
}
