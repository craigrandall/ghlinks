//! Build script: embeds the git description (or commit hash) into the
//! binary as the `GHLINKS_BUILD` compile-time environment variable, so
//! `env!("GHLINKS_BUILD")` in source code surfaces the exact source
//! revision alongside `ghlinks_version` in `report.json`.
//!
//! This is more robust than relying on `Cargo.toml`'s version alone:
//! if you forget to bump the version in `Cargo.toml` after tagging, the
//! build identifier still pinpoints which commit produced the binary.
//!
//! Resolution order:
//! 1. `GHLINKS_BUILD` environment variable (explicit override â useful
//!    for reproducible builds or CI that wants to stamp a different
//!    identifier).
//! 2. `git describe --tags --always --dirty` output (the default in a
//!    normal working tree â produces something like `v0.15.0` or
//!    `v0.14.9-3-g1a2b3c4-dirty`).
//! 3. `"unknown"` if git is unavailable (e.g. a tarball source with no
//!    `.git` directory) â the build must never fail just because git
//!    isn't present.

use std::process::Command;

fn main() {
    let build = std::env::var("GHLINKS_BUILD")
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| {
            Command::new("git")
                .args(["describe", "--tags", "--always", "--dirty"])
                .output()
                .ok()
                .and_then(|o| {
                    if o.status.success() {
                        String::from_utf8(o.stdout)
                            .ok()
                            .map(|s| s.trim().to_string())
                    } else {
                        None
                    }
                })
        })
        .unwrap_or_else(|| "unknown".to_string());

    // Rerun the build script if the git HEAD or tags change, so a
    // re-build after `git pull` or `git tag` picks up the new identifier
    // without needing a clean rebuild.
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=.git/refs/tags");

    println!("cargo:rustc-env=GHLINKS_BUILD={build}");
}
