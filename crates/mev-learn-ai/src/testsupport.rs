//! Test-support surface — not public API beyond what `pub` requires for `tests/` to link.
//!
//! Mirrors `mev::testsupport::unique_temp_dir`; `mev-learn-ai` cannot depend on `mev` (see the
//! crate doc comment in `lib.rs`), so this is mirrored here rather than imported.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

/// Build a collision-proof, process-unique temp directory path.
///
/// Fixed-name temp dirs (`temp_dir().join("mev-some-test")`) are safe across
/// *different* tests in one process but race when the **same** test runs in
/// two concurrent processes (two terminals, two concurrent agent sessions) —
/// one process wipes the fixture directory out from under the other mid-test.
///
/// `pid` alone does not close this: threads of one process share a `pid`. Nor
/// does `pid` + nanosecond timestamp alone: macOS `SystemTime::now()` is only
/// microsecond-resolution, so two calls in the same microsecond can collide.
/// The trailing atomic counter is what guarantees uniqueness *within* a
/// process; the differing `pid` is what closes the hazard *across* processes.
#[doc(hidden)]
pub fn unique_temp_dir(prefix: &str) -> PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let seq = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    std::env::temp_dir().join(format!("{prefix}-{}-{nanos}-{seq}", std::process::id()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unique_temp_dir_never_repeats_for_the_same_prefix() {
        let a = unique_temp_dir("mev-learn-ai-testsupport-probe");
        let b = unique_temp_dir("mev-learn-ai-testsupport-probe");
        assert_ne!(
            a, b,
            "unique_temp_dir must never hand out the same path twice for the same prefix"
        );
    }

    #[test]
    fn unique_temp_dir_is_under_the_system_temp_dir_and_carries_the_prefix() {
        let path = unique_temp_dir("mev-learn-ai-prefix-probe");
        assert!(
            path.starts_with(std::env::temp_dir()),
            "fixtures must live under the system temp dir; got {}",
            path.display()
        );
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        assert!(
            name.starts_with("mev-learn-ai-prefix-probe-"),
            "the caller's prefix must lead the directory name; got {name}"
        );
    }
}
