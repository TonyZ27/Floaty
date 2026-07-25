use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::ffi::{c_char, c_void};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use libloading::Library;
use objc2::rc::autoreleasepool;
use objc2::runtime::AnyObject;
use objc2::{class, msg_send};

use crate::assignment::coordinator::{ObserverFactory, ObserverRegistration};
use crate::assignment::BindingEvent;
use crate::error::{AppError, AppResult};

#[derive(Debug, Clone, Copy, serde::Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AccessibilityStatus {
    Granted,
    Denied,
}

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn AXIsProcessTrusted() -> u8;
    fn AXIsProcessTrustedWithOptions(options: *const c_void) -> u8;
    fn AXUIElementCreateApplication(pid: i32) -> *mut c_void;
    fn AXUIElementCopyAttributeValues(
        element: *const c_void,
        attribute: *const c_void,
        index: isize,
        max_values: isize,
        values: *mut *mut c_void,
    ) -> i32;
    fn AXObserverCreate(pid: i32, callback: AxObserverCallback, observer: *mut *mut c_void) -> i32;
    fn AXObserverAddNotification(
        observer: *const c_void,
        element: *const c_void,
        notification: *const c_void,
        refcon: *mut c_void,
    ) -> i32;
    fn AXObserverRemoveNotification(
        observer: *const c_void,
        element: *const c_void,
        notification: *const c_void,
    ) -> i32;
    fn AXObserverGetRunLoopSource(observer: *const c_void) -> *mut c_void;
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFRetain(value: *const c_void) -> *const c_void;
    fn CFRelease(value: *const c_void);
    fn CFRunLoopGetMain() -> *mut c_void;
    fn CFRunLoopAddSource(run_loop: *mut c_void, source: *mut c_void, mode: *const c_void);
    fn CFRunLoopRemoveSource(run_loop: *mut c_void, source: *mut c_void, mode: *const c_void);
    fn CFStringCreateWithCString(
        allocator: *const c_void,
        value: *const c_char,
        encoding: u32,
    ) -> *mut c_void;
    fn CFStringGetCString(
        value: *const c_void,
        buffer: *mut c_char,
        buffer_size: isize,
        encoding: u32,
    ) -> u8;
    static kCFRunLoopDefaultMode: *const c_void;
}

type AxObserverCallback = unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void, *mut c_void);
type AxUiElementGetWindow = unsafe extern "C" fn(*const c_void, *mut u32) -> i32;

const AX_SUCCESS: i32 = 0;
const AX_ERROR_NOTIFICATION_UNSUPPORTED: i32 = -25205;
const AX_MAX_WINDOWS_PER_APPLICATION: isize = 1_024;
const UTF8_ENCODING: u32 = 0x0800_0100;
const HISERVICES_PATH: &str = "/System/Library/Frameworks/ApplicationServices.framework/Frameworks/HIServices.framework/HIServices";
const APPLICATION_SERVICES_PATH: &str =
    "/System/Library/Frameworks/ApplicationServices.framework/ApplicationServices";

const AX_WINDOWS_ATTRIBUTE: &[u8] = b"AXWindows\0";
const AX_DESTROYED_NOTIFICATION: &[u8] = b"AXUIElementDestroyed\0";
const AX_MINIMIZED_NOTIFICATION: &[u8] = b"AXWindowMiniaturized\0";
const AX_RESTORED_NOTIFICATION: &[u8] = b"AXWindowDeminiaturized\0";
const AX_RESIZED_NOTIFICATION: &[u8] = b"AXResized\0";
const AX_FULLSCREEN_NOTIFICATION: &[u8] = b"AXFullScreenChanged\0";

/// Checks the current trust state without presenting a system prompt.
pub fn status() -> AccessibilityStatus {
    // SAFETY: AXIsProcessTrusted has no arguments and returns a scalar.
    status_from_boolean(unsafe { AXIsProcessTrusted() })
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

        status_from_boolean(trusted)
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

fn status_from_boolean(value: u8) -> AccessibilityStatus {
    if value != 0 {
        AccessibilityStatus::Granted
    } else {
        AccessibilityStatus::Denied
    }
}

pub struct MacOSAccessibilityObserverFactory {
    app: tauri::AppHandle,
}

impl MacOSAccessibilityObserverFactory {
    pub fn new(app: tauri::AppHandle) -> Self {
        Self { app }
    }
}

#[async_trait]
impl ObserverFactory for MacOSAccessibilityObserverFactory {
    async fn observe(
        &self,
        pid: i32,
        window_id: u32,
        callback: Arc<dyn Fn(BindingEvent) + Send + Sync>,
    ) -> AppResult<Box<dyn ObserverRegistration>> {
        if status() != AccessibilityStatus::Granted {
            return Err(permission_required());
        }

        let observer_id = run_on_main_thread(&self.app, {
            let callback = callback.clone();
            move || {
                require_main_thread()?;
                NATIVE_OBSERVERS.with(|observers| {
                    let observer = unsafe { NativeObserver::create(pid, window_id, callback) }?;
                    let observer_id = NEXT_OBSERVER_ID.with(|next| {
                        let id = next.get();
                        next.set(id.saturating_add(1));
                        id
                    });
                    observers.borrow_mut().insert(observer_id, observer);
                    Ok(observer_id)
                })
            }
        })
        .await?;

        let permission_callback = callback.clone();
        let permission_task = tauri::async_runtime::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(1));
            interval.tick().await;
            loop {
                interval.tick().await;
                if status() != AccessibilityStatus::Granted {
                    permission_callback(BindingEvent::PermissionRevoked);
                    break;
                }
            }
        });

        Ok(Box::new(MacOSObserverRegistration {
            app: self.app.clone(),
            observer_id,
            permission_task: Some(permission_task),
            stopped: false,
        }))
    }
}

struct MacOSObserverRegistration {
    app: tauri::AppHandle,
    observer_id: u64,
    permission_task: Option<tauri::async_runtime::JoinHandle<()>>,
    stopped: bool,
}

#[async_trait]
impl ObserverRegistration for MacOSObserverRegistration {
    async fn stop(&mut self) -> AppResult<()> {
        if self.stopped {
            return Ok(());
        }
        if let Some(task) = self.permission_task.take() {
            task.abort();
        }
        let observer_id = self.observer_id;
        run_on_main_thread(&self.app, move || {
            NATIVE_OBSERVERS.with(|observers| {
                observers.borrow_mut().remove(&observer_id);
            });
            Ok(())
        })
        .await?;
        self.stopped = true;
        Ok(())
    }
}

impl Drop for MacOSObserverRegistration {
    fn drop(&mut self) {
        if self.stopped {
            return;
        }
        if let Some(task) = self.permission_task.take() {
            task.abort();
        }
        let observer_id = self.observer_id;
        let _ = self.app.run_on_main_thread(move || {
            NATIVE_OBSERVERS.with(|observers| {
                observers.borrow_mut().remove(&observer_id);
            });
        });
    }
}

thread_local! {
    static NATIVE_OBSERVERS: RefCell<HashMap<u64, NativeObserver>> = RefCell::new(HashMap::new());
    static NEXT_OBSERVER_ID: Cell<u64> = const { Cell::new(1) };
}

struct ObserverContext {
    callback: Arc<dyn Fn(BindingEvent) + Send + Sync>,
}

struct NativeObserver {
    observer: *mut c_void,
    window: *mut c_void,
    notifications: Vec<*mut c_void>,
    context: Box<ObserverContext>,
    source_installed: bool,
}

impl NativeObserver {
    unsafe fn create(
        pid: i32,
        window_id: u32,
        callback: Arc<dyn Fn(BindingEvent) + Send + Sync>,
    ) -> AppResult<Self> {
        let window = unsafe { copy_exact_window(pid, window_id)? };
        let mut observer: *mut c_void = std::ptr::null_mut();
        let create_result =
            unsafe { AXObserverCreate(pid, observer_callback, &mut observer as *mut *mut c_void) };
        if create_result != AX_SUCCESS || observer.is_null() {
            unsafe { CFRelease(window.cast_const()) };
            return Err(observer_unavailable());
        }

        let mut native = Self {
            observer,
            window,
            notifications: Vec::new(),
            context: Box::new(ObserverContext { callback }),
            source_installed: false,
        };
        let required = [
            AX_DESTROYED_NOTIFICATION,
            AX_MINIMIZED_NOTIFICATION,
            AX_RESTORED_NOTIFICATION,
            AX_RESIZED_NOTIFICATION,
        ];
        for notification in required {
            unsafe { native.add_notification(notification, false)? };
        }
        unsafe { native.add_notification(AX_FULLSCREEN_NOTIFICATION, true)? };

        let source = unsafe { AXObserverGetRunLoopSource(native.observer) };
        let run_loop = unsafe { CFRunLoopGetMain() };
        if source.is_null() || run_loop.is_null() {
            return Err(observer_unavailable());
        }
        unsafe { CFRunLoopAddSource(run_loop, source, kCFRunLoopDefaultMode) };
        native.source_installed = true;
        Ok(native)
    }

    unsafe fn add_notification(&mut self, name: &'static [u8], optional: bool) -> AppResult<()> {
        let notification = unsafe { cf_string(name)? };
        let context = (&mut *self.context as *mut ObserverContext).cast::<c_void>();
        let result =
            unsafe { AXObserverAddNotification(self.observer, self.window, notification, context) };
        if result == AX_SUCCESS {
            self.notifications.push(notification);
            Ok(())
        } else {
            unsafe { CFRelease(notification.cast_const()) };
            if optional && result == AX_ERROR_NOTIFICATION_UNSUPPORTED {
                Ok(())
            } else {
                Err(observer_unavailable())
            }
        }
    }
}

impl Drop for NativeObserver {
    fn drop(&mut self) {
        debug_assert!(require_main_thread().is_ok());
        unsafe {
            let source = AXObserverGetRunLoopSource(self.observer);
            let run_loop = CFRunLoopGetMain();
            if self.source_installed && !source.is_null() && !run_loop.is_null() {
                CFRunLoopRemoveSource(run_loop, source, kCFRunLoopDefaultMode);
            }
            for notification in self.notifications.drain(..) {
                let _ = AXObserverRemoveNotification(self.observer, self.window, notification);
                CFRelease(notification.cast_const());
            }
            CFRelease(self.window.cast_const());
            CFRelease(self.observer.cast_const());
        }
    }
}

unsafe extern "C" fn observer_callback(
    _observer: *mut c_void,
    _element: *mut c_void,
    notification: *mut c_void,
    refcon: *mut c_void,
) {
    if notification.is_null() || refcon.is_null() {
        return;
    }
    let Some(event) = (unsafe { notification_event(notification) }) else {
        return;
    };
    let context = unsafe { &*(refcon.cast::<ObserverContext>()) };
    (context.callback)(event);
}

unsafe fn notification_event(notification: *const c_void) -> Option<BindingEvent> {
    let mut buffer = [0_i8; 96];
    let copied = unsafe {
        CFStringGetCString(
            notification,
            buffer.as_mut_ptr(),
            buffer.len() as isize,
            UTF8_ENCODING,
        )
    };
    if copied == 0 {
        return None;
    }
    let name = unsafe { std::ffi::CStr::from_ptr(buffer.as_ptr()) }.to_bytes();
    match name {
        b"AXUIElementDestroyed" => Some(BindingEvent::TargetClosed),
        b"AXWindowMiniaturized" => Some(BindingEvent::TargetMinimized),
        b"AXWindowDeminiaturized" => Some(BindingEvent::TargetRestored),
        b"AXResized" | b"AXFullScreenChanged" => Some(BindingEvent::TargetSpaceChanged),
        _ => None,
    }
}

unsafe fn copy_exact_window(pid: i32, expected_window_id: u32) -> AppResult<*mut c_void> {
    let application = unsafe { AXUIElementCreateApplication(pid) };
    if application.is_null() {
        return Err(observer_unavailable());
    }

    let attribute = match unsafe { cf_string(AX_WINDOWS_ATTRIBUTE) } {
        Ok(attribute) => attribute,
        Err(error) => {
            unsafe { CFRelease(application.cast_const()) };
            return Err(error);
        }
    };
    let mut windows: *mut c_void = std::ptr::null_mut();
    let copy_result = unsafe {
        AXUIElementCopyAttributeValues(
            application,
            attribute,
            0,
            AX_MAX_WINDOWS_PER_APPLICATION,
            &mut windows,
        )
    };
    unsafe { CFRelease(attribute.cast_const()) };
    unsafe { CFRelease(application.cast_const()) };
    if copy_result != AX_SUCCESS || windows.is_null() {
        return Err(observer_unavailable());
    }

    let window_id_api = match unsafe { WindowIdApi::load() } {
        Ok(api) => api,
        Err(error) => {
            unsafe { CFRelease(windows.cast_const()) };
            return Err(error);
        }
    };
    let array = windows.cast::<AnyObject>();
    let count: usize = unsafe { msg_send![array, count] };
    let mut selected = std::ptr::null_mut();
    for index in 0..count {
        let window: *mut AnyObject = unsafe { msg_send![array, objectAtIndex: index] };
        let mut window_id = 0_u32;
        let result =
            unsafe { (window_id_api.get_window)(window.cast_const().cast(), &mut window_id) };
        if result == AX_SUCCESS && window_id == expected_window_id {
            selected = unsafe { CFRetain(window.cast_const().cast()) }.cast_mut();
            break;
        }
    }
    unsafe { CFRelease(windows.cast_const()) };
    if selected.is_null() {
        Err(observer_unavailable())
    } else {
        Ok(selected)
    }
}

struct WindowIdApi {
    _library: Library,
    get_window: AxUiElementGetWindow,
}

impl WindowIdApi {
    unsafe fn load() -> AppResult<Self> {
        for path in [HISERVICES_PATH, APPLICATION_SERVICES_PATH] {
            let Ok(library) = (unsafe { Library::new(path) }) else {
                continue;
            };
            let get_window = {
                let Ok(symbol) =
                    (unsafe { library.get::<AxUiElementGetWindow>(b"_AXUIElementGetWindow\0") })
                else {
                    continue;
                };
                *symbol
            };
            return Ok(Self {
                _library: library,
                get_window,
            });
        }
        Err(observer_unavailable())
    }
}

unsafe fn cf_string(value: &'static [u8]) -> AppResult<*mut c_void> {
    let string = unsafe {
        CFStringCreateWithCString(
            std::ptr::null(),
            value.as_ptr().cast::<c_char>(),
            UTF8_ENCODING,
        )
    };
    if string.is_null() {
        Err(observer_unavailable())
    } else {
        Ok(string)
    }
}

async fn run_on_main_thread<T, F>(app: &tauri::AppHandle, operation: F) -> AppResult<T>
where
    T: Send + 'static,
    F: FnOnce() -> AppResult<T> + Send + 'static,
{
    let (sender, receiver) = tokio::sync::oneshot::channel();
    app.run_on_main_thread(move || {
        let _ = sender.send(operation());
    })
    .map_err(|error| AppError::Other(format!("main-thread dispatch failed: {error}")))?;
    receiver
        .await
        .map_err(|_| AppError::Other("main-thread operation was cancelled".into()))?
}

fn permission_required() -> AppError {
    AppError::Other("Accessibility permission required".into())
}

fn observer_unavailable() -> AppError {
    AppError::Other("Accessibility window observation unavailable".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apple_boolean_is_converted_without_assuming_rust_bool_abi() {
        assert_eq!(status_from_boolean(0), AccessibilityStatus::Denied);
        assert_eq!(status_from_boolean(1), AccessibilityStatus::Granted);
        assert_eq!(status_from_boolean(u8::MAX), AccessibilityStatus::Granted);
    }

    #[test]
    fn observer_notifications_map_to_semantic_binding_events() {
        unsafe fn event(name: &'static [u8]) -> Option<BindingEvent> {
            let notification = unsafe { cf_string(name).unwrap() };
            let event = unsafe { notification_event(notification) };
            unsafe { CFRelease(notification.cast_const()) };
            event
        }

        assert_eq!(
            unsafe { event(AX_DESTROYED_NOTIFICATION) },
            Some(BindingEvent::TargetClosed)
        );
        assert_eq!(
            unsafe { event(AX_MINIMIZED_NOTIFICATION) },
            Some(BindingEvent::TargetMinimized)
        );
        assert_eq!(
            unsafe { event(AX_RESTORED_NOTIFICATION) },
            Some(BindingEvent::TargetRestored)
        );
        assert_eq!(
            unsafe { event(AX_RESIZED_NOTIFICATION) },
            Some(BindingEvent::TargetSpaceChanged)
        );
        assert_eq!(
            unsafe { event(AX_FULLSCREEN_NOTIFICATION) },
            Some(BindingEvent::TargetSpaceChanged)
        );
    }
}
