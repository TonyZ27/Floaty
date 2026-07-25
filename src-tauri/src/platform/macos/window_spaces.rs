use std::ffi::c_void;

use libloading::Library;
use objc2::runtime::AnyObject;
use objc2::{class, msg_send};

use crate::error::{AppError, AppResult};

const SKYLIGHT_PATH: &str = "/System/Library/PrivateFrameworks/SkyLight.framework/SkyLight";
const ALL_SPACES_SELECTOR: i32 = 0x7;
const ORDER_OUT: i32 = 0;
const ORDER_ABOVE: i32 = 1;
const COLLECTION_BEHAVIOR_CAN_JOIN_ALL_SPACES: usize = 1 << 0;
const COLLECTION_BEHAVIOR_FULL_SCREEN_AUXILIARY: usize = 1 << 8;
const UNAVAILABLE: &str = "native window-space attachment unavailable";

type MainConnectionId = unsafe extern "C" fn() -> u32;
type CopySpacesForWindows = unsafe extern "C" fn(u32, i32, *const c_void) -> *mut c_void;
type MoveWindowsToManagedSpace = unsafe extern "C" fn(u32, *const c_void, u64) -> i32;
type OrderWindow = unsafe extern "C" fn(u32, u32, i32, u32) -> i32;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowAttachment {
    pub target_window_id: u32,
    pub note_window_id: u32,
    pub target_space_id: u64,
    pub note_space_id: u64,
}

impl WindowAttachment {
    pub fn is_same_space(&self) -> bool {
        self.target_space_id != 0 && self.target_space_id == self.note_space_id
    }
}

pub trait WindowSpaceBackend: Send + Sync {
    fn attach(&self, target_window_id: u32, note_window_id: u32) -> AppResult<WindowAttachment>;
    fn detach(&self, note_window_id: u32) -> AppResult<()>;
    fn order_out(&self, note_window_id: u32) -> AppResult<()>;
    fn order_front(&self, target_window_id: u32, note_window_id: u32) -> AppResult<()>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct MacOSWindowSpaceBackend;

impl WindowSpaceBackend for MacOSWindowSpaceBackend {
    fn attach(&self, target_window_id: u32, note_window_id: u32) -> AppResult<WindowAttachment> {
        unavailable_if_zero(target_window_id)?;
        unavailable_if_zero(note_window_id)?;

        // SAFETY: SkyLight is loaded at runtime and every symbol is checked before use.
        // Objective-C and Core Foundation pointers remain contained in this module.
        unsafe {
            let sky_light = SkyLight::load()?;
            let connection = sky_light.connection()?;
            let target_space_id = sky_light.space_for_window(connection, target_window_id)?;
            sky_light.space_for_window(connection, note_window_id)?;

            set_note_collection_behavior(note_window_id, true)?;
            sky_light.move_window(connection, note_window_id, target_space_id)?;
            sky_light.order_window(connection, note_window_id, ORDER_ABOVE, target_window_id)?;

            let target_space_id = sky_light.space_for_window(connection, target_window_id)?;
            let note_space_id = sky_light.space_for_window(connection, note_window_id)?;
            let attachment = WindowAttachment {
                target_window_id,
                note_window_id,
                target_space_id,
                note_space_id,
            };

            if !attachment.is_same_space() {
                return Err(unavailable());
            }

            Ok(attachment)
        }
    }

    fn detach(&self, note_window_id: u32) -> AppResult<()> {
        unavailable_if_zero(note_window_id)?;

        // SAFETY: the note window is resolved from NSApplication's live windows and
        // no Objective-C pointer leaves this module.
        unsafe { set_note_collection_behavior(note_window_id, false) }
    }

    fn order_out(&self, note_window_id: u32) -> AppResult<()> {
        unavailable_if_zero(note_window_id)?;

        // SAFETY: dynamically resolved function pointers are invoked while their
        // owning library is alive.
        unsafe {
            let sky_light = SkyLight::load()?;
            let connection = sky_light.connection()?;
            sky_light.order_window(connection, note_window_id, ORDER_OUT, 0)
        }
    }

    fn order_front(&self, target_window_id: u32, note_window_id: u32) -> AppResult<()> {
        unavailable_if_zero(target_window_id)?;
        unavailable_if_zero(note_window_id)?;

        // SAFETY: dynamically resolved function pointers are invoked while their
        // owning library is alive.
        unsafe {
            let sky_light = SkyLight::load()?;
            let connection = sky_light.connection()?;
            sky_light.order_window(connection, note_window_id, ORDER_ABOVE, target_window_id)
        }
    }
}

pub fn note_window_id(window: &tauri::WebviewWindow) -> AppResult<u32> {
    // SAFETY: this only asks AppKit for a scalar process-wide thread property.
    unsafe { require_main_thread()? };

    let ns_window = window
        .ns_window()
        .map_err(|_| unavailable())?
        .cast::<AnyObject>();
    if ns_window.is_null() {
        return Err(unavailable());
    }

    // SAFETY: Tauri owns this live NSWindow. We read a scalar property and do not
    // retain or expose the Objective-C pointer.
    let window_number: isize = unsafe { msg_send![ns_window, windowNumber] };
    u32::try_from(window_number)
        .ok()
        .filter(|id| *id != 0)
        .ok_or_else(unavailable)
}

struct SkyLight {
    _library: Library,
    main_connection_id: MainConnectionId,
    copy_spaces_for_windows: CopySpacesForWindows,
    move_windows_to_managed_space: MoveWindowsToManagedSpace,
    order_window: OrderWindow,
}

impl SkyLight {
    unsafe fn load() -> AppResult<Self> {
        // SAFETY: loading is restricted to the known system framework path. Symbols
        // are copied as function pointers and only used while `_library` is alive.
        let library = unsafe { Library::new(SKYLIGHT_PATH) }.map_err(|_| unavailable())?;
        let main_connection_id = {
            let symbol = unsafe { library.get::<MainConnectionId>(b"SLSMainConnectionID\0") }
                .map_err(|_| unavailable())?;
            *symbol
        };
        let copy_spaces_for_windows = {
            let symbol =
                unsafe { library.get::<CopySpacesForWindows>(b"SLSCopySpacesForWindows\0") }
                    .map_err(|_| unavailable())?;
            *symbol
        };
        let move_windows_to_managed_space = {
            let symbol = unsafe {
                library.get::<MoveWindowsToManagedSpace>(b"SLSMoveWindowsToManagedSpace\0")
            }
            .map_err(|_| unavailable())?;
            *symbol
        };
        let order_window = {
            let symbol = unsafe { library.get::<OrderWindow>(b"SLSOrderWindow\0") }
                .map_err(|_| unavailable())?;
            *symbol
        };

        Ok(Self {
            _library: library,
            main_connection_id,
            copy_spaces_for_windows,
            move_windows_to_managed_space,
            order_window,
        })
    }

    unsafe fn connection(&self) -> AppResult<u32> {
        // SAFETY: function pointer was resolved from SkyLight with the matching ABI.
        let connection = unsafe { (self.main_connection_id)() };
        unavailable_if_zero(connection)?;
        Ok(connection)
    }

    unsafe fn space_for_window(&self, connection: u32, window_id: u32) -> AppResult<u64> {
        // SAFETY: the autoreleased NSNumber/NSArray are used only for this call.
        let window_number: *mut AnyObject =
            unsafe { msg_send![class!(NSNumber), numberWithUnsignedInt: window_id] };
        if window_number.is_null() {
            return Err(unavailable());
        }
        let windows: *mut AnyObject =
            unsafe { msg_send![class!(NSArray), arrayWithObject: window_number] };
        if windows.is_null() {
            return Err(unavailable());
        }

        // SAFETY: function pointer was resolved from SkyLight. Copy naming gives us
        // ownership of the returned CFArray, released below on every non-null path.
        let spaces = unsafe {
            (self.copy_spaces_for_windows)(
                connection,
                ALL_SPACES_SELECTOR,
                windows.cast::<c_void>(),
            )
        }
        .cast::<AnyObject>();
        if spaces.is_null() {
            return Err(unavailable());
        }

        // SAFETY: the returned CFArray is toll-free bridged to NSArray and contains
        // managed Space IDs represented as NSNumber values.
        let count: usize = unsafe { msg_send![spaces, count] };
        let space_id = if count == 0 {
            0
        } else {
            let number: *mut AnyObject = unsafe { msg_send![spaces, objectAtIndex: 0usize] };
            if number.is_null() {
                0
            } else {
                unsafe { msg_send![number, unsignedLongLongValue] }
            }
        };
        let _: () = unsafe { msg_send![spaces, release] };

        unavailable_if_zero(space_id)?;
        Ok(space_id)
    }

    unsafe fn move_window(&self, connection: u32, window_id: u32, space_id: u64) -> AppResult<()> {
        // SAFETY: the autoreleased NSNumber/NSArray remain alive for the duration of
        // the dynamically resolved SkyLight call.
        let window_number: *mut AnyObject =
            unsafe { msg_send![class!(NSNumber), numberWithUnsignedInt: window_id] };
        if window_number.is_null() {
            return Err(unavailable());
        }
        let windows: *mut AnyObject =
            unsafe { msg_send![class!(NSArray), arrayWithObject: window_number] };
        if windows.is_null() {
            return Err(unavailable());
        }
        let result = unsafe {
            (self.move_windows_to_managed_space)(connection, windows.cast::<c_void>(), space_id)
        };
        success_or_unavailable(result)
    }

    unsafe fn order_window(
        &self,
        connection: u32,
        window_id: u32,
        order: i32,
        relative_to_window_id: u32,
    ) -> AppResult<()> {
        // SAFETY: function pointer was resolved from SkyLight with the matching ABI.
        let result =
            unsafe { (self.order_window)(connection, window_id, order, relative_to_window_id) };
        success_or_unavailable(result)
    }
}

unsafe fn set_note_collection_behavior(window_id: u32, attached: bool) -> AppResult<()> {
    unsafe { require_main_thread()? };

    // SAFETY: NSApplication is already initialized by Tauri. The returned arrays and
    // windows are autoreleased/live AppKit objects used only on this call stack.
    let application: *mut AnyObject =
        unsafe { msg_send![class!(NSApplication), sharedApplication] };
    if application.is_null() {
        return Err(unavailable());
    }
    let windows: *mut AnyObject = unsafe { msg_send![application, windows] };
    if windows.is_null() {
        return Err(unavailable());
    }
    let count: usize = unsafe { msg_send![windows, count] };

    for index in 0..count {
        let window: *mut AnyObject = unsafe { msg_send![windows, objectAtIndex: index] };
        if window.is_null() {
            continue;
        }
        let candidate: isize = unsafe { msg_send![window, windowNumber] };
        if u32::try_from(candidate).ok() != Some(window_id) {
            continue;
        }

        let behavior: usize = unsafe { msg_send![window, collectionBehavior] };
        let behavior = if attached {
            (behavior & !COLLECTION_BEHAVIOR_CAN_JOIN_ALL_SPACES)
                | COLLECTION_BEHAVIOR_FULL_SCREEN_AUXILIARY
        } else {
            behavior & !COLLECTION_BEHAVIOR_FULL_SCREEN_AUXILIARY
        };
        let _: () = unsafe { msg_send![window, setCollectionBehavior: behavior] };
        return Ok(());
    }

    Err(unavailable())
}

unsafe fn require_main_thread() -> AppResult<()> {
    // SAFETY: `NSThread.isMainThread` returns a scalar and does not retain an object.
    let is_main_thread: bool = unsafe { msg_send![class!(NSThread), isMainThread] };
    if is_main_thread {
        Ok(())
    } else {
        Err(unavailable())
    }
}

fn success_or_unavailable(code: i32) -> AppResult<()> {
    if code == 0 {
        Ok(())
    } else {
        Err(unavailable())
    }
}

fn unavailable_if_zero<T>(value: T) -> AppResult<()>
where
    T: PartialEq + From<u8>,
{
    if value == T::from(0) {
        Err(unavailable())
    } else {
        Ok(())
    }
}

fn unavailable() -> AppError {
    AppError::Other(UNAVAILABLE.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attachment_only_passes_for_the_same_nonzero_space() {
        assert!(WindowAttachment {
            target_window_id: 10,
            note_window_id: 20,
            target_space_id: 30,
            note_space_id: 30,
        }
        .is_same_space());
        assert!(!WindowAttachment {
            target_window_id: 10,
            note_window_id: 20,
            target_space_id: 30,
            note_space_id: 31,
        }
        .is_same_space());
        assert!(!WindowAttachment {
            target_window_id: 10,
            note_window_id: 20,
            target_space_id: 0,
            note_space_id: 0,
        }
        .is_same_space());
    }

    #[test]
    fn sky_light_symbols_are_resolved_at_runtime() {
        // SAFETY: the test only opens the system framework and resolves the four
        // function pointers; it does not invoke a private API.
        assert!(unsafe { SkyLight::load() }.is_ok());
    }
}
