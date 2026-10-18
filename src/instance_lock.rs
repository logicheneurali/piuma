//! One bar per session: an exclusive flock on
//! `$XDG_RUNTIME_DIR/piuma.lock`, held for the whole life of the
//! process. Hyprland's own `hyprland.start` block re-runs piuma
//! whenever the compositor restarts, so the lock (not a pidfile) is
//! what keeps a single instance — the `File` *is* the lock (dropping
//! it, or dying, releases it, so a stale file can never wedge a
//! start).

use std::path::PathBuf;
use std::time::Instant;

/// Whether a `/proc/<pid>/cmdline` dump is the bar itself: every arg
/// split by NUL must contain `piuma` as its own argument — the setup
/// binary (`piuma-setup`) and unrelated owners of a recycled pid must
/// never match.
pub(crate) fn cmdline_is_piuma(cmdline: &[u8]) -> bool {
    cmdline
        .split(|b| *b == 0)
        .any(|arg| arg == b"piuma" || arg.ends_with(b"/piuma"))
}

/// The holder pid written inside the lock file (plain digits); anything
/// else means "nothing to replace".
pub(crate) fn lock_pid(bytes: &[u8]) -> Option<i32> {
    std::str::from_utf8(bytes).ok()?.trim().parse().ok()
}

/// Claim the lock for this process. A holder still alive is replaced,
/// the most recent launch winning. `None` means "run unlocked" (no
/// runtime dir, unusable lock): a degraded start, never a failed one.
pub(crate) fn acquire_instance_lock() -> Option<std::fs::File> {
    use std::fs::TryLockError;
    let Some(runtime) = std::env::var_os("XDG_RUNTIME_DIR") else {
        eprintln!("[piuma] no XDG_RUNTIME_DIR — running unlocked");
        return None;
    };
    let path = PathBuf::from(runtime).join("piuma.lock");
    let file = match std::fs::OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .open(&path)
    {
        Ok(file) => file,
        Err(e) => {
            eprintln!(
                "[piuma] cannot open {} ({e}) — running unlocked",
                path.display()
            );
            return None;
        }
    };
    match file.try_lock() {
        Ok(()) => {
            let _ = std::fs::write(&path, std::process::id().to_string());
            Some(file)
        }
        // A previous instance still holds it: replace it and wait,
        // bounded, for its lock to come free.
        Err(TryLockError::WouldBlock) => {
            if take_over_lock(&file, &path) {
                Some(file)
            } else {
                None
            }
        }
        Err(e) => {
            eprintln!("[piuma] lock unusable ({e}) — running unlocked");
            None
        }
    }
}

/// Replacement loop: identify the holder from the lock file and SIGTERM
/// it once its `/proc` cmdline is confirmed to be piuma — the pid is
/// re-read inside the loop, because the holder may still be in the
/// window between locking and writing it — then claim the lock as soon
/// as the old process has released it. piuma installs no signal
/// handler, so SIGTERM is an immediate exit by default; the two second
/// bound only guards a holder that would refuse to die (and any
/// holder, foreign or dead, that is not piuma is never signalled).
/// Returns whether the lock was claimed in time.
fn take_over_lock(file: &std::fs::File, path: &std::path::Path) -> bool {
    let mut signalled: Option<i32> = None;
    let deadline = Instant::now() + std::time::Duration::from_secs(2);
    loop {
        if file.try_lock().is_ok() {
            let _ = std::fs::write(path, std::process::id().to_string());
            eprintln!("[piuma] single instance: lock acquired");
            return true;
        }
        if let Some(pid) = std::fs::read(path)
            .ok()
            .and_then(|bytes| lock_pid(&bytes))
            .filter(|&pid| pid != std::process::id() as i32 && signalled != Some(pid))
        {
            let cmdline = std::fs::read(format!("/proc/{pid}/cmdline")).unwrap_or_default();
            if cmdline_is_piuma(&cmdline) {
                eprintln!("[piuma] single instance: replacing the previous bar (pid {pid})");
                let _ = std::process::Command::new("kill")
                    .args(["-TERM", &pid.to_string()])
                    .status();
                signalled = Some(pid);
            }
        }
        if Instant::now() >= deadline {
            eprintln!("[piuma] single instance: previous bar did not exit — running unlocked");
            return false;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The lock's holder must be recognised as the bar itself in every
    /// launch form — and never as the setup binary or an unrelated
    /// process that happens to own a recycled pid.
    #[test]
    fn lock_cmdline_only_matches_the_bar_binary() {
        assert!(cmdline_is_piuma(b"./target/release/piuma\0"));
        assert!(cmdline_is_piuma(
            b"/home/effedi/Documenti/Rust/Piuma/target/release/piuma\0"
        ));
        assert!(cmdline_is_piuma(b"piuma\0"));
        assert!(!cmdline_is_piuma(b"/usr/bin/piuma-setup\0"));
        assert!(!cmdline_is_piuma(b"python3\0"));
        assert!(!cmdline_is_piuma(b""));
    }

    /// The pid is plain digits inside the lock file; anything else
    /// (empty or corrupted file) means "no holder to replace".
    #[test]
    fn lock_pid_reads_the_holder() {
        assert_eq!(lock_pid(b"12345\n"), Some(12345));
        assert_eq!(lock_pid(b" 42 "), Some(42));
        assert_eq!(lock_pid(b""), None);
        assert_eq!(lock_pid(b"not-a-pid"), None);
    }

    /// The flock semantics the takeover relies on: two handles of the
    /// same file (two launches) conflict, and the lock is released with
    /// the closing handle — a dead holder can never wedge the start.
    #[test]
    fn lock_blocks_the_second_handle_until_the_first_drops() {
        use std::fs::TryLockError;
        let path =
            std::env::temp_dir().join(format!("piuma-lock-test-{}", std::process::id()));
        let open = || {
            std::fs::OpenOptions::new()
                .create(true)
                .read(true)
                .write(true)
                .open(&path)
                .expect("lock file must open")
        };
        let first = open();
        first.try_lock().expect("a free lock must be claimable");
        let second = open();
        assert!(matches!(second.try_lock(), Err(TryLockError::WouldBlock)));
        drop(first); // process death behaves like this drop
        second.try_lock().expect("the lock must be free again");
        drop(second);
        let _ = std::fs::remove_file(&path);
    }
}
