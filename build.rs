//! Interchouette fork: tags the version shown by `trunk --version` with the fork
//! build revision, so a downloaded binary identifies exactly what it was built from.
//!
//! `TRUNK_BUILD_REVISION` (set by the release workflow to the commit SHA) yields
//! `<version>+itc.<revision>`; without it the version is `<version>+itc`.

fn main() {
    println!("cargo:rerun-if-env-changed=TRUNK_BUILD_REVISION");

    let version = std::env::var("CARGO_PKG_VERSION").expect("CARGO_PKG_VERSION is set by cargo");
    let revision = std::env::var("TRUNK_BUILD_REVISION").unwrap_or_default();
    let revision = revision.trim();

    let build_version = if revision.is_empty() {
        format!("{version}+itc")
    } else {
        assert!(
            revision
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-'),
            "TRUNK_BUILD_REVISION must be a SemVer build identifier ([0-9A-Za-z-]), got {revision:?}"
        );
        format!("{version}+itc.{revision}")
    };

    println!("cargo:rustc-env=TRUNK_BUILD_VERSION={build_version}");
}
