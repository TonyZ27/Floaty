use std::ffi::c_void;

use objc2::rc::autoreleasepool;
use objc2::runtime::AnyObject;
use objc2::{class, msg_send};

use crate::error::{AppError, AppResult};

#[derive(Debug, Clone, Copy, serde::Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AccessibilityStatus {
    Granted,
    Denied,
}

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn AXIsProcessTrusted() -> bool;
    fn AXIsProcessTrustedWithOptions(options: *const c_void) -> bool;
}

/// Checks the current trust state without presenting a system prompt.
pub fn status() -> AccessibilityStatus {
    // SAFETY: AXIsProcessTrusted has no arguments and returns a scalar.
    if unsafe { AXIsProcessTrusted() } {
        AccessibilityStatus::Granted
    } else {
        AccessibilityStatus::Denied
    }
}

/// Requests trust using macOS's own Accessibility prompt.
///
/// Call this only in direct response to an explicit user action.
pub fn request_with_system_prompt() -> AccessibilityStatus {
    autoreleasepool(|_| {
        // SAFETY: all Objective-C objects are autoreleased and remain alive for
        // the duration of the AX call. No pointer escapes this pool.
        let trusted = unsafe {
            let key = ns_string(b"AXTrustedCheckOptionPrompt\0");
            let value: *mut AnyObject = msg_send![class!(NSNumber), numberWithBool: true];
            let options: *mut AnyObject =
                msg_send![class!(NSDictionary), dictionaryWithObject: value, forKey: key];
            AXIsProcessTrustedWithOptions(options.cast())
        };

        if trusted {
            AccessibilityStatus::Granted
        } else {
            AccessibilityStatus::Denied
        }
    })
}

/// Opens the macOS Accessibility privacy pane.
///
/// AppKit access is kept on the main thread. The caller should invoke this from
/// Tauri's main-thread dispatcher.
pub fn open_system_settings() -> AppResult<()> {
    require_main_thread()?;

    autoreleasepool(|_| {
        // SAFETY: URL and workspace are autoreleased process-owned AppKit
        // objects. `openURL:` returns a scalar and no object pointer escapes.
        let opened = unsafe {
            let url_string = ns_string(
                b"x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility\0",
            );
            let url: *mut AnyObject = msg_send![class!(NSURL), URLWithString: url_string];
            if url.is_null() {
                false
            } else {
                let workspace: *mut AnyObject = msg_send![class!(NSWorkspace), sharedWorkspace];
                let opened: bool = msg_send![workspace, openURL: url];
                opened
            }
        };

        if opened {
            Ok(())
        } else {
            Err(AppError::Other(
                "unable to open Accessibility settings".into(),
            ))
        }
    })
}

fn require_main_thread() -> AppResult<()> {
    // SAFETY: this reads a scalar process-wide AppKit thread property.
    let is_main_thread: bool = unsafe { msg_send![class!(NSThread), isMainThread] };
    if is_main_thread {
        Ok(())
    } else {
        Err(AppError::Other(
            "Accessibility settings must be opened on the main thread".into(),
        ))
    }
}

unsafe fn ns_string(value: &'static [u8]) -> *mut AnyObject {
    // SAFETY: callers pass static, NUL-terminated UTF-8 byte strings.
    unsafe { msg_send![class!(NSString), stringWithUTF8String: value.as_ptr()] }
}
