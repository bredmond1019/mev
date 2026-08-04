//! Build-time provenance stamp for `mev conformance`'s `toolchain-freshness` check.
//!
//! Stamps three `cargo:rustc-env` values into the binary so a running `mev` can be
//! compared against the source tree it was built from:
//!
//! - `MEV_BUILD_GIT_SHA` — `git rev-parse HEAD` run in `CARGO_MANIFEST_DIR` at build time.
//! - `MEV_BUILD_DIRTY` — `"1"` if `git status --porcelain` was non-empty at build time,
//!   `"0"` otherwise.
//! - `MEV_BUILD_SOURCE_DIR` — `CARGO_MANIFEST_DIR`, so the check knows where to re-run
//!   `git rev-parse HEAD` live.
//!
//! If git is unavailable, or any command fails, every value falls back to the literal
//! `"unknown"` rather than failing the build — a dev machine without git on `PATH`, or a
//! source tree copied without `.git/`, must still be able to build `mev`.
//!
//! Reruns are triggered only by commits (`.git/HEAD`, `.git/index` changing), not by every
//! `cargo` invocation, so the stamp refreshes on commit without forcing a rebuild each time.

use std::path::Path;
use std::process::Command;

fn run_git(manifest_dir: &str, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(manifest_dir)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout)
        .ok()
        .map(|s| s.trim().to_string())
}

fn main() {
    let manifest_dir =
        std::env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| "unknown".to_string());

    let git_sha = if Path::new(&manifest_dir).exists() {
        run_git(&manifest_dir, &["rev-parse", "HEAD"]).unwrap_or_else(|| "unknown".to_string())
    } else {
        "unknown".to_string()
    };

    let dirty = if Path::new(&manifest_dir).exists() {
        match run_git(&manifest_dir, &["status", "--porcelain"]) {
            Some(status) => {
                if status.is_empty() {
                    "0".to_string()
                } else {
                    "1".to_string()
                }
            }
            None => "unknown".to_string(),
        }
    } else {
        "unknown".to_string()
    };

    println!("cargo:rustc-env=MEV_BUILD_GIT_SHA={git_sha}");
    println!("cargo:rustc-env=MEV_BUILD_DIRTY={dirty}");
    println!("cargo:rustc-env=MEV_BUILD_SOURCE_DIR={manifest_dir}");

    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=.git/index");
}
