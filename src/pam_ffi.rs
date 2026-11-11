//! Minimal Linux-PAM client for the lock screen: `pam_start`,
//! `pam_authenticate`, `pam_acct_mgmt` and `pam_end`, plus the
//! conversation callback PAM uses to ask for the password.
//!
//! The `pam`/`pam-sys` crates generate their bindings with bindgen while
//! building, which makes libclang a build dependency just to call a system
//! library. The declarations below are the fixed part of the API this
//! project needs.

use std::ffi::{c_char, c_int, c_void, CString};
use std::mem::size_of;
use std::ptr;

/// `struct pam_message`: the message style and its text.
#[repr(C)]
struct PamMessage {
    msg_style: c_int,
    msg: *const c_char,
}

/// `struct pam_response`: PAM owns and frees it after the callback.
#[repr(C)]
struct PamResponse {
    resp: *mut c_char,
    resp_retcode: c_int,
}

type ConvFn = unsafe extern "C" fn(
    c_int,
    *mut *const PamMessage,
    *mut *mut PamResponse,
    *mut c_void,
) -> c_int;

#[repr(C)]
struct PamConv {
    conv: Option<ConvFn>,
    appdata_ptr: *mut c_void,
}

type PamHandle = *mut c_void;

#[link(name = "pam")]
extern "C" {
    fn pam_start(
        service: *const c_char,
        user: *const c_char,
        conv: *const PamConv,
        handle: *mut PamHandle,
    ) -> c_int;
    fn pam_authenticate(handle: PamHandle, flags: c_int) -> c_int;
    fn pam_acct_mgmt(handle: PamHandle, flags: c_int) -> c_int;
    fn pam_end(handle: PamHandle, status: c_int) -> c_int;
}

// `security/_pam_types.h`: the prompt styles and the codes a login uses.
const PAM_SUCCESS: c_int = 0;
const PAM_PROMPT_ECHO_OFF: c_int = 1;
const PAM_PROMPT_ECHO_ON: c_int = 2;
const PAM_ERROR_MSG: c_int = 3;
const PAM_TEXT_INFO: c_int = 4;
const PAM_BUF_ERR: c_int = 5;
const PAM_CONV_ERR: c_int = 19;

/// What the conversation answers: the session user for the echo-on
/// prompt and the password for the blind one.
struct Credentials {
    login: CString,
    password: CString,
}

/// `pam_conv.conv`: PAM asks one question per message; the response array
/// and its strings are malloc'd because PAM frees them itself.
unsafe extern "C" fn converse(
    num_msg: c_int,
    msg: *mut *const PamMessage,
    response: *mut *mut PamResponse,
    appdata: *mut c_void,
) -> c_int {
    if num_msg <= 0 || msg.is_null() || response.is_null() || appdata.is_null() {
        return PAM_CONV_ERR;
    }
    let replies = libc::calloc(num_msg as usize, size_of::<PamResponse>()) as *mut PamResponse;
    if replies.is_null() {
        return PAM_BUF_ERR;
    }
    let credentials = &*(appdata as *const Credentials);
    for i in 0..num_msg as isize {
        let message = *msg.offset(i);
        let answer = match (*message).msg_style {
            PAM_PROMPT_ECHO_OFF => Some(&credentials.password),
            PAM_PROMPT_ECHO_ON => Some(&credentials.login),
            // Informational messages carry no answer.
            PAM_TEXT_INFO => None,
            // An error message aborts the exchange, like the pam crate.
            PAM_ERROR_MSG => return fail(replies, i),
            _ => return fail(replies, i),
        };
        if let Some(answer) = answer {
            let copy = libc::strdup(answer.as_ptr());
            if copy.is_null() {
                return fail(replies, i);
            }
            (*replies.offset(i)).resp = copy;
        }
    }
    *response = replies;
    PAM_SUCCESS
}

/// Free a half-built response array (answers included) and report the
/// conversation error.
unsafe fn fail(replies: *mut PamResponse, filled: isize) -> c_int {
    for i in 0..filled {
        libc::free((*replies.offset(i)).resp as *mut c_void);
    }
    libc::free(replies as *mut c_void);
    PAM_CONV_ERR
}

/// Verify `password` for `login` against `service` (a `/etc/pam.d`
/// name). Blocking: PAM runs the whole stack, so call it off the UI
/// thread.
pub(crate) fn authenticate(service: &str, login: &str, password: &str) -> Result<(), String> {
    let service = CString::new(service).map_err(|_| "PAM: invalid service name")?;
    let login = CString::new(login).map_err(|_| "PAM: invalid user name")?;
    let credentials = Credentials {
        login: login.clone(),
        password: CString::new(password).map_err(|_| "PAM: password contains a NUL")?,
    };
    let conversation = PamConv {
        conv: Some(converse),
        appdata_ptr: &credentials as *const Credentials as *mut c_void,
    };
    let mut handle: PamHandle = ptr::null_mut();
    // SAFETY: the strings and the conversation outlive every call (PAM
    // only reads them) and the handle is closed before they are dropped.
    unsafe {
        let mut status = pam_start(service.as_ptr(), login.as_ptr(), &conversation, &mut handle);
        if status != PAM_SUCCESS {
            return Err(format!("PAM start: {}", code(status)));
        }
        // The pam crate checked both: an expired account must not unlock.
        status = pam_authenticate(handle, 0);
        if status == PAM_SUCCESS {
            status = pam_acct_mgmt(handle, 0);
        }
        pam_end(handle, status);
        if status == PAM_SUCCESS {
            Ok(())
        } else {
            Err(format!("PAM: {}", code(status)))
        }
    }
}

/// Name of the codes a failed login returns: the card only says "Wrong
/// password", this ends up in the log.
fn code(status: c_int) -> String {
    match status {
        6 => "permission denied".to_string(),
        7 => "authentication failure".to_string(),
        9 => "authentication information unavailable".to_string(),
        10 => "unknown user".to_string(),
        11 => "maximum tries exceeded".to_string(),
        12 => "password change required".to_string(),
        13 => "account expired".to_string(),
        19 => "conversation error".to_string(),
        other => format!("error {other}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CStr;

    /// Both prompts are answered from the credentials: the password for
    /// the blind prompt, the login for the echo-on one.
    #[test]
    fn conversation_answers_both_prompts() {
        for (style, expected) in [
            (PAM_PROMPT_ECHO_OFF, "secret"),
            (PAM_PROMPT_ECHO_ON, "user"),
        ] {
            let credentials = Credentials {
                login: CString::new("user").unwrap(),
                password: CString::new("secret").unwrap(),
            };
            let question = CString::new("prompt").unwrap();
            let mut message = PamMessage {
                msg_style: style,
                msg: question.as_ptr(),
            };
            let mut messages = [&mut message as *const PamMessage];
            let mut responses: *mut PamResponse = ptr::null_mut();
            let status = unsafe {
                converse(
                    1,
                    messages.as_mut_ptr(),
                    &mut responses,
                    &credentials as *const Credentials as *mut c_void,
                )
            };
            assert_eq!(status, PAM_SUCCESS);
            assert!(!responses.is_null());
            unsafe {
                let answer = CStr::from_ptr((*responses).resp).to_string_lossy();
                assert_eq!(answer, expected);
                libc::free((*responses).resp as *mut c_void);
                libc::free(responses as *mut c_void);
            }
        }
    }

    /// An error message aborts the conversation instead of answering it.
    #[test]
    fn conversation_rejects_error_messages() {
        let credentials = Credentials {
            login: CString::new("user").unwrap(),
            password: CString::new("secret").unwrap(),
        };
        let question = CString::new("boom").unwrap();
        let mut message = PamMessage {
            msg_style: PAM_ERROR_MSG,
            msg: question.as_ptr(),
        };
        let mut messages = [&mut message as *const PamMessage];
        let mut responses: *mut PamResponse = ptr::null_mut();
        let status = unsafe {
            converse(
                1,
                messages.as_mut_ptr(),
                &mut responses,
                &credentials as *const Credentials as *mut c_void,
            )
        };
        assert_eq!(status, PAM_CONV_ERR);
        assert!(responses.is_null());
    }
}
