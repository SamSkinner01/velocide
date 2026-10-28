//! Which build this is (docs/distribution.md "Build identity"): the release version, the
//! commit it was built from and the GitHub repository the client's updater checks.
//!
//! Set at compile time by `scripts/build_dist.sh` through `VELOCIDE_VERSION`,
//! `VELOCIDE_COMMIT` and `VELOCIDE_REPO` (`option_env!`, so changing one rebuilds). A plain
//! `cargo build` / `cargo run` is a dev build: version `dev`, no commit.

/// The release's version (its tag: `build-42`, `v1.0.0`), or `dev`.
pub const VERSION: &str = match option_env!("VELOCIDE_VERSION") {
    Some(v) if !v.is_empty() => v,
    _ => "dev",
};

/// The full 40-hex commit the binary was built from (empty in dev builds).
pub const COMMIT: &str = match option_env!("VELOCIDE_COMMIT") {
    Some(c) => c,
    None => "",
};

/// `owner/name` of the GitHub repository whose releases the updater checks.
pub const REPO: &str = match option_env!("VELOCIDE_REPO") {
    Some(r) if !r.is_empty() => r,
    _ => "SamSkinner01/velocide",
};

/// Whether `s` is a full commit hash (40 hex digits, either case).
pub fn is_full_commit(s: &str) -> bool {
    s.len() == 40 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// A release build: built with a full commit and a version other than `dev` (CI and
/// `scripts/package.sh` builds). Only release builds look for updates.
pub fn is_release() -> bool {
    is_full_commit(COMMIT) && VERSION != "dev"
}

/// The first 7 digits of a commit (all of it if shorter), as GitHub shows them.
pub fn short_commit(commit: &str) -> &str {
    commit.get(..7).unwrap_or(commit)
}

/// What players compare: `build-42 (1a2b3c4)` for a release build, `dev` otherwise.
pub fn label() -> String {
    version_label(VERSION, COMMIT)
}

/// [`label`] for any version and commit (the updater names other builds this way).
pub fn version_label(version: &str, commit: &str) -> String {
    if is_full_commit(commit) && version != "dev" {
        format!("{version} ({})", short_commit(commit).to_ascii_lowercase())
    } else {
        version.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHA: &str = "63837fb778d6a39f02be0ce76ff89f2bc8d74bdc";

    #[test]
    fn commits() {
        assert!(is_full_commit(SHA));
        assert!(is_full_commit(&SHA.to_uppercase()));
        assert!(!is_full_commit(&SHA[..39]));
        assert!(!is_full_commit("main"));
        assert!(!is_full_commit(&format!("{}g", &SHA[..39])));
        assert_eq!(short_commit(SHA), "63837fb");
        assert_eq!(short_commit("abc"), "abc");
    }

    #[test]
    fn labels() {
        assert_eq!(version_label("build-42", SHA), "build-42 (63837fb)");
        assert_eq!(version_label("dev", SHA), "dev");
        assert_eq!(version_label("dev", ""), "dev");
        assert_eq!(version_label("build-42", "main"), "build-42");
    }

    #[test]
    fn this_build() {
        // `cargo test` is a dev build unless the variables are set.
        assert_eq!(is_release(), is_full_commit(COMMIT) && VERSION != "dev");
        assert!(!VERSION.is_empty());
        assert!(REPO.contains('/'));
        if !is_release() {
            assert_eq!(label(), VERSION);
        }
    }
}
