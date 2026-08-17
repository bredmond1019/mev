//! Advisory lock guarding `emit-state --write` against concurrent writers.
//!
//! Two agents running `emit-state --write` at the same time from different sub-repos
//! can interleave writes to the same derived files (a leaf `state.json`, a
//! `docs/projects/<slug>.md` cache doc, a tier rollup, `master-plan.md`'s wave table,
//! …) because none of those writes are individually atomic. `E_EMIT_LINKED_WORKTREE`
//! does not help here — it guards against a *different* surface (a linked git
//! worktree resolving derived-file paths from `brain.toml` instead of CWD) and does
//! not fire for D46's symlinked `planning/` vaults, which is the actual concurrency
//! surface.
//!
//! [`acquire_lock`] takes an exclusive advisory lockfile at the brain root
//! (`<root>/.mev-emit.lock`) for the duration of a `--write`, recording the owning
//! pid. A second concurrent `--write` polls briefly, then fails with
//! [`LockError::Held`] rather than interleaving. A lockfile whose owning pid is no
//! longer alive is reclaimed rather than blocking forever. Dry-run callers must not
//! call this at all — the lock is `--write`-only by construction of the caller in
//! `main.rs`.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

use thiserror::Error;

/// Name of the lockfile, relative to the brain root.
const LOCK_FILE_NAME: &str = ".mev-emit.lock";

/// Default bound on how long [`acquire_lock`] will retry before giving up.
pub const DEFAULT_LOCK_TIMEOUT: Duration = Duration::from_secs(3);

/// Poll interval while waiting for a held lock to be released.
const POLL_INTERVAL: Duration = Duration::from_millis(50);

/// Errors returned by [`acquire_lock`].
#[derive(Debug, Error)]
pub enum LockError {
    /// Another live process holds the lock and did not release it within the
    /// configured timeout.
    #[error(
        "another emit-state --write (pid {holder_pid}) holds the lock at {lock_path}; \
         waited {waited_secs}s. Retry once it finishes."
    )]
    Held {
        holder_pid: u32,
        lock_path: PathBuf,
        waited_secs: u64,
    },

    /// The lockfile could not be created, read, or removed.
    #[error("could not access lockfile {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
}

/// A held advisory lock. Releases (deletes the lockfile) on drop, on both the
/// success and error paths of the caller — no explicit `release()` call is needed.
#[derive(Debug)]
pub struct LockGuard {
    path: PathBuf,
    /// Set to `false` by tests that want to observe the file surviving a guard drop
    /// (e.g. simulating a crashed holder). Always `true` in production.
    release_on_drop: bool,
}

impl LockGuard {
    /// Path to the underlying lockfile, exposed for tests/diagnostics.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for LockGuard {
    fn drop(&mut self) {
        if self.release_on_drop {
            let _ = fs::remove_file(&self.path);
        }
    }
}

/// Acquire the exclusive advisory lock at `<root>/.mev-emit.lock`, retrying for up
/// to `timeout` if another live process already holds it.
///
/// - If no lockfile exists, or the existing one names a pid that is no longer
///   alive (a stale lock, e.g. a killed process), it is created/reclaimed
///   immediately and an owning [`LockGuard`] is returned.
/// - If a live process holds it, this polls every [`POLL_INTERVAL`] until either
///   the lock is released/reclaimed or `timeout` elapses, at which point
///   [`LockError::Held`] is returned.
pub fn acquire_lock(root: &Path, timeout: Duration) -> Result<LockGuard, LockError> {
    let lock_path = root.join(LOCK_FILE_NAME);
    let started = Instant::now();

    loop {
        match try_create_lock(&lock_path) {
            Ok(()) => {
                return Ok(LockGuard {
                    path: lock_path,
                    release_on_drop: true,
                });
            }
            Err(TryCreateError::Held { holder_pid }) => {
                if !pid_is_alive(holder_pid) {
                    // Stale lock: the owning process is gone. Reclaim by removing
                    // the file and looping to retry the create.
                    let _ = fs::remove_file(&lock_path);
                    continue;
                }
                if started.elapsed() >= timeout {
                    return Err(LockError::Held {
                        holder_pid,
                        lock_path,
                        waited_secs: started.elapsed().as_secs(),
                    });
                }
                thread::sleep(POLL_INTERVAL);
            }
            Err(TryCreateError::Io(source)) => {
                return Err(LockError::Io {
                    path: lock_path,
                    source,
                });
            }
        }
    }
}

enum TryCreateError {
    /// The lockfile already exists and names a (possibly stale) pid.
    Held {
        holder_pid: u32,
    },
    Io(io::Error),
}

/// Attempt to atomically create the lockfile and write the current pid into it.
///
/// Uses `OpenOptions::create_new` for the atomicity: if the file already exists,
/// creation fails with `AlreadyExists` rather than silently truncating another
/// holder's lock.
fn try_create_lock(lock_path: &Path) -> Result<(), TryCreateError> {
    use std::io::Write as _;

    match fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(lock_path)
    {
        Ok(mut f) => {
            let pid = std::process::id();
            if let Err(e) = write!(f, "{pid}") {
                return Err(TryCreateError::Io(e));
            }
            Ok(())
        }
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
            let holder_pid = fs::read_to_string(lock_path)
                .ok()
                .and_then(|s| s.trim().parse::<u32>().ok());
            match holder_pid {
                Some(pid) => Err(TryCreateError::Held { holder_pid: pid }),
                // Lockfile exists but is empty/unparseable — treat as unknown-but-live
                // so we don't spin-reclaim a lockfile mid-write by another process;
                // report pid 0 (never a real pid) so callers can still identify this
                // case, and rely on the timeout to bound the wait.
                None => Err(TryCreateError::Held { holder_pid: 0 }),
            }
        }
        Err(e) => Err(TryCreateError::Io(e)),
    }
}

/// Best-effort liveness check for `pid` using `ps -p <pid>`.
///
/// `pid == 0` (the "unparseable lockfile" sentinel from [`try_create_lock`]) is
/// always reported alive so it is never spuriously reclaimed. Any failure to
/// invoke `ps` at all (missing from `PATH`, non-Unix, etc.) fails closed — the
/// pid is reported alive so a real held lock is never reclaimed out from under
/// its owner just because the liveness probe itself broke.
pub(crate) fn pid_is_alive(pid: u32) -> bool {
    if pid == 0 {
        return true;
    }
    match std::process::Command::new("ps")
        .arg("-p")
        .arg(pid.to_string())
        .output()
    {
        Ok(output) => output.status.success(),
        Err(_) => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root() -> PathBuf {
        let dir = crate::testsupport::unique_temp_dir("mev-emit-lock-test");
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn acquire_lock_creates_and_releases_lockfile() {
        let root = temp_root();
        let lock_path = root.join(LOCK_FILE_NAME);
        assert!(!lock_path.exists());

        {
            let guard = acquire_lock(&root, Duration::from_millis(200)).expect("should acquire");
            assert!(lock_path.exists(), "lockfile should exist while held");
            assert_eq!(guard.path(), lock_path);
            let contents = fs::read_to_string(&lock_path).unwrap();
            assert_eq!(contents.trim().parse::<u32>().unwrap(), std::process::id());
        }

        assert!(!lock_path.exists(), "lockfile should be removed on drop");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn acquire_lock_second_call_times_out_while_first_holds() {
        let root = temp_root();
        let _first = acquire_lock(&root, Duration::from_millis(200)).expect("first should acquire");

        let err = acquire_lock(&root, Duration::from_millis(150))
            .expect_err("second concurrent acquire should fail while first is held");
        match err {
            LockError::Held { holder_pid, .. } => {
                assert_eq!(holder_pid, std::process::id());
            }
            other => panic!("expected LockError::Held, got {other:?}"),
        }

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn acquire_lock_reclaims_stale_lock_from_dead_pid() {
        let root = temp_root();
        let lock_path = root.join(LOCK_FILE_NAME);
        // A pid essentially guaranteed not to be alive: a huge value beyond any
        // real pid range on a Unix system.
        fs::write(&lock_path, "999999999").unwrap();

        let guard = acquire_lock(&root, Duration::from_millis(500))
            .expect("stale lock from a dead pid should be reclaimed");
        let contents = fs::read_to_string(guard.path()).unwrap();
        assert_eq!(contents.trim().parse::<u32>().unwrap(), std::process::id());

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn released_guard_allows_a_subsequent_acquire() {
        let root = temp_root();
        {
            let _guard = acquire_lock(&root, Duration::from_millis(200)).unwrap();
        }
        // Guard dropped, lockfile removed — a fresh acquire should succeed immediately.
        let _guard2 = acquire_lock(&root, Duration::from_millis(200))
            .expect("should acquire after prior guard released");

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn pid_is_alive_true_for_current_process() {
        assert!(pid_is_alive(std::process::id()));
    }

    #[test]
    fn pid_is_alive_false_for_implausible_pid() {
        assert!(!pid_is_alive(999_999_999));
    }
}
