//! Password check for the lock screen: PAM, the same stack `login`/`su`
//! use. No hyprlock, no daemon: the bar opens a PAM transaction for the
//! session user and reports success or failure. The password never leaves
//! this process and is never logged.

use std::ffi::CStr;

/// PAM services to try, in order: a dedicated one if the user installed
/// it (a stricter stack), then the distro's standard authentication
/// stack (Arch/Fedora `system-auth`, Debian `common-auth`), then `login`.
const SERVICES: [&str; 5] = ["piuma", "hyprlock", "system-auth", "common-auth", "login"];

/// The first candidate the system actually has. `None` = no PAM config
/// at all, which locks the screen with no way to unlock it — the caller
/// refuses to show a lock it cannot open.
fn pick_service(exists: impl Fn(&str) -> bool) -> Option<&'static str> {
    SERVICES.into_iter().find(|name| exists(name))
}

fn pam_service() -> Option<&'static str> {
    pick_service(|name| {
        std::path::Path::new(&format!("/etc/pam.d/{name}")).is_file()
    })
}

/// The current session user's name (`getpwuid(getuid())`), the account
/// whose password PAM must verify. Not `$USER`: that is inherited and
/// could point at another account.
fn current_user() -> Option<String> {
    // SAFETY: getpwuid returns a pointer into a static buffer owned by
    // libc (valid until the next call on this thread), dereferenced
    // immediately.
    unsafe {
        let pw = libc::getpwuid(libc::getuid());
        if pw.is_null() {
            return None;
        }
        CStr::from_ptr((*pw).pw_name)
            .to_str()
            .ok()
            .map(str::to_string)
    }
}

/// Whether the lock screen can be unlocked at all: a PAM service and the
/// session user must be resolvable. Checked before showing the overlay.
pub(crate) fn available() -> bool {
    pam_service().is_some() && current_user().is_some()
}

/// Verify `password` for the session user. Blocking (PAM runs the whole
/// authentication stack), so call it off the UI thread.
pub(crate) fn verify(password: &str) -> Result<(), String> {
    let service = pam_service().ok_or("no PAM service in /etc/pam.d")?;
    let login = current_user().ok_or("cannot resolve the session user")?;
    let mut client = pam::Client::with_password(service)
        .map_err(|e| format!("PAM start: {e}"))?;
    client.conversation_mut().set_credentials(login, password);
    client
        .authenticate()
        .map_err(|e| format!("PAM: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The dedicated services win when present; otherwise the distro's
    /// standard stack. A system with none of them yields `None`, so the
    /// lock screen refuses to open instead of trapping the session.
    #[test]
    fn service_pick_prefers_the_specific_stacks() {
        assert_eq!(pick_service(|n| n == "system-auth"), Some("system-auth"));
        assert_eq!(
            pick_service(|n| n == "hyprlock" || n == "system-auth"),
            Some("hyprlock"),
            "a dedicated service beats the distro stack"
        );
        assert_eq!(
            pick_service(|n| n == "piuma" || n == "hyprlock"),
            Some("piuma")
        );
        assert_eq!(pick_service(|_| false), None);
    }
}
