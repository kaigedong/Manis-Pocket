//! Resolves the version the About window shows.
//!
//! `CARGO_PKG_VERSION` alone cannot tell two builds apart, and the number the
//! package metadata reports is derived from git: `packaging/package-version.sh`
//! prints `0.1.0.r1136.gf85a27d` for a build that is not attached to a release.
//! Reusing that shape keeps "关于" in agreement with `pacman -Qi manis-pocket`
//! instead of inventing a third number. A release build can state its version
//! outright with `MANIS_POCKET_VERSION`, which is the same value the packaging
//! already resolved.
//!
//! The About window is the only consumer, so this stays a build script rather
//! than a runtime lookup: the binary should not have to find a git checkout to
//! say what it is.

use std::process::Command;

fn main() {
    println!("cargo:rerun-if-env-changed=MANIS_POCKET_VERSION");

    let version = std::env::var("MANIS_POCKET_VERSION")
        .ok()
        .map(|version| version.trim().to_string())
        .filter(|version| !version.is_empty())
        .unwrap_or_else(git_version);
    println!("cargo:rustc-env=MANIS_POCKET_BUILD_VERSION={version}");
}

/// `package-version.sh`'s fallback, so a local build reports the same string the
/// package manager would: `0.1.0.r<commits>.g<revision>`.
///
/// The git directory is watched so a new commit changes the number instead of
/// leaving a stale one baked into the next binary.
fn git_version() -> String {
    let package = std::env::var("CARGO_PKG_VERSION").unwrap_or_else(|_| "0.0.0".to_string());
    let manifest = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".to_string());
    let git = |arguments: &[&str]| -> Option<String> {
        let output = Command::new("git")
            .arg("-C")
            .arg(&manifest)
            .args(arguments)
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        String::from_utf8(output.stdout)
            .ok()
            .map(|text| text.trim().to_string())
            .filter(|text| !text.is_empty())
    };

    if let Some(directory) = git(&["rev-parse", "--absolute-git-dir"]) {
        println!("cargo:rerun-if-changed={directory}/HEAD");
        println!("cargo:rerun-if-changed={directory}/refs");
    }

    match (
        git(&["rev-list", "--count", "HEAD"]),
        git(&["rev-parse", "--short=7", "HEAD"]),
    ) {
        (Some(commits), Some(revision)) => format!("{package}.r{commits}.g{revision}"),
        // A source tarball has no git; the crate version is the honest answer.
        _ => package,
    }
}
