//! iOS backend: calls the Swift bridge in this crate's `ios/` package through C symbols.
//!
//! Swift exports `dga_iap_ios_command` with `@_cdecl`; Rust exports `dga_iap_on_message`,
//! which Swift calls with every result. Both sides exchange UTF-8 JSON strings.

use std::ffi::{CStr, CString, c_char};

use super::Backend;
use crate::bridge::{self, Command};

unsafe extern "C" {
    fn dga_iap_ios_command(json: *const c_char);
}

pub(crate) struct IosBackend;

impl Backend for IosBackend {
    fn send(&self, command: Command) {
        let json = match serde_json::to_string(&command).map(CString::new) {
            Ok(Ok(json)) => json,
            _ => return tracing::error!("dga_bevy_iap: can't encode {command:?}"),
        };
        // SAFETY: Swift copies the string before returning.
        unsafe { dga_iap_ios_command(json.as_ptr()) };
    }
}

/// Called by the Swift bridge with a JSON result. Swift owns `json`; it is only read here.
#[unsafe(no_mangle)]
pub extern "C" fn dga_iap_on_message(json: *const c_char) {
    if json.is_null() {
        return;
    }
    // SAFETY: Swift passes a valid NUL-terminated string that outlives this call.
    match unsafe { CStr::from_ptr(json) }.to_str() {
        Ok(json) => bridge::post_native_json(json),
        Err(err) => tracing::error!("dga_bevy_iap: non-UTF-8 message from Swift bridge: {err}"),
    }
}
