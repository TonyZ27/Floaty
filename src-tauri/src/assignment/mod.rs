pub mod state;

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
    SimilarWindowOpened,
    PermissionRevoked,
}
