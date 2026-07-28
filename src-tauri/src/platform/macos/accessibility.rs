use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::ffi::{c_char, c_void};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use libloading::Library;
use objc2::rc::{autoreleasepool, Allocated, Retained};
use objc2::runtime::{AnyObject, NSObject};
use objc2::{
    class, declare_class, msg_send, msg_send_id, mutability, sel, ClassType, DeclaredClass,
};

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
    fn AXUIElementCopyAttributeValue(
        element: *const c_void,
        attribute: *const c_void,
        value: *mut *mut c_void,
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
const AX_FOCUSED_WINDOW_ATTRIBUTE: &[u8] = b"AXFocusedWindow\0";
const AX_FULLSCREEN_ATTRIBUTE: &[u8] = b"AXFullScreen\0";
const AX_DESTROYED_NOTIFICATION: &[u8] = b"AXUIElementDestroyed\0";
const AX_MINIMIZED_NOTIFICATION: &[u8] = b"AXWindowMiniaturized\0";
const AX_RESTORED_NOTIFICATION: &[u8] = b"AXWindowDeminiaturized\0";
const AX_RESIZED_NOTIFICATION: &[u8] = b"AXResized\0";
const AX_FULLSCREEN_NOTIFICATION: &[u8] = b"AXFullScreenChanged\0";
const AX_APPLICATION_ACTIVATED_NOTIFICATION: &[u8] = b"AXApplicationActivated\0";
const AX_APPLICATION_DEACTIVATED_NOTIFICATION: &[u8] = b"AXApplicationDeactivated\0";
const AX_FOCUSED_WINDOW_CHANGED_NOTIFICATION: &[u8] = b"AXFocusedWindowChanged\0";
const AX_MAIN_WINDOW_CHANGED_NOTIFICATION: &[u8] = b"AXMainWindowChanged\0";
const NS_WORKSPACE_DID_ACTIVATE_APPLICATION_NOTIFICATION: &[u8] =
    b"NSWorkspaceDidActivateApplicationNotification\0";

struct WorkspaceActivationIvars {
    pid: i32,
    target_window_id: u32,
    callback: Arc<dyn Fn(BindingEvent) + Send + Sync>,
}

declare_class!(
    struct WorkspaceActivationObserver;

    // SAFETY: NSObject has no subclassing requirements and the ivars are only
    // read after initialization. Notification delivery occurs on the main thread.
    unsafe impl ClassType for WorkspaceActivationObserver {
        type Super = NSObject;
        type Mutability = mutability::InteriorMutable;
        const NAME: &'static str = "FloatyWorkspaceActivationObserver";
    }

    impl DeclaredClass for WorkspaceActivationObserver {
        type Ivars = WorkspaceActivationIvars;
    }

    unsafe impl WorkspaceActivationObserver {
        #[method(workspaceApplicationActivated:)]
        fn workspace_application_activated(&self, _notification: *mut AnyObject) {
            let ivars = self.ivars();
            (ivars.callback)(target_focus_event(ivars.pid, ivars.target_window_id));
        }
    }
);

impl WorkspaceActivationObserver {
    fn new(
        pid: i32,
        target_window_id: u32,
        callback: Arc<dyn Fn(BindingEvent) + Send + Sync>,
    ) -> Retained<Self> {
        // SAFETY: the declared class initializes its ivars before invoking
        // NSObject's designated initializer.
        unsafe {
            let allocated: Allocated<Self> = msg_send_id![Self::class(), alloc];
            let initialized = allocated.set_ivars(WorkspaceActivationIvars {
                pid,
                target_window_id,
                callback,
            });
            msg_send_id![super(initialized), init]
        }
    }
}

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
        let correction_app = self.app.clone();
        let permission_task = tauri::async_runtime::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(1));
            interval.tick().await;
            loop {
                interval.tick().await;
                if status() != AccessibilityStatus::Granted {
                    permission_callback(BindingEvent::PermissionRevoked);
                    break;
                }
                let correction = run_on_main_thread(&correction_app, move || {
                    Ok(target_focus_event(pid, window_id))
                })
                .await;
                if let Ok(event) = correction {
                    permission_callback(event);
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
        let observer_id = self.observer_id;
        run_on_main_thread(&self.app, move || {
            NATIVE_OBSERVERS.with(|observers| {
                observers.borrow_mut().remove(&observer_id);
            });
            Ok(())
        })
        .await?;
        if let Some(task) = self.permission_task.take() {
            task.abort();
        }
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
    pid: i32,
    target_window_id: u32,
    callback: Arc<dyn Fn(BindingEvent) + Send + Sync>,
}

struct NativeObserver {
    observer: *mut c_void,
    application: *mut c_void,
    window: *mut c_void,
    notifications: Vec<NativeNotification>,
    context: Box<ObserverContext>,
    workspace_activation_observer: Option<Retained<WorkspaceActivationObserver>>,
    source_installed: bool,
}

struct NativeNotification {
    element: *mut c_void,
    name: *mut c_void,
}

impl NativeObserver {
    unsafe fn create(
        pid: i32,
        window_id: u32,
        callback: Arc<dyn Fn(BindingEvent) + Send + Sync>,
    ) -> AppResult<Self> {
        let window = unsafe { copy_exact_window(pid, window_id)? };
        let application = unsafe { AXUIElementCreateApplication(pid) };
        if application.is_null() {
            unsafe { CFRelease(window.cast_const()) };
            return Err(observer_unavailable());
        }
        let mut observer: *mut c_void = std::ptr::null_mut();
        let create_result =
            unsafe { AXObserverCreate(pid, observer_callback, &mut observer as *mut *mut c_void) };
        if create_result != AX_SUCCESS || observer.is_null() {
            unsafe { CFRelease(application.cast_const()) };
            unsafe { CFRelease(window.cast_const()) };
            return Err(observer_unavailable());
        }

        let mut native = Self {
            observer,
            application,
            window,
            notifications: Vec::new(),
            context: Box::new(ObserverContext {
                pid,
                target_window_id: window_id,
                callback: callback.clone(),
            }),
            workspace_activation_observer: None,
            source_installed: false,
        };
        let required = [
            AX_DESTROYED_NOTIFICATION,
            AX_MINIMIZED_NOTIFICATION,
            AX_RESTORED_NOTIFICATION,
            AX_RESIZED_NOTIFICATION,
        ];
        for notification in required {
            unsafe { native.add_notification(native.window, notification, false)? };
        }
        unsafe { native.add_notification(native.window, AX_FULLSCREEN_NOTIFICATION, true)? };
        for notification in [
            AX_APPLICATION_ACTIVATED_NOTIFICATION,
            AX_APPLICATION_DEACTIVATED_NOTIFICATION,
            AX_FOCUSED_WINDOW_CHANGED_NOTIFICATION,
            AX_MAIN_WINDOW_CHANGED_NOTIFICATION,
        ] {
            unsafe { native.add_notification(native.application, notification, true)? };
        }

        let source = unsafe { AXObserverGetRunLoopSource(native.observer) };
        let run_loop = unsafe { CFRunLoopGetMain() };
        if source.is_null() || run_loop.is_null() {
            return Err(observer_unavailable());
        }
        unsafe { CFRunLoopAddSource(run_loop, source, kCFRunLoopDefaultMode) };
        native.source_installed = true;
        native.workspace_activation_observer =
            Some(unsafe { install_workspace_activation_observer(pid, window_id, callback) });
        Ok(native)
    }

    unsafe fn add_notification(
        &mut self,
        element: *mut c_void,
        name: &'static [u8],
        optional: bool,
    ) -> AppResult<()> {
        let notification = unsafe { cf_string(name)? };
        let context = (&mut *self.context as *mut ObserverContext).cast::<c_void>();
        let result =
            unsafe { AXObserverAddNotification(self.observer, element, notification, context) };
        if result == AX_SUCCESS {
            self.notifications.push(NativeNotification {
                element,
                name: notification,
            });
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
            if let Some(observer) = self.workspace_activation_observer.take() {
                remove_workspace_activation_observer(&observer);
            }
            let source = AXObserverGetRunLoopSource(self.observer);
            let run_loop = CFRunLoopGetMain();
            if self.source_installed && !source.is_null() && !run_loop.is_null() {
                CFRunLoopRemoveSource(run_loop, source, kCFRunLoopDefaultMode);
            }
            for notification in self.notifications.drain(..) {
                let _ = AXObserverRemoveNotification(
                    self.observer,
                    notification.element,
                    notification.name,
                );
                CFRelease(notification.name.cast_const());
            }
            CFRelease(self.application.cast_const());
            CFRelease(self.window.cast_const());
            CFRelease(self.observer.cast_const());
        }
    }
}

unsafe fn install_workspace_activation_observer(
    pid: i32,
    target_window_id: u32,
    callback: Arc<dyn Fn(BindingEvent) + Send + Sync>,
) -> Retained<WorkspaceActivationObserver> {
    let observer = WorkspaceActivationObserver::new(pid, target_window_id, callback);
    let workspace: *mut AnyObject = unsafe { msg_send![class!(NSWorkspace), sharedWorkspace] };
    let center: *mut AnyObject = unsafe { msg_send![workspace, notificationCenter] };
    let name = unsafe { ns_string(NS_WORKSPACE_DID_ACTIVATE_APPLICATION_NOTIFICATION) };
    unsafe {
        let _: () = msg_send![
            center,
            addObserver: &*observer,
            selector: sel!(workspaceApplicationActivated:),
            name: name,
            object: std::ptr::null::<AnyObject>()
        ];
    }
    observer
}

unsafe fn remove_workspace_activation_observer(observer: &WorkspaceActivationObserver) {
    let workspace: *mut AnyObject = unsafe { msg_send![class!(NSWorkspace), sharedWorkspace] };
    let center: *mut AnyObject = unsafe { msg_send![workspace, notificationCenter] };
    let name = unsafe { ns_string(NS_WORKSPACE_DID_ACTIVATE_APPLICATION_NOTIFICATION) };
    unsafe {
        let _: () = msg_send![
            center,
            removeObserver: observer,
            name: name,
            object: std::ptr::null::<AnyObject>()
        ];
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
    let Some(kind) = (unsafe { notification_kind(notification) }) else {
        return;
    };
    let context = unsafe { &*(refcon.cast::<ObserverContext>()) };
    let event = match kind {
        NativeNotificationKind::Event(event) => event,
        NativeNotificationKind::EvaluateFocus => {
            target_focus_event(context.pid, context.target_window_id)
        }
    };
    (context.callback)(event);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NativeNotificationKind {
    Event(BindingEvent),
    EvaluateFocus,
}

unsafe fn notification_kind(notification: *const c_void) -> Option<NativeNotificationKind> {
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
        b"AXUIElementDestroyed" => Some(NativeNotificationKind::Event(BindingEvent::TargetClosed)),
        b"AXWindowMiniaturized" => {
            Some(NativeNotificationKind::Event(BindingEvent::TargetMinimized))
        }
        b"AXWindowDeminiaturized" => {
            Some(NativeNotificationKind::Event(BindingEvent::TargetRestored))
        }
        b"AXResized" | b"AXFullScreenChanged" => Some(NativeNotificationKind::Event(
            BindingEvent::TargetSpaceChanged,
        )),
        b"AXApplicationDeactivated" => {
            Some(NativeNotificationKind::Event(BindingEvent::TargetUnfocused))
        }
        b"AXApplicationActivated" | b"AXFocusedWindowChanged" | b"AXMainWindowChanged" => {
            Some(NativeNotificationKind::EvaluateFocus)
        }
        _ => None,
    }
}

fn target_focus_event(pid: i32, target_window_id: u32) -> BindingEvent {
    let frontmost_pid = frontmost_application_pid();
    if frontmost_pid != Some(pid) {
        return BindingEvent::TargetUnfocused;
    }

    let (focused_window_id, is_fullscreen) = unsafe { focused_window_state(pid) };
    focus_event(
        frontmost_pid,
        pid,
        focused_window_id,
        target_window_id,
        is_fullscreen,
    )
}

/// Re-evaluates the live frontmost application and exact AX focused window.
///
/// Call this on the macOS main thread immediately before mutating window order;
/// notification payloads may already be stale by the time async handling runs.
pub(crate) fn exact_target_is_focused(pid: i32, target_window_id: u32) -> bool {
    target_focus_event(pid, target_window_id) == BindingEvent::TargetFocused
}

fn focus_event(
    frontmost_pid: Option<i32>,
    target_pid: i32,
    focused_window_id: Option<u32>,
    target_window_id: u32,
    is_fullscreen: Option<bool>,
) -> BindingEvent {
    if frontmost_pid == Some(target_pid)
        && focused_window_id == Some(target_window_id)
        && is_fullscreen == Some(false)
    {
        BindingEvent::TargetFocused
    } else {
        BindingEvent::TargetUnfocused
    }
}

fn frontmost_application_pid() -> Option<i32> {
    autoreleasepool(|_| {
        // SAFETY: callers run this on the main thread. The returned application
        // is autoreleased and only its scalar process identifier leaves the pool.
        unsafe {
            let workspace: *mut AnyObject = msg_send![class!(NSWorkspace), sharedWorkspace];
            let application: *mut AnyObject = msg_send![workspace, frontmostApplication];
            if application.is_null() {
                None
            } else {
                Some(msg_send![application, processIdentifier])
            }
        }
    })
}

unsafe fn focused_window_state(pid: i32) -> (Option<u32>, Option<bool>) {
    let application = unsafe { AXUIElementCreateApplication(pid) };
    if application.is_null() {
        return (None, None);
    }
    let attribute = match unsafe { cf_string(AX_FOCUSED_WINDOW_ATTRIBUTE) } {
        Ok(attribute) => attribute,
        Err(_) => {
            unsafe { CFRelease(application.cast_const()) };
            return (None, None);
        }
    };
    let mut focused_window: *mut c_void = std::ptr::null_mut();
    let copy_result =
        unsafe { AXUIElementCopyAttributeValue(application, attribute, &mut focused_window) };
    unsafe { CFRelease(attribute.cast_const()) };
    unsafe { CFRelease(application.cast_const()) };
    if copy_result != AX_SUCCESS || focused_window.is_null() {
        if !focused_window.is_null() {
            unsafe { CFRelease(focused_window.cast_const()) };
        }
        return (None, None);
    }

    let window_id = unsafe { copy_window_id(focused_window) };
    let is_fullscreen = unsafe { copy_ax_bool(focused_window, AX_FULLSCREEN_ATTRIBUTE) };
    unsafe { CFRelease(focused_window.cast_const()) };
    (window_id, is_fullscreen)
}

unsafe fn copy_window_id(window: *const c_void) -> Option<u32> {
    let window_id_api = unsafe { WindowIdApi::load() }.ok()?;
    let mut window_id = 0_u32;
    let result = unsafe { (window_id_api.get_window)(window, &mut window_id) };
    (result == AX_SUCCESS).then_some(window_id)
}

unsafe fn copy_ax_bool(element: *const c_void, attribute_name: &'static [u8]) -> Option<bool> {
    let attribute = unsafe { cf_string(attribute_name) }.ok()?;
    let mut value: *mut c_void = std::ptr::null_mut();
    let result = unsafe { AXUIElementCopyAttributeValue(element, attribute, &mut value) };
    unsafe { CFRelease(attribute.cast_const()) };
    if result != AX_SUCCESS || value.is_null() {
        if !value.is_null() {
            unsafe { CFRelease(value.cast_const()) };
        }
        return None;
    }
    let result: bool = unsafe { msg_send![value.cast::<AnyObject>(), boolValue] };
    unsafe { CFRelease(value.cast_const()) };
    Some(result)
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
        unsafe fn kind(name: &'static [u8]) -> Option<NativeNotificationKind> {
            let notification = unsafe { cf_string(name).unwrap() };
            let kind = unsafe { notification_kind(notification) };
            unsafe { CFRelease(notification.cast_const()) };
            kind
        }

        assert_eq!(
            unsafe { kind(AX_DESTROYED_NOTIFICATION) },
            Some(NativeNotificationKind::Event(BindingEvent::TargetClosed))
        );
        assert_eq!(
            unsafe { kind(AX_MINIMIZED_NOTIFICATION) },
            Some(NativeNotificationKind::Event(BindingEvent::TargetMinimized))
        );
        assert_eq!(
            unsafe { kind(AX_RESTORED_NOTIFICATION) },
            Some(NativeNotificationKind::Event(BindingEvent::TargetRestored))
        );
        assert_eq!(
            unsafe { kind(AX_RESIZED_NOTIFICATION) },
            Some(NativeNotificationKind::Event(
                BindingEvent::TargetSpaceChanged
            ))
        );
        assert_eq!(
            unsafe { kind(AX_FULLSCREEN_NOTIFICATION) },
            Some(NativeNotificationKind::Event(
                BindingEvent::TargetSpaceChanged
            ))
        );
        assert_eq!(
            unsafe { kind(AX_APPLICATION_DEACTIVATED_NOTIFICATION) },
            Some(NativeNotificationKind::Event(BindingEvent::TargetUnfocused))
        );
        for notification in [
            AX_APPLICATION_ACTIVATED_NOTIFICATION,
            AX_FOCUSED_WINDOW_CHANGED_NOTIFICATION,
            AX_MAIN_WINDOW_CHANGED_NOTIFICATION,
        ] {
            assert_eq!(
                unsafe { kind(notification) },
                Some(NativeNotificationKind::EvaluateFocus)
            );
        }
    }

    #[test]
    fn focus_requires_frontmost_app_exact_window_and_non_fullscreen_state() {
        let target_pid = 41;
        let target_window_id = 73;

        assert_eq!(
            focus_event(
                Some(target_pid),
                target_pid,
                Some(target_window_id),
                target_window_id,
                Some(false),
            ),
            BindingEvent::TargetFocused
        );

        for (frontmost_pid, focused_window_id, is_fullscreen) in [
            (Some(99), Some(target_window_id), Some(false)),
            (Some(target_pid), Some(88), Some(false)),
            (Some(target_pid), None, Some(false)),
            (Some(target_pid), Some(target_window_id), Some(true)),
            (Some(target_pid), Some(target_window_id), None),
        ] {
            assert_eq!(
                focus_event(
                    frontmost_pid,
                    target_pid,
                    focused_window_id,
                    target_window_id,
                    is_fullscreen,
                ),
                BindingEvent::TargetUnfocused
            );
        }
    }
}
