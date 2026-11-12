//! Restart the running bar from the setup's `Reload Piuma` button.
//!
//! The bar creates its layer surfaces at boot, so a new/removed panel
//! cannot appear while it runs. This helper stops it (SIGTERM lets the
//! exit cleanup restore the app themes and the dynamic gaps) and starts
//! the sibling binary again with the edited config.

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::thread::sleep;
use std::time::{Duration, Instant};

/// The bar binary next to `piuma-setup`: both ship in the same directory.
fn bar_binary() -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| format!("piuma-setup path: {e}"))?;
    let bar = exe
        .parent()
        .ok_or("piuma-setup has no parent directory")?
        .join("piuma");
    bar.is_file()
        .then_some(bar.clone())
        .ok_or_else(|| format!("piuma not found next to piuma-setup ({})", bar.display()))
}

/// Ask the running bar to show its screensaver now: `SIGUSR1` reaches
/// the signal subscription (see `screensaver::signal_subscription`), so
/// the overlay appears without waiting for the idle timer.
pub(crate) fn trigger_screensaver() -> Result<(), String> {
    let out = Command::new("pkill")
        .args(["-USR1", "-x", "piuma"])
        .output()
        .map_err(|e| format!("cannot signal piuma: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err("piuma is not running".to_string())
    }
}

/// Restart the bar with `config`. A no-op wait for a bar that was not
/// running: the button doubles as "start it".
pub(crate) fn reload(config: PathBuf) -> Result<(), String> {
    let bar = bar_binary()?;
    // `piuma-setup` does not match `-x piuma`: only the bar is signalled.
    let _ = Command::new("pkill")
        .args(["-TERM", "-x", "piuma"])
        .status();
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        let running = Command::new("pgrep")
            .args(["-x", "piuma"])
            .status()
            .is_ok_and(|status| status.success());
        if !running {
            break;
        }
        sleep(Duration::from_millis(100));
    }
    let still_running = Command::new("pgrep")
        .args(["-x", "piuma"])
        .status()
        .is_ok_and(|status| status.success());
    if still_running {
        return Err("piuma did not stop — reload it manually".to_string());
    }
    // `setsid -f` detaches the bar from the setup's session/terminal.
    let mut child = Command::new("setsid")
        .arg("-f")
        .arg(&bar)
        .arg("--config")
        .arg(&config)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("cannot start piuma: {e}"))?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}
