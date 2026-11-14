//! Reaping fire-and-forget children.
//!
//! The bar runs for the whole session: every bare `Command::spawn()`
//! leaves a zombie once the child exits. `reaping` moves the wait to a
//! throwaway thread, so the caller stays non-blocking and the child is
//! collected.

use std::process::Command;
use std::time::{Duration, Instant};

/// How long a started action is watched for an early failure: a typo or
/// a missing binary exits at once, while a GUI app is still running and
/// must not be waited for (it would report minutes later, or never).
const GRACE: Duration = Duration::from_millis(1200);

/// Run `command` on a detached thread that waits for it (reaping it).
pub(crate) fn reaping(mut command: Command) {
    std::thread::spawn(move || {
        if let Err(err) = command.status() {
            eprintln!("[piuma] process: {err}");
        }
    });
}

/// Run `command` and watch it for [`GRACE`]: a command that fails at
/// startup comes back as a one-line reason for the OSD, while one still
/// running after the grace (the GUI app it launched) is left alone.
pub(crate) fn run_reporting(mut command: Command, label: String) -> Option<String> {
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(err) => return Some(format!("{label}: {err}")),
    };
    let deadline = Instant::now() + GRACE;
    loop {
        match child.try_wait() {
            Ok(Some(status)) if !status.success() => {
                let code = status
                    .code()
                    .map(|code| code.to_string())
                    .unwrap_or_else(|| "signal".to_string());
                return Some(format!("{label}: exit {code}"));
            }
            Ok(Some(_)) => return None,
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(50));
            }
            // Still running (or unreadable): not a startup failure, but
            // the child will exit later and needs a waiter.
            Ok(None) | Err(_) => {
                std::thread::spawn(move || {
                    let _ = child.wait();
                });
                return None;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_failed_action_reports_its_exit_code() {
        let mut command = Command::new("sh");
        command.args(["-c", "exit 3"]);
        assert_eq!(
            run_reporting(command, "CPU".to_string()),
            Some("CPU: exit 3".to_string())
        );
    }

    #[test]
    fn a_successful_action_reports_nothing() {
        let mut command = Command::new("sh");
        command.args(["-c", "true"]);
        assert_eq!(run_reporting(command, "CPU".to_string()), None);
    }
}
