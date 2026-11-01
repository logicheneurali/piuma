//! XDG Base Directory paths, shared by the bar and the setup editor.
//!
//! The spec's defaults are applied when a variable is absent, empty or
//! relative: `$HOME/.config`, `$HOME/.local/share`, `$HOME/.cache`, and
//! `/usr/local/share:/usr/share`. When even `$HOME` is missing (systemd
//! units, cron, minimal sessions) the passwd entry supplies it, like
//! the `dirs` crate did; with no passwd entry either, `None` lets every
//! caller keep its own fallback (the setup runs from `.`, the bar
//! reads `piuma.json` from the working directory, theme writes are
//! skipped). No environment is required for a normal desktop session.
use std::ffi::{OsStr, OsString};
use std::path::PathBuf;

/// Non-empty environment value.
fn var(key: &str) -> Option<OsString> {
    std::env::var_os(key).filter(|value| !value.is_empty())
}

/// An absolute path from an environment value; relative entries are
/// invalid per the spec and are treated as absent.
fn absolute(value: Option<OsString>) -> Option<PathBuf> {
    value
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
}

/// An XDG directory with its spec fallback under the user's home.
fn fallback_dir(value: Option<OsString>, fallback: &str) -> Option<PathBuf> {
    absolute(value).or_else(|| home().map(|home| home.join(fallback)))
}

/// The user's home: `$HOME` when absolute, else the passwd entry.
pub fn home() -> Option<PathBuf> {
    absolute(var("HOME")).or_else(passwd_home)
}

/// `$XDG_CONFIG_HOME` or `$HOME/.config`.
pub fn config() -> Option<PathBuf> {
    fallback_dir(var("XDG_CONFIG_HOME"), ".config")
}

/// `$XDG_DATA_HOME` or `$HOME/.local/share`.
pub fn data() -> Option<PathBuf> {
    fallback_dir(var("XDG_DATA_HOME"), ".local/share")
}

/// `$XDG_CACHE_HOME` or `$HOME/.cache`.
pub fn cache() -> Option<PathBuf> {
    fallback_dir(var("XDG_CACHE_HOME"), ".cache")
}

/// The data dirs to search: `$XDG_DATA_DIRS` (absolute entries only)
/// or `/usr/local/share:/usr/share` when it is absent or has no valid
/// entry.
pub fn data_dirs() -> Vec<PathBuf> {
    data_dirs_from(var("XDG_DATA_DIRS").as_deref())
}

/// `data()` followed by `data_dirs()`: the full freedesktop search path.
pub fn data_search_dirs() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = data().into_iter().collect();
    dirs.extend(data_dirs());
    dirs
}

/// Pure worker of [`data_dirs`] (tests pass a literal value).
fn data_dirs_from(raw: Option<&OsStr>) -> Vec<PathBuf> {
    let dirs: Vec<PathBuf> = raw.map(split_paths).unwrap_or_default();
    if dirs.is_empty() {
        vec![
            PathBuf::from("/usr/local/share"),
            PathBuf::from("/usr/share"),
        ]
    } else {
        dirs
    }
}

/// Split a colon-separated path list, keeping absolute entries only.
#[cfg(unix)]
fn split_paths(value: &OsStr) -> Vec<PathBuf> {
    use std::os::unix::ffi::{OsStrExt, OsStringExt};

    value
        .as_bytes()
        .split(|byte| *byte == b':')
        .filter(|part| !part.is_empty())
        .map(|part| PathBuf::from(OsString::from_vec(part.to_vec())))
        .filter(|path| path.is_absolute())
        .collect()
}

/// The passwd home when the environment cannot provide one. A long
/// path simply grows the buffer once the call reports `ERANGE`.
#[cfg(unix)]
fn passwd_home() -> Option<PathBuf> {
    use std::ffi::{CStr, OsString};
    use std::os::unix::ffi::OsStringExt;

    let uid = unsafe { libc::getuid() };
    let mut size = 1024usize;
    loop {
        let mut buffer = vec![0 as libc::c_char; size];
        let mut entry: libc::passwd = unsafe { std::mem::zeroed() };
        let mut result: *mut libc::passwd = std::ptr::null_mut();
        let code = unsafe {
            libc::getpwuid_r(
                uid,
                &mut entry,
                buffer.as_mut_ptr(),
                buffer.len(),
                &mut result,
            )
        };
        if code == libc::ERANGE && size < 65_536 {
            size *= 2;
            continue;
        }
        if code != 0 || result.is_null() || entry.pw_dir.is_null() {
            return None;
        }
        let bytes = unsafe { CStr::from_ptr(entry.pw_dir) }.to_bytes().to_vec();
        if bytes.is_empty() {
            return None;
        }
        let path = PathBuf::from(OsString::from_vec(bytes));
        return path.is_absolute().then_some(path);
    }
}

#[cfg(not(unix))]
fn passwd_home() -> Option<PathBuf> {
    None
}

#[cfg(not(unix))]
fn split_paths(value: &OsStr) -> Vec<PathBuf> {
    value
        .to_string_lossy()
        .split(':')
        .filter(|part| !part.is_empty())
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Relative or empty values never win over the spec fallback.
    #[test]
    fn relative_and_empty_values_are_ignored() {
        assert_eq!(absolute(None), None);
        assert_eq!(absolute(Some(OsString::from(""))), None);
        assert_eq!(absolute(Some(OsString::from("relative/dir"))), None);
        assert_eq!(
            absolute(Some(OsString::from("/abs/dir"))),
            Some(PathBuf::from("/abs/dir"))
        );
        assert_eq!(
            fallback_dir(Some(OsString::from("relative")), ".config"),
            home().map(|home| home.join(".config"))
        );
    }

    /// The data-dir list keeps absolute entries and falls back to the
    /// spec defaults when nothing valid is left.
    #[test]
    fn data_dirs_follow_the_spec() {
        let defaults = vec![PathBuf::from("/usr/local/share"), PathBuf::from("/usr/share")];
        assert_eq!(data_dirs_from(None), defaults);
        assert_eq!(data_dirs_from(Some(OsStr::new(""))), defaults);
        assert_eq!(data_dirs_from(Some(OsStr::new(":"))), defaults);
        assert_eq!(
            data_dirs_from(Some(OsStr::new("/opt/share:relative:/usr/share"))),
            vec![PathBuf::from("/opt/share"), PathBuf::from("/usr/share")]
        );
    }

    /// The search path starts from the user's data dir, then the system
    /// dirs; the cache lives under `$HOME/.cache` by default.
    #[test]
    fn search_path_starts_from_the_user_data_dir() {
        let dirs = data_search_dirs();
        if let Some(data) = data() {
            assert_eq!(dirs.first(), Some(&data));
        }
        if let Some(home) = home() {
            assert_eq!(cache(), Some(home.join(".cache")));
            assert_eq!(config(), Some(home.join(".config")));
        }
    }

    /// The passwd fallback yields an absolute directory (or nothing on
    /// a system without passwd entries, e.g. a container).
    #[test]
    fn passwd_home_is_absolute_when_present() {
        if let Some(home) = passwd_home() {
            assert!(home.is_absolute());
        }
    }
}
