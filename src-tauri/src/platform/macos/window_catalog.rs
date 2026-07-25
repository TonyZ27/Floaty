use std::collections::HashMap;
use std::ffi::{c_char, c_void, CStr};
use std::ptr;

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use libloading::Library;
use objc2::rc::autoreleasepool;
use objc2::runtime::AnyObject;
use objc2::{class, msg_send};

use crate::assignment::{WindowCatalog, WindowDescriptor};
use crate::error::{AppError, AppResult};

use super::accessibility::{self, AccessibilityStatus};

const FLOATY_BUNDLE_ID: &str = "app.floaty.desktop";
const HISERVICES_PATH: &str = "/System/Library/Frameworks/ApplicationServices.framework/Frameworks/HIServices.framework/HIServices";
const APPLICATION_SERVICES_PATH: &str =
    "/System/Library/Frameworks/ApplicationServices.framework/ApplicationServices";
const WINDOW_DISCOVERY_UNAVAILABLE: &str = "native window discovery unavailable";
const ACCESSIBILITY_REQUIRED: &str = "Accessibility permission required";

const CG_WINDOW_LIST_OPTION_ON_SCREEN_ONLY: u32 = 1 << 0;
const CG_WINDOW_LIST_EXCLUDE_DESKTOP_ELEMENTS: u32 = 1 << 4;
const CG_NULL_WINDOW_ID: u32 = 0;

const AX_SUCCESS: i32 = 0;
const AX_MAX_WINDOWS_PER_APPLICATION: isize = 1_024;
const NS_APPLICATION_ACTIVATION_POLICY_REGULAR: isize = 0;
const NS_BITMAP_IMAGE_FILE_TYPE_PNG: usize = 4;

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
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
    fn CGWindowListCopyWindowInfo(option: u32, relative_to_window: u32) -> *mut c_void;
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFRelease(value: *const c_void);
}

type AxUiElementGetWindow = unsafe extern "C" fn(*const c_void, *mut u32) -> i32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Role {
    Normal,
    Dialog,
}

#[derive(Debug, Clone)]
struct WindowCandidate {
    descriptor: WindowDescriptor,
    role: Role,
    visible: bool,
    minimized: bool,
    system_owned: bool,
}

fn filter_eligible(windows: Vec<WindowCandidate>, floaty_bundle_id: &str) -> Vec<WindowDescriptor> {
    windows
        .into_iter()
        .filter(|window| {
            window.descriptor.app_bundle_id != floaty_bundle_id
                && window.role == Role::Normal
                && window.visible
                && !window.minimized
                && !window.system_owned
        })
        .map(|window| window.descriptor)
        .collect()
}

fn contains_identity(windows: &[WindowDescriptor], pid: i32, window_id: u32) -> bool {
    windows
        .iter()
        .any(|window| window.pid == pid && window.window_id == window_id)
}

/// Metadata-only macOS window discovery.
///
/// `list_eligible` and `contains` must run on the AppKit main thread because
/// application names and icons come from `NSRunningApplication`. Core Graphics
/// is used only for window identity, owner, visibility, z-order, and bounds.
#[derive(Debug, Clone)]
pub struct MacOSWindowCatalog {
    floaty_bundle_id: String,
}

impl Default for MacOSWindowCatalog {
    fn default() -> Self {
        Self {
            floaty_bundle_id: FLOATY_BUNDLE_ID.into(),
        }
    }
}

impl MacOSWindowCatalog {
    pub fn new(floaty_bundle_id: impl Into<String>) -> Self {
        Self {
            floaty_bundle_id: floaty_bundle_id.into(),
        }
    }
}

impl WindowCatalog for MacOSWindowCatalog {
    fn list_eligible(&self) -> AppResult<Vec<WindowDescriptor>> {
        if accessibility::status() != AccessibilityStatus::Granted {
            return Err(AppError::Other(ACCESSIBILITY_REQUIRED.into()));
        }
        require_main_thread()?;

        autoreleasepool(|_| {
            // SAFETY: all Core Foundation and Objective-C values are copied into
            // Rust-owned metadata before their scoped owners/pool are released.
            let windows = unsafe { enumerate_candidates()? };
            Ok(filter_eligible(windows, &self.floaty_bundle_id))
        })
    }

    fn contains(&self, pid: i32, window_id: u32) -> AppResult<bool> {
        if pid <= 0 || window_id == 0 {
            return Ok(false);
        }
        let windows = self.list_eligible()?;
        Ok(contains_identity(&windows, pid, window_id))
    }
}

#[derive(Debug)]
struct CgWindow {
    pid: i32,
    window_id: u32,
}

#[derive(Debug)]
struct AppMetadata {
    bundle_id: String,
    name: String,
    icon_data_url: Option<String>,
    hidden: bool,
    system_owned: bool,
}

#[derive(Debug)]
struct AxWindowMetadata {
    role: Role,
    minimized: bool,
    title: String,
}

struct WindowIdApi {
    _library: Library,
    get_window: AxUiElementGetWindow,
}

impl WindowIdApi {
    unsafe fn load() -> AppResult<Self> {
        for path in [HISERVICES_PATH, APPLICATION_SERVICES_PATH] {
            // SAFETY: both paths are fixed system framework locations.
            let Ok(library) = (unsafe { Library::new(path) }) else {
                continue;
            };
            let get_window = {
                // SAFETY: the symbol signature is the private AX window-ID
                // bridge used only for scalar CGWindowID lookup.
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
        Err(unavailable())
    }
}

struct OwnedCf(*mut c_void);

impl OwnedCf {
    fn new(value: *mut c_void) -> Option<Self> {
        (!value.is_null()).then_some(Self(value))
    }

    fn as_object(&self) -> *mut AnyObject {
        self.0.cast()
    }
}

impl Drop for OwnedCf {
    fn drop(&mut self) {
        // SAFETY: this wrapper is created only for Copy/Create-rule values and
        // releases each retained reference exactly once.
        unsafe { CFRelease(self.0.cast_const()) };
    }
}

unsafe fn enumerate_candidates() -> AppResult<Vec<WindowCandidate>> {
    let cg_windows = unsafe { copy_cg_windows()? };
    let window_id_api = unsafe { WindowIdApi::load()? };
    let mut app_cache = HashMap::<i32, Option<AppMetadata>>::new();
    let mut ax_cache = HashMap::<i32, HashMap<u32, AxWindowMetadata>>::new();
    let mut candidates = Vec::with_capacity(cg_windows.len());

    for cg_window in cg_windows {
        let app = app_cache
            .entry(cg_window.pid)
            .or_insert_with(|| unsafe { application_metadata(cg_window.pid) });
        let Some(app) = app.as_ref() else {
            continue;
        };

        let ax_windows = ax_cache.entry(cg_window.pid).or_insert_with(|| {
            unsafe { copy_ax_windows(cg_window.pid, &window_id_api) }.unwrap_or_default()
        });
        let Some(ax_window) = ax_windows.remove(&cg_window.window_id) else {
            continue;
        };

        candidates.push(WindowCandidate {
            descriptor: WindowDescriptor {
                app_bundle_id: app.bundle_id.clone(),
                app_name: app.name.clone(),
                app_icon_data_url: app.icon_data_url.clone(),
                pid: cg_window.pid,
                window_id: cg_window.window_id,
                window_title: ax_window.title,
            },
            role: ax_window.role,
            visible: !app.hidden,
            minimized: ax_window.minimized,
            system_owned: app.system_owned,
        });
    }

    Ok(candidates)
}

unsafe fn copy_cg_windows() -> AppResult<Vec<CgWindow>> {
    let options = CG_WINDOW_LIST_OPTION_ON_SCREEN_ONLY | CG_WINDOW_LIST_EXCLUDE_DESKTOP_ELEMENTS;
    // SAFETY: Core Graphics returns a retained CFArray containing metadata
    // dictionaries; OwnedCf releases the array after all values are copied.
    let list = OwnedCf::new(unsafe { CGWindowListCopyWindowInfo(options, CG_NULL_WINDOW_ID) })
        .ok_or_else(unavailable)?;
    let count: usize = unsafe { msg_send![list.as_object(), count] };

    let window_number_key = unsafe { ns_string(b"kCGWindowNumber\0") };
    let owner_pid_key = unsafe { ns_string(b"kCGWindowOwnerPID\0") };
    let layer_key = unsafe { ns_string(b"kCGWindowLayer\0") };
    let on_screen_key = unsafe { ns_string(b"kCGWindowIsOnscreen\0") };
    let bounds_key = unsafe { ns_string(b"kCGWindowBounds\0") };
    let width_key = unsafe { ns_string(b"Width\0") };
    let height_key = unsafe { ns_string(b"Height\0") };

    let mut windows = Vec::with_capacity(count);
    for index in 0..count {
        let dictionary: *mut AnyObject =
            unsafe { msg_send![list.as_object(), objectAtIndex: index] };
        if dictionary.is_null() {
            continue;
        }

        let window_id = unsafe { dictionary_u32(dictionary, window_number_key) };
        let pid = unsafe { dictionary_i32(dictionary, owner_pid_key) };
        let layer = unsafe { dictionary_i32(dictionary, layer_key) };
        let on_screen = unsafe { dictionary_bool(dictionary, on_screen_key) };
        let bounds: *mut AnyObject = unsafe { msg_send![dictionary, objectForKey: bounds_key] };
        let width = unsafe { dictionary_f64(bounds, width_key) };
        let height = unsafe { dictionary_f64(bounds, height_key) };

        if let (Some(window_id), Some(pid), Some(0), Some(true), Some(width), Some(height)) =
            (window_id, pid, layer, on_screen, width, height)
        {
            if window_id != 0 && pid > 0 && width > 0.0 && height > 0.0 {
                windows.push(CgWindow { pid, window_id });
            }
        }
    }
    Ok(windows)
}

unsafe fn application_metadata(pid: i32) -> Option<AppMetadata> {
    // SAFETY: NSRunningApplication is an autoreleased AppKit object scoped to
    // the caller's autorelease pool.
    let application: *mut AnyObject = unsafe {
        msg_send![
            class!(NSRunningApplication),
            runningApplicationWithProcessIdentifier: pid
        ]
    };
    if application.is_null() {
        return None;
    }

    let terminated: bool = unsafe { msg_send![application, isTerminated] };
    if terminated {
        return None;
    }

    let bundle_id_object: *mut AnyObject = unsafe { msg_send![application, bundleIdentifier] };
    let name_object: *mut AnyObject = unsafe { msg_send![application, localizedName] };
    let bundle_id = unsafe { ns_string_to_rust(bundle_id_object) }?;
    let name = unsafe { ns_string_to_rust(name_object) }?;
    let hidden: bool = unsafe { msg_send![application, isHidden] };
    let activation_policy: isize = unsafe { msg_send![application, activationPolicy] };
    let icon: *mut AnyObject = unsafe { msg_send![application, icon] };

    Some(AppMetadata {
        bundle_id,
        name,
        icon_data_url: unsafe { icon_png_data_url(icon) },
        hidden,
        system_owned: activation_policy != NS_APPLICATION_ACTIVATION_POLICY_REGULAR,
    })
}

unsafe fn copy_ax_windows(
    pid: i32,
    window_id_api: &WindowIdApi,
) -> AppResult<HashMap<u32, AxWindowMetadata>> {
    // SAFETY: AXUIElementCreateApplication follows the Create rule.
    let application =
        OwnedCf::new(unsafe { AXUIElementCreateApplication(pid) }).ok_or_else(unavailable)?;
    let windows_attribute = unsafe { ns_string(b"AXWindows\0") };
    let mut windows_value = ptr::null_mut();
    // SAFETY: AX writes a retained CFArray to `windows_value` on success.
    let result = unsafe {
        AXUIElementCopyAttributeValues(
            application.0.cast_const(),
            windows_attribute.cast(),
            0,
            AX_MAX_WINDOWS_PER_APPLICATION,
            &mut windows_value,
        )
    };
    if result != AX_SUCCESS {
        return Ok(HashMap::new());
    }
    let Some(windows) = OwnedCf::new(windows_value) else {
        return Ok(HashMap::new());
    };

    let count: usize = unsafe { msg_send![windows.as_object(), count] };
    let mut metadata = HashMap::with_capacity(count);
    for index in 0..count {
        let element: *mut c_void = unsafe {
            let object: *mut AnyObject = msg_send![windows.as_object(), objectAtIndex: index];
            object.cast()
        };
        if element.is_null() {
            continue;
        }

        let mut window_id = 0;
        // SAFETY: `_AXUIElementGetWindow` writes one scalar CGWindowID and does
        // not transfer ownership.
        if unsafe { (window_id_api.get_window)(element.cast_const(), &mut window_id) } != AX_SUCCESS
            || window_id == 0
        {
            continue;
        }

        let role = unsafe { copy_ax_string(element, b"AXRole\0") };
        let subrole = unsafe { copy_ax_string(element, b"AXSubrole\0") };
        let role = if role.as_deref() == Some("AXWindow")
            && subrole.as_deref() == Some("AXStandardWindow")
        {
            Role::Normal
        } else {
            Role::Dialog
        };
        let minimized = unsafe { copy_ax_bool(element, b"AXMinimized\0") }.unwrap_or(false);
        let title = unsafe { copy_ax_string(element, b"AXTitle\0") }.unwrap_or_default();

        metadata.insert(
            window_id,
            AxWindowMetadata {
                role,
                minimized,
                title,
            },
        );
    }
    Ok(metadata)
}

unsafe fn copy_ax_string(element: *const c_void, attribute: &'static [u8]) -> Option<String> {
    let value = unsafe { copy_ax_value(element, attribute) }?;
    unsafe { ns_string_to_rust(value.as_object()) }
}

unsafe fn copy_ax_bool(element: *const c_void, attribute: &'static [u8]) -> Option<bool> {
    let value = unsafe { copy_ax_value(element, attribute) }?;
    Some(unsafe { msg_send![value.as_object(), boolValue] })
}

unsafe fn copy_ax_value(element: *const c_void, attribute: &'static [u8]) -> Option<OwnedCf> {
    let attribute = unsafe { ns_string(attribute) };
    let mut value = ptr::null_mut();
    // SAFETY: AX writes a retained CF value on success.
    let result = unsafe { AXUIElementCopyAttributeValue(element, attribute.cast(), &mut value) };
    (result == AX_SUCCESS)
        .then(|| OwnedCf::new(value))
        .flatten()
}

unsafe fn icon_png_data_url(icon: *mut AnyObject) -> Option<String> {
    if icon.is_null() {
        return None;
    }
    let tiff_data: *mut AnyObject = unsafe { msg_send![icon, TIFFRepresentation] };
    if tiff_data.is_null() {
        return None;
    }

    let representation: *mut AnyObject = unsafe {
        let allocated: *mut AnyObject = msg_send![class!(NSBitmapImageRep), alloc];
        msg_send![allocated, initWithData: tiff_data]
    };
    if representation.is_null() {
        return None;
    }

    let properties: *mut AnyObject = unsafe { msg_send![class!(NSDictionary), dictionary] };
    let png_data: *mut AnyObject = unsafe {
        msg_send![
            representation,
            representationUsingType: NS_BITMAP_IMAGE_FILE_TYPE_PNG,
            properties: properties
        ]
    };
    let encoded = if png_data.is_null() {
        None
    } else {
        let length: usize = unsafe { msg_send![png_data, length] };
        let bytes: *const u8 = unsafe { msg_send![png_data, bytes] };
        if bytes.is_null() || length == 0 {
            None
        } else {
            // SAFETY: NSData owns `length` readable bytes for this autorelease
            // pool; base64 encoding completes before the pool is drained.
            let bytes = unsafe { std::slice::from_raw_parts(bytes, length) };
            Some(format!("data:image/png;base64,{}", STANDARD.encode(bytes)))
        }
    };

    // SAFETY: `representation` came from alloc/init and is released exactly once.
    unsafe {
        let _: () = msg_send![representation, release];
    }
    encoded
}

fn require_main_thread() -> AppResult<()> {
    // SAFETY: this reads a scalar process-wide AppKit thread property.
    let is_main_thread: bool = unsafe { msg_send![class!(NSThread), isMainThread] };
    if is_main_thread {
        Ok(())
    } else {
        Err(AppError::Other(
            "window discovery must run on the main thread".into(),
        ))
    }
}

fn unavailable() -> AppError {
    AppError::Other(WINDOW_DISCOVERY_UNAVAILABLE.into())
}

unsafe fn ns_string(value: &'static [u8]) -> *mut AnyObject {
    // SAFETY: callers pass static, NUL-terminated UTF-8 byte strings.
    unsafe { msg_send![class!(NSString), stringWithUTF8String: value.as_ptr()] }
}

unsafe fn ns_string_to_rust(value: *mut AnyObject) -> Option<String> {
    if value.is_null() {
        return None;
    }
    let utf8: *const c_char = unsafe { msg_send![value, UTF8String] };
    if utf8.is_null() {
        return None;
    }
    // SAFETY: NSString returns a NUL-terminated UTF-8 view valid while the
    // string is alive. We copy it before returning.
    Some(
        unsafe { CStr::from_ptr(utf8) }
            .to_string_lossy()
            .into_owned(),
    )
}

unsafe fn dictionary_value(
    dictionary: *mut AnyObject,
    key: *mut AnyObject,
) -> Option<*mut AnyObject> {
    if dictionary.is_null() || key.is_null() {
        return None;
    }
    let value: *mut AnyObject = unsafe { msg_send![dictionary, objectForKey: key] };
    (!value.is_null()).then_some(value)
}

unsafe fn dictionary_u32(dictionary: *mut AnyObject, key: *mut AnyObject) -> Option<u32> {
    let value = unsafe { dictionary_value(dictionary, key) }?;
    Some(unsafe { msg_send![value, unsignedIntValue] })
}

unsafe fn dictionary_i32(dictionary: *mut AnyObject, key: *mut AnyObject) -> Option<i32> {
    let value = unsafe { dictionary_value(dictionary, key) }?;
    Some(unsafe { msg_send![value, intValue] })
}

unsafe fn dictionary_bool(dictionary: *mut AnyObject, key: *mut AnyObject) -> Option<bool> {
    let value = unsafe { dictionary_value(dictionary, key) }?;
    Some(unsafe { msg_send![value, boolValue] })
}

unsafe fn dictionary_f64(dictionary: *mut AnyObject, key: *mut AnyObject) -> Option<f64> {
    let value = unsafe { dictionary_value(dictionary, key) }?;
    Some(unsafe { msg_send![value, doubleValue] })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(
        app_bundle_id: &str,
        app_name: &str,
        window_id: u32,
        role: Role,
        visible: bool,
        minimized: bool,
    ) -> WindowCandidate {
        WindowCandidate {
            descriptor: WindowDescriptor {
                app_bundle_id: app_bundle_id.into(),
                app_name: app_name.into(),
                app_icon_data_url: None,
                pid: window_id as i32,
                window_id,
                window_title: String::new(),
            },
            role,
            visible,
            minimized,
            system_owned: false,
        }
    }

    #[test]
    fn catalog_only_keeps_visible_normal_external_windows() {
        let windows = vec![
            candidate("app.floaty.desktop", "Floaty", 1, Role::Normal, true, false),
            candidate("com.google.Chrome", "Chrome", 2, Role::Normal, true, false),
            candidate("com.apple.finder", "Open", 3, Role::Dialog, true, false),
            candidate(
                "com.tinyspeck.slackmacgap",
                "Slack",
                4,
                Role::Normal,
                false,
                true,
            ),
        ];
        let eligible = filter_eligible(windows, "app.floaty.desktop");
        assert_eq!(
            eligible
                .iter()
                .map(|window| window.window_id)
                .collect::<Vec<_>>(),
            vec![2]
        );
    }

    #[test]
    fn catalog_preserves_front_to_back_order_and_excludes_system_apps() {
        let front = candidate("com.google.Chrome", "Chrome", 20, Role::Normal, true, false);
        let mut system = candidate("com.apple.dock", "Dock", 21, Role::Normal, true, false);
        system.system_owned = true;
        let back = candidate(
            "com.apple.TextEdit",
            "TextEdit",
            22,
            Role::Normal,
            true,
            false,
        );

        let eligible = filter_eligible(vec![front, system, back], FLOATY_BUNDLE_ID);
        assert_eq!(
            eligible
                .iter()
                .map(|window| window.window_id)
                .collect::<Vec<_>>(),
            vec![20, 22]
        );
    }

    #[test]
    fn contains_requires_exact_process_and_window_identity() {
        let windows = vec![
            candidate("com.google.Chrome", "Chrome", 42, Role::Normal, true, false).descriptor,
        ];

        assert!(contains_identity(&windows, 42, 42));
        assert!(!contains_identity(&windows, 43, 42));
        assert!(!contains_identity(&windows, 42, 43));
    }

    #[test]
    fn exact_ax_to_cg_window_bridge_is_resolved_at_runtime() {
        // SAFETY: this test only resolves the function pointer from a fixed
        // system framework; it does not call AX or inspect any window.
        assert!(unsafe { WindowIdApi::load() }.is_ok());
    }
}
