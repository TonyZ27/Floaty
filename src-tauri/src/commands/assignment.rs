use std::sync::Arc;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State, WebviewUrl, WebviewWindowBuilder};

use crate::assignment::coordinator::{BindingCoordinator, MainThreadWindowCatalog};
use crate::assignment::{BindingState, WindowDescriptor};
use crate::db::window_bindings::WindowBinding;
use crate::db::Db;
use crate::error::{AppError, AppResult};

const TARGET_UNAVAILABLE: &str = "selected target window is unavailable";
const PICKER_WIDTH: f64 = 720.0;
const PICKER_HEIGHT: f64 = 480.0;

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AccessibilityStatusView {
    Granted,
    Denied,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WindowBindingView {
    pub sticky_id: String,
    pub app_bundle_id: String,
    pub app_name: String,
    /// Resolved live from the current exact window; unavailable bindings use null.
    pub app_icon_data_url: Option<String>,
    pub window_title: String,
    pub state: BindingState,
    pub manually_hidden: bool,
}

#[tauri::command]
pub fn get_accessibility_status() -> AccessibilityStatusView {
    #[cfg(target_os = "macos")]
    {
        return accessibility_status_view(crate::platform::macos::accessibility::status());
    }
    #[cfg(not(target_os = "macos"))]
    AccessibilityStatusView::Denied
}

#[tauri::command]
pub async fn request_accessibility_permission(
    app: AppHandle,
) -> AppResult<AccessibilityStatusView> {
    #[cfg(target_os = "macos")]
    {
        return run_on_main_thread(&app, || {
            Ok(accessibility_status_view(
                crate::platform::macos::accessibility::request_with_system_prompt(),
            ))
        })
        .await;
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = app;
        Err(AppError::Other(
            "window binding is available on macOS only".into(),
        ))
    }
}

#[tauri::command]
pub async fn open_accessibility_settings(app: AppHandle) -> AppResult<()> {
    #[cfg(target_os = "macos")]
    {
        return run_on_main_thread(
            &app,
            crate::platform::macos::accessibility::open_system_settings,
        )
        .await;
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = app;
        Err(AppError::Other(
            "window binding is available on macOS only".into(),
        ))
    }
}

#[tauri::command]
pub async fn list_eligible_windows(
    catalog: State<'_, Arc<MainThreadWindowCatalog>>,
) -> AppResult<Vec<WindowDescriptor>> {
    catalog.list_eligible().await
}

#[tauri::command]
pub async fn get_window_binding(
    db: State<'_, Db>,
    catalog: State<'_, Arc<MainThreadWindowCatalog>>,
    sticky_id: String,
) -> AppResult<Option<WindowBindingView>> {
    let Some(binding) = crate::db::window_bindings::get(&db, &sticky_id).await? else {
        return Ok(None);
    };
    let descriptors = catalog.list_eligible().await.unwrap_or_default();
    Ok(Some(binding_view(&db, binding, &descriptors).await?))
}

#[tauri::command]
pub async fn list_window_bindings(
    db: State<'_, Db>,
    catalog: State<'_, Arc<MainThreadWindowCatalog>>,
) -> AppResult<Vec<WindowBindingView>> {
    let bindings = crate::db::window_bindings::list(&db).await?;
    let descriptors = catalog.list_eligible().await.unwrap_or_default();
    let mut views = Vec::with_capacity(bindings.len());
    for binding in bindings {
        views.push(binding_view(&db, binding, &descriptors).await?);
    }
    Ok(views)
}

#[tauri::command]
pub async fn bind_note_to_window(
    coordinator: State<'_, BindingCoordinator>,
    catalog: State<'_, Arc<MainThreadWindowCatalog>>,
    db: State<'_, Db>,
    sticky_id: String,
    target: WindowDescriptor,
) -> AppResult<WindowBindingView> {
    let current = catalog.list_eligible().await?;
    let target = resolve_exact_descriptor(&current, &target)?;
    let binding = coordinator.bind(&sticky_id, target.clone()).await?;
    binding_view(&db, binding, &[target]).await
}

#[tauri::command]
pub async fn unbind_note_from_window(
    coordinator: State<'_, BindingCoordinator>,
    sticky_id: String,
) -> AppResult<()> {
    coordinator.unbind(&sticky_id).await
}

#[tauri::command]
pub async fn reopen_bound_note(
    app: AppHandle,
    coordinator: State<'_, BindingCoordinator>,
    catalog: State<'_, Arc<MainThreadWindowCatalog>>,
    db: State<'_, Db>,
    sticky_id: String,
) -> AppResult<WindowBindingView> {
    let binding = coordinator.reopen(&sticky_id).await?;
    let descriptors = catalog.list_eligible().await.unwrap_or_default();
    let view = binding_view(&db, binding, &descriptors).await?;
    emit_binding_changed(&app, &sticky_id);
    crate::tray::refresh_menu(&app).await;
    Ok(view)
}

#[tauri::command]
pub async fn open_window_picker(
    app: AppHandle,
    db: State<'_, Db>,
    sticky_id: String,
) -> AppResult<()> {
    let (label, route) = picker_window_parts(&sticky_id)?;
    if crate::db::stickies::get(&db, &sticky_id)
        .await?
        .deleted_at
        .is_some()
    {
        return Err(AppError::NotFound);
    }
    if let Some(window) = app.get_webview_window(&label) {
        window
            .show()
            .map_err(|error| AppError::Other(format!("show window picker: {error}")))?;
        let _ = window.set_focus();
        return Ok(());
    }
    WebviewWindowBuilder::new(&app, &label, WebviewUrl::App(route.into()))
        .title("绑定到窗口")
        .inner_size(PICKER_WIDTH, PICKER_HEIGHT)
        .min_inner_size(PICKER_WIDTH, PICKER_HEIGHT)
        .max_inner_size(PICKER_WIDTH, PICKER_HEIGHT)
        .resizable(false)
        .decorations(true)
        .transparent(false)
        .visible(true)
        .build()
        .map_err(|error| AppError::Other(format!("window picker build: {error}")))?;
    Ok(())
}

fn resolve_exact_descriptor(
    current: &[WindowDescriptor],
    selected: &WindowDescriptor,
) -> AppResult<WindowDescriptor> {
    current
        .iter()
        .find(|candidate| {
            candidate.pid == selected.pid && candidate.window_id == selected.window_id
        })
        .cloned()
        .ok_or_else(|| AppError::Other(TARGET_UNAVAILABLE.into()))
}

async fn binding_view(
    db: &Db,
    binding: WindowBinding,
    descriptors: &[WindowDescriptor],
) -> AppResult<WindowBindingView> {
    let runtime_identity = binding
        .runtime_pid
        .and_then(|pid| i32::try_from(pid).ok())
        .zip(
            binding
                .runtime_window_id
                .and_then(|window_id| u32::try_from(window_id).ok()),
        );
    let app_icon_data_url = runtime_identity
        .and_then(|(pid, window_id)| {
            descriptors
                .iter()
                .find(|descriptor| descriptor.pid == pid && descriptor.window_id == window_id)
        })
        .and_then(|descriptor| descriptor.app_icon_data_url.clone());
    let manually_hidden = crate::db::stickies::get(db, &binding.sticky_id)
        .await?
        .hidden
        == 1;
    Ok(WindowBindingView {
        sticky_id: binding.sticky_id,
        app_bundle_id: binding.app_bundle_id,
        app_name: binding.app_name,
        app_icon_data_url,
        window_title: binding.window_title,
        state: binding.state,
        manually_hidden,
    })
}

fn picker_window_parts(sticky_id: &str) -> AppResult<(String, String)> {
    let parsed = ulid::Ulid::from_string(sticky_id)
        .map_err(|_| AppError::Other("invalid sticky id".into()))?;
    if parsed.to_string() != sticky_id {
        return Err(AppError::Other("invalid sticky id".into()));
    }
    Ok((
        format!("window-picker-{sticky_id}"),
        format!("index.html#/window-picker/{sticky_id}"),
    ))
}

fn emit_binding_changed(app: &AppHandle, sticky_id: &str) {
    if let Err(error) = app.emit("binding-changed", sticky_id) {
        eprintln!(
            "[floaty] emit binding-changed failed for {}: {}",
            sticky_id, error
        );
    }
}

#[cfg(target_os = "macos")]
fn accessibility_status_view(
    status: crate::platform::macos::accessibility::AccessibilityStatus,
) -> AccessibilityStatusView {
    match status {
        crate::platform::macos::accessibility::AccessibilityStatus::Granted => {
            AccessibilityStatusView::Granted
        }
        crate::platform::macos::accessibility::AccessibilityStatus::Denied => {
            AccessibilityStatusView::Denied
        }
    }
}

async fn run_on_main_thread<T, F>(app: &AppHandle, operation: F) -> AppResult<T>
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

#[cfg(test)]
mod tests {
    use super::*;

    fn descriptor(pid: i32, window_id: u32, app_name: &str) -> WindowDescriptor {
        WindowDescriptor {
            app_bundle_id: format!("com.example.{app_name}"),
            app_name: app_name.into(),
            app_icon_data_url: Some("data:image/png;base64,icon".into()),
            pid,
            window_id,
            window_title: "Document".into(),
        }
    }

    #[test]
    fn command_revalidation_uses_exact_pid_and_window_id_and_canonical_metadata() {
        let current = vec![descriptor(42, 101, "Canonical")];
        let mut selected = descriptor(42, 101, "Spoofed");
        selected.window_title = "Stale title".into();

        let resolved = resolve_exact_descriptor(&current, &selected).unwrap();

        assert_eq!(resolved.app_name, "Canonical");
        assert_eq!(resolved.window_title, "Document");
        assert!(resolve_exact_descriptor(&current, &descriptor(43, 101, "Other")).is_err());
        assert!(resolve_exact_descriptor(&current, &descriptor(42, 102, "Other")).is_err());
    }

    #[test]
    fn picker_label_and_route_accept_only_canonical_ulids() {
        let sticky_id = ulid::Ulid::new().to_string();
        assert_eq!(
            picker_window_parts(&sticky_id).unwrap(),
            (
                format!("window-picker-{sticky_id}"),
                format!("index.html#/window-picker/{sticky_id}")
            )
        );
        for invalid in [
            "../sticky",
            "01ARZ3NDEKTSV4RRFFQ69G5FAV/index",
            "window-picker-x",
            "01arz3ndektsv4rrffq69g5fav",
        ] {
            assert!(picker_window_parts(invalid).is_err());
        }
    }

    #[test]
    fn binding_view_serializes_camel_case_and_actual_state_value() {
        let view = WindowBindingView {
            sticky_id: "sticky".into(),
            app_bundle_id: "com.example.Target".into(),
            app_name: "Target".into(),
            app_icon_data_url: None,
            window_title: "Document".into(),
            state: BindingState::ReconnectRequired,
            manually_hidden: true,
        };

        let value = serde_json::to_value(view).unwrap();

        assert_eq!(value["stickyId"], "sticky");
        assert_eq!(value["appBundleId"], "com.example.Target");
        assert_eq!(value["appIconDataUrl"], serde_json::Value::Null);
        assert_eq!(value["state"], "reconnect_required");
        assert_eq!(value["manuallyHidden"], true);
        assert!(value.get("sticky_id").is_none());
    }
}
