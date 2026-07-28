use std::collections::HashMap;
use std::ffi::c_void;
use std::sync::Mutex;

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
const RECOVERY_REQUIRED: &str = "native window-space attachment failed; recovery required";

type MainConnectionId = unsafe extern "C" fn() -> u32;
type CopySpacesForWindows = unsafe extern "C" fn(u32, i32, *const c_void) -> *mut c_void;
type MoveWindowsToManagedSpace = unsafe extern "C" fn(u32, *const c_void, u64);
type OrderWindow = unsafe extern "C" fn(u32, u32, i32, u32) -> i32;
type GetWindowSubLevel = unsafe extern "C" fn(u32, u32) -> i32;
type SetWindowSubLevel = unsafe extern "C" fn(u32, u32, i32);

/// A receipt that verifies the Note's immediate managed-Space placement.
///
/// It does not prove that macOS will carry the Note into a later full-screen
/// Space with the target. That behavior remains part of the mandatory manual gate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowAttachment {
    pub target_window_id: u32,
    pub note_window_id: u32,
    pub target_space_id: u64,
    pub note_space_id: u64,
    pub note_space_before_attach: u64,
    pub move_was_observable: bool,
}

impl WindowAttachment {
    pub fn placement_verified(&self) -> bool {
        self.target_space_id != 0 && self.target_space_id == self.note_space_id
    }
}

/// Native window-Space operations.
///
/// Every method must be called from the macOS main thread. Calls from another
/// thread fail with `native window-space attachment unavailable`.
pub trait WindowSpaceBackend: Send + Sync {
    fn attach(&self, target_window_id: u32, note_window_id: u32) -> AppResult<WindowAttachment>;
    fn detach(&self, note_window_id: u32) -> AppResult<()>;
    fn order_out(&self, note_window_id: u32) -> AppResult<()>;
    fn order_front(&self, target_window_id: u32, note_window_id: u32) -> AppResult<()>;
    fn promote(&self, target_window_id: u32, note_window_id: u32) -> AppResult<()>;
    fn restore_ordering(&self, note_window_id: u32) -> AppResult<()>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct OriginalWindowState {
    space_id: u64,
    collection_behavior: usize,
    sublevel: i32,
}

/// Keep one backend instance alive for the lifetime of active attachments so
/// `detach` can restore each Note's pre-binding Space and collection behavior.
#[derive(Debug, Default)]
pub struct MacOSWindowSpaceBackend {
    original_states: Mutex<HashMap<u32, OriginalWindowState>>,
}

impl WindowSpaceBackend for MacOSWindowSpaceBackend {
    fn attach(&self, target_window_id: u32, note_window_id: u32) -> AppResult<WindowAttachment> {
        // SAFETY: this checks a scalar AppKit thread property before native work.
        unsafe { require_main_thread()? };
        unavailable_if_zero(target_window_id)?;
        unavailable_if_zero(note_window_id)?;

        // SAFETY: SkyLight is loaded at runtime and every symbol is checked before use.
        // Objective-C and Core Foundation pointers remain contained in this module.
        unsafe {
            let sky_light = SkyLight::load()?;
            let connection = sky_light.connection()?;
            let target_space_id =
                sky_light.single_space_for_window(connection, target_window_id)?;
            let note_space_before_attach =
                sky_light.single_space_for_window(connection, note_window_id)?;
            let original_sublevel = sky_light.window_sublevel(connection, note_window_id);

            let saved_origin = self.original_state(note_window_id);
            let had_existing_token = saved_origin.is_some();
            let original_collection_behavior = set_note_collection_behavior(note_window_id, true)?;
            let rollback_state = OriginalWindowState {
                space_id: note_space_before_attach,
                collection_behavior: original_collection_behavior,
                sublevel: original_sublevel,
            };
            let binding_origin = saved_origin.unwrap_or(rollback_state);
            // Save recovery state before the first private Space mutation.
            self.remember_original_state(note_window_id, binding_origin);

            let result = (|| {
                sky_light.move_window(connection, note_window_id, target_space_id)?;
                sky_light.order_window(
                    connection,
                    note_window_id,
                    ORDER_ABOVE,
                    target_window_id,
                )?;

                let target_space_id =
                    sky_light.single_space_for_window(connection, target_window_id)?;
                let note_space_id =
                    sky_light.single_space_for_window(connection, note_window_id)?;
                let attachment = WindowAttachment {
                    target_window_id,
                    note_window_id,
                    target_space_id,
                    note_space_id,
                    note_space_before_attach,
                    move_was_observable: note_space_before_attach != note_space_id,
                };

                if !attachment.placement_verified() {
                    return Err(unavailable());
                }

                Ok(attachment)
            })();

            match result {
                Ok(attachment) => Ok(attachment),
                Err(forward_error) => {
                    let rollback_succeeded = sky_light
                        .restore_complete_window_state(connection, note_window_id, rollback_state)
                        .is_ok();
                    if !should_keep_recovery_token(had_existing_token, rollback_succeeded) {
                        self.forget_original_state(note_window_id);
                    }

                    if rollback_succeeded {
                        Err(forward_error)
                    } else {
                        Err(recovery_required())
                    }
                }
            }
        }
    }

    fn detach(&self, note_window_id: u32) -> AppResult<()> {
        // SAFETY: this checks a scalar AppKit thread property before native work.
        unsafe { require_main_thread()? };
        unavailable_if_zero(note_window_id)?;

        // SAFETY: native pointers remain inside this module, and Space restoration
        // is verified before the runtime restoration token is removed.
        unsafe {
            let original_state = self
                .original_state(note_window_id)
                .ok_or_else(unavailable)?;
            let sky_light = SkyLight::load()?;
            let connection = sky_light.connection()?;
            sky_light.restore_complete_window_state(connection, note_window_id, original_state)?;
            self.forget_original_state(note_window_id);
            Ok(())
        }
    }

    fn order_out(&self, note_window_id: u32) -> AppResult<()> {
        // SAFETY: this checks a scalar AppKit thread property before native work.
        unsafe { require_main_thread()? };
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
        // SAFETY: this checks a scalar AppKit thread property before native work.
        unsafe { require_main_thread()? };
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

    fn promote(&self, target_window_id: u32, note_window_id: u32) -> AppResult<()> {
        // SAFETY: this checks a scalar AppKit thread property before native work.
        unsafe { require_main_thread()? };
        unavailable_if_zero(target_window_id)?;
        unavailable_if_zero(note_window_id)?;

        let original_state = self
            .original_state(note_window_id)
            .ok_or_else(unavailable)?;
        // SAFETY: dynamically resolved function pointers are invoked while their
        // owning library is alive, and sublevel changes are verified by readback.
        unsafe {
            let sky_light = SkyLight::load()?;
            let connection = sky_light.connection()?;
            let promoted_sublevel =
                promoted_sublevel(sky_light.window_sublevel(connection, target_window_id))?;
            if let Err(error) = sky_light.set_window_sublevel_verified(
                connection,
                note_window_id,
                promoted_sublevel,
            ) {
                let _ = sky_light.set_window_sublevel_verified(
                    connection,
                    note_window_id,
                    original_state.sublevel,
                );
                return Err(error);
            }
            if let Err(error) =
                sky_light.order_window(connection, note_window_id, ORDER_ABOVE, target_window_id)
            {
                let _ = sky_light.set_window_sublevel_verified(
                    connection,
                    note_window_id,
                    original_state.sublevel,
                );
                return Err(error);
            }
            Ok(())
        }
    }

    fn restore_ordering(&self, note_window_id: u32) -> AppResult<()> {
        // SAFETY: this checks a scalar AppKit thread property before native work.
        unsafe { require_main_thread()? };
        unavailable_if_zero(note_window_id)?;
        let Some(original_state) = self.original_state(note_window_id) else {
            return Ok(());
        };

        // SAFETY: dynamically resolved function pointers are invoked while their
        // owning library is alive, and the change is verified by readback.
        unsafe {
            let sky_light = SkyLight::load()?;
            let connection = sky_light.connection()?;
            sky_light.set_window_sublevel_verified(
                connection,
                note_window_id,
                original_state.sublevel,
            )
        }
    }
}

impl MacOSWindowSpaceBackend {
    fn original_state(&self, note_window_id: u32) -> Option<OriginalWindowState> {
        self.original_states
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(&note_window_id)
            .copied()
    }

    fn remember_original_state(&self, note_window_id: u32, state: OriginalWindowState) {
        self.original_states
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .entry(note_window_id)
            .or_insert(state);
    }

    fn forget_original_state(&self, note_window_id: u32) {
        self.original_states
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(&note_window_id);
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
    get_window_sublevel: GetWindowSubLevel,
    set_window_sublevel: SetWindowSubLevel,
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
        let get_window_sublevel = {
            let symbol = unsafe { library.get::<GetWindowSubLevel>(b"SLSGetWindowSubLevel\0") }
                .map_err(|_| unavailable())?;
            *symbol
        };
        let set_window_sublevel = {
            let symbol = unsafe { library.get::<SetWindowSubLevel>(b"SLSSetWindowSubLevel\0") }
                .map_err(|_| unavailable())?;
            *symbol
        };

        Ok(Self {
            _library: library,
            main_connection_id,
            copy_spaces_for_windows,
            move_windows_to_managed_space,
            order_window,
            get_window_sublevel,
            set_window_sublevel,
        })
    }

    unsafe fn connection(&self) -> AppResult<u32> {
        // SAFETY: function pointer was resolved from SkyLight with the matching ABI.
        let connection = unsafe { (self.main_connection_id)() };
        unavailable_if_zero(connection)?;
        Ok(connection)
    }

    unsafe fn spaces_for_window(&self, connection: u32, window_id: u32) -> AppResult<Vec<u64>> {
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
        let mut space_ids = Vec::with_capacity(count);
        for index in 0..count {
            let number: *mut AnyObject = unsafe { msg_send![spaces, objectAtIndex: index] };
            let space_id = if number.is_null() {
                0
            } else {
                unsafe { msg_send![number, unsignedLongLongValue] }
            };
            space_ids.push(space_id);
        }
        let _: () = unsafe { msg_send![spaces, release] };

        if space_ids.is_empty() || space_ids.contains(&0) {
            return Err(unavailable());
        }
        Ok(space_ids)
    }

    unsafe fn single_space_for_window(&self, connection: u32, window_id: u32) -> AppResult<u64> {
        let spaces = unsafe { self.spaces_for_window(connection, window_id)? };
        only_space(&spaces)
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
        // SAFETY: this SkyLight function has a void ABI. A silent failure is detected
        // by `attach` reading the Note's managed Space back after this call.
        unsafe {
            (self.move_windows_to_managed_space)(connection, windows.cast::<c_void>(), space_id)
        };
        Ok(())
    }

    unsafe fn restore_window_state(
        &self,
        connection: u32,
        window_id: u32,
        state: OriginalWindowState,
    ) -> AppResult<()> {
        let placement_result = match unsafe {
            self.move_window(connection, window_id, state.space_id)
        } {
            Ok(()) => {
                let restored_space = unsafe { self.single_space_for_window(connection, window_id) };
                match restored_space {
                    Ok(space_id) if space_id == state.space_id => Ok(()),
                    _ => Err(unavailable()),
                }
            }
            Err(error) => Err(error),
        };

        // Restore AppKit behavior even when the private Space move silently fails.
        let behavior_result =
            unsafe { restore_note_collection_behavior(window_id, state.collection_behavior) };
        placement_result?;
        behavior_result
    }

    unsafe fn restore_complete_window_state(
        &self,
        connection: u32,
        window_id: u32,
        state: OriginalWindowState,
    ) -> AppResult<()> {
        let ordering_result =
            unsafe { self.set_window_sublevel_verified(connection, window_id, state.sublevel) };
        let window_result = unsafe { self.restore_window_state(connection, window_id, state) };
        ordering_result?;
        window_result
    }

    unsafe fn window_sublevel(&self, connection: u32, window_id: u32) -> i32 {
        unsafe { (self.get_window_sublevel)(connection, window_id) }
    }

    unsafe fn set_window_sublevel_verified(
        &self,
        connection: u32,
        window_id: u32,
        sublevel: i32,
    ) -> AppResult<()> {
        unsafe { (self.set_window_sublevel)(connection, window_id, sublevel) };
        if unsafe { self.window_sublevel(connection, window_id) } == sublevel {
            Ok(())
        } else {
            Err(unavailable())
        }
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

unsafe fn set_note_collection_behavior(window_id: u32, attached: bool) -> AppResult<usize> {
    unsafe {
        update_note_collection_behavior(window_id, |behavior| {
            if attached {
                (behavior & !COLLECTION_BEHAVIOR_CAN_JOIN_ALL_SPACES)
                    | COLLECTION_BEHAVIOR_FULL_SCREEN_AUXILIARY
            } else {
                behavior & !COLLECTION_BEHAVIOR_FULL_SCREEN_AUXILIARY
            }
        })
    }
}

unsafe fn restore_note_collection_behavior(
    window_id: u32,
    collection_behavior: usize,
) -> AppResult<()> {
    unsafe { update_note_collection_behavior(window_id, |_| collection_behavior).map(|_| ()) }
}

unsafe fn update_note_collection_behavior<F>(window_id: u32, update: F) -> AppResult<usize>
where
    F: FnOnce(usize) -> usize,
{
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

        let previous_behavior: usize = unsafe { msg_send![window, collectionBehavior] };
        let next_behavior = update(previous_behavior);
        let _: () = unsafe { msg_send![window, setCollectionBehavior: next_behavior] };
        return Ok(previous_behavior);
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

fn only_space(space_ids: &[u64]) -> AppResult<u64> {
    match space_ids {
        [space_id] if *space_id != 0 => Ok(*space_id),
        _ => Err(unavailable()),
    }
}

fn should_keep_recovery_token(had_existing_token: bool, rollback_succeeded: bool) -> bool {
    had_existing_token || !rollback_succeeded
}

fn success_or_unavailable(code: i32) -> AppResult<()> {
    if code == 0 {
        Ok(())
    } else {
        Err(unavailable())
    }
}

fn promoted_sublevel(target_sublevel: i32) -> AppResult<i32> {
    target_sublevel.checked_add(1).ok_or_else(unavailable)
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

fn recovery_required() -> AppError {
    AppError::Other(RECOVERY_REQUIRED.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn placement(
        target_space_id: u64,
        note_space_id: u64,
        note_space_before_attach: u64,
    ) -> WindowAttachment {
        WindowAttachment {
            target_window_id: 10,
            note_window_id: 20,
            target_space_id,
            note_space_id,
            note_space_before_attach,
            move_was_observable: note_space_before_attach != note_space_id,
        }
    }

    #[test]
    fn placement_requires_the_same_nonzero_space() {
        assert!(placement(30, 30, 20).placement_verified());
        assert!(!placement(30, 31, 20).placement_verified());
        assert!(!placement(0, 0, 20).placement_verified());
    }

    #[test]
    fn preexisting_same_space_is_not_reported_as_an_observable_move() {
        let receipt = placement(30, 30, 30);
        assert!(receipt.placement_verified());
        assert!(!receipt.move_was_observable);
    }

    #[test]
    fn multi_space_windows_are_rejected() {
        assert_eq!(only_space(&[30]).unwrap(), 30);
        assert!(only_space(&[]).is_err());
        assert!(only_space(&[0]).is_err());
        assert!(only_space(&[30, 31]).is_err());
    }

    #[test]
    fn runtime_restore_token_preserves_the_first_bind_state() {
        let backend = MacOSWindowSpaceBackend::default();
        let first = OriginalWindowState {
            space_id: 30,
            collection_behavior: 10,
            sublevel: 0,
        };
        let rebind = OriginalWindowState {
            space_id: 40,
            collection_behavior: 20,
            sublevel: 1,
        };

        backend.remember_original_state(20, first);
        backend.remember_original_state(20, rebind);
        assert_eq!(backend.original_state(20), Some(first));

        backend.forget_original_state(20);
        assert_eq!(backend.original_state(20), None);
    }

    #[test]
    fn rollback_failure_preserves_a_recovery_token() {
        assert!(should_keep_recovery_token(false, false));
        assert!(should_keep_recovery_token(true, false));
        assert!(should_keep_recovery_token(true, true));
        assert!(!should_keep_recovery_token(false, true));
        assert_eq!(
            recovery_required().to_string(),
            "native window-space attachment failed; recovery required"
        );
    }

    #[test]
    fn sky_light_symbols_are_resolved_at_runtime() {
        // SAFETY: the test only opens the system framework and resolves the four
        // function pointers; it does not invoke a private API.
        assert!(unsafe { SkyLight::load() }.is_ok());
    }

    #[test]
    fn silent_move_noop_fails_the_readback_contract() {
        assert!(!placement(30, 40, 40).placement_verified());
    }

    #[test]
    fn promotion_is_exactly_one_sublevel_above_target() {
        assert_eq!(promoted_sublevel(0).unwrap(), 1);
        assert_eq!(promoted_sublevel(-1).unwrap(), 0);
        assert!(promoted_sublevel(i32::MAX).is_err());
    }
}
