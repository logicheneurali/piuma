//! Reaping fire-and-forget children.
//!
//! The bar runs for the whole session: every bare `Command::spawn()`
//! leaves a zombie once the child exits. `reaping` moves the wait to a
//! throwaway thread, so the caller stays non-blocking and the child is
//! collected.

use std::process::Command;

/// Run `command` on a detached thread that waits for it (reaping it).
pub(crate) fn reaping(mut command: Command) {
    std::thread::spawn(move || {
        if let Err(err) = command.status() {
            eprintln!("[piuma] process: {err}");
        }
    });
}
