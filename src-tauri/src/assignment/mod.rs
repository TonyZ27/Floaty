pub mod coordinator;
pub mod state;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WindowDescriptor {
    pub app_bundle_id: String,
    pub app_name: String,
    pub app_icon_data_url: Option<String>,
    pub pid: i32,
    pub window_id: u32,
    pub window_title: String,
}

pub trait WindowCatalog: Send + Sync {
    fn list_eligible(&self) -> crate::error::AppResult<Vec<WindowDescriptor>>;
    fn contains(&self, pid: i32, window_id: u32) -> crate::error::AppResult<bool>;
}

#[cfg(not(target_os = "macos"))]
#[derive(Debug, Default)]
pub struct UnsupportedWindowCatalog;

#[cfg(not(target_os = "macos"))]
impl WindowCatalog for UnsupportedWindowCatalog {
    fn list_eligible(&self) -> crate::error::AppResult<Vec<WindowDescriptor>> {
        Err(crate::error::AppError::Other(
            "window binding is available on macOS only".into(),
        ))
    }

    fn contains(&self, _pid: i32, _window_id: u32) -> crate::error::AppResult<bool> {
        Err(crate::error::AppError::Other(
            "window binding is available on macOS only".into(),
        ))
    }
}

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize, PartialEq, Eq, sqlx::Type)]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "TEXT", rename_all = "snake_case")]
pub enum BindingState {
    Active,
    TargetClosed,
    ReconnectRequired,
    Unavailable,
}

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeState {
    Visible,
    Suspended,
    ManuallyHidden,
    SuspendedManuallyHidden,
    TargetClosed,
    ReconnectRequired,
    Unavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BindingEvent {
    UserHidNote,
    UserOpenedNote,
    TargetMinimized,
    TargetRestored,
    TargetClosed,
    TargetSpaceChanged,
    SimilarWindowOpened,
    PermissionRevoked,
}
