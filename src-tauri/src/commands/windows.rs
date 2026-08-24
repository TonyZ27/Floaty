use crate::db::Db;
use crate::error::{AppError, AppResult};
use crate::windows;
use tauri::{AppHandle, Emitter, Manager, State};

async fn reopen_if_bound(app: &AppHandle, db: &Db, sticky_id: &str) -> AppResult<bool> {
    if crate::db::window_bindings::get(db, sticky_id)
        .await?
        .is_none()
    {
        return Ok(false);
    }
    let coordinator = app
        .try_state::<crate::assignment::coordinator::BindingCoordinator>()
        .ok_or_else(|| AppError::Other("window binding coordinator unavailable".into()))?;
    coordinator.reopen(sticky_id).await?;
    if let Err(error) = app.emit("binding-changed", sticky_id) {
        eprintln!(
            "[floaty] emit binding-changed failed for {}: {}",
            sticky_id, error
        );
    }
    Ok(true)
}

#[tauri::command]
pub async fn open_sticky_window(
    app: AppHandle,
    db: State<'_, Db>,
    sticky_id: String,
) -> AppResult<()> {
    if !reopen_if_bound(&app, &db, &sticky_id).await? {
        let sticky = crate::db::stickies::get(&db, &sticky_id).await?;
        windows::open(&app, &sticky).await?;
    }
    crate::tray::refresh_menu(&app).await;
    Ok(())
}

#[tauri::command]
pub async fn hide_sticky(
    app: AppHandle,
    db: State<'_, Db>,
    sticky_id: String,
) -> AppResult<()> {
    windows::hide(&app, &sticky_id, &db).await?;
    if crate::db::window_bindings::get(&db, &sticky_id)
        .await?
        .is_some()
    {
        let coordinator = app
            .try_state::<crate::assignment::coordinator::BindingCoordinator>()
            .ok_or_else(|| AppError::Other("window binding coordinator unavailable".into()))?;
        coordinator
            .handle(
                &sticky_id,
                crate::assignment::BindingEvent::UserHidNote,
            )
            .await?;
        if let Err(error) = app.emit("binding-changed", &sticky_id) {
            eprintln!(
                "[floaty] emit binding-changed failed for {}: {}",
                sticky_id, error
            );
        }
    }
    crate::tray::refresh_menu(&app).await;
    Ok(())
}

#[tauri::command]
pub async fn show_sticky(
    app: AppHandle,
    db: State<'_, Db>,
    sticky_id: String,
) -> AppResult<()> {
    if !reopen_if_bound(&app, &db, &sticky_id).await? {
        windows::show(&app, &sticky_id, &db).await?;
    }
    crate::tray::refresh_menu(&app).await;
    Ok(())
}

#[tauri::command]
pub async fn toggle_pin(
    app: AppHandle,
    db: State<'_, Db>,
    sticky_id: String,
) -> AppResult<bool> {
    let pinned = windows::toggle_pin(&app, &sticky_id, &db).await?;
    crate::tray::refresh_menu(&app).await;
    Ok(pinned)
}

#[tauri::command]
pub async fn show_all_stickies(app: AppHandle, db: State<'_, Db>) -> AppResult<usize> {
    let n = windows::show_all(&app, &db).await?;
    crate::tray::refresh_menu(&app).await;
    Ok(n)
}

#[tauri::command]
pub async fn tile_all_stickies(app: AppHandle, db: State<'_, Db>) -> AppResult<usize> {
    let n = windows::tile_all(&app, &db).await?;
    crate::tray::refresh_menu(&app).await;
    Ok(n)
}

#[tauri::command]
pub async fn new_sticky_window(app: AppHandle, db: State<'_, Db>) -> AppResult<String> {
    let sticky = crate::db::stickies::create_default(&db).await?;
    windows::open(&app, &sticky).await?;
    crate::tray::refresh_menu(&app).await;
    Ok(sticky.id)
}

#[tauri::command]
pub async fn open_welcome(app: AppHandle, db: State<'_, Db>) -> AppResult<String> {
    let sticky = windows::create_welcome(&app, &db).await?;
    crate::tray::refresh_menu(&app).await;
    Ok(sticky.id)
}

#[tauri::command]
pub async fn open_preferences(app: AppHandle) -> AppResult<()> {
    use tauri::{Manager, WebviewUrl, WebviewWindowBuilder};
    if let Some(w) = app.get_webview_window("preferences") {
        let _ = w.show();
        let _ = w.set_focus();
        return Ok(());
    }
    WebviewWindowBuilder::new(&app, "preferences", WebviewUrl::App("index.html#/preferences".into()))
        .title("Floaty 偏好设置")
        .inner_size(480.0, 540.0)
        .min_inner_size(360.0, 280.0)
        .resizable(true)
        .decorations(true)
        .transparent(false)
        .visible(true)
        .build()
        .map_err(|e| crate::error::AppError::Other(format!("preferences build: {}", e)))?;
    Ok(())
}

#[tauri::command]
pub async fn open_search(app: AppHandle) -> AppResult<()> {
    use tauri::{Manager, WebviewUrl, WebviewWindowBuilder};
    if let Some(w) = app.get_webview_window("search") {
        let _ = w.show();
        let _ = w.set_focus();
        return Ok(());
    }
    WebviewWindowBuilder::new(&app, "search", WebviewUrl::App("index.html#/search".into()))
        .title("搜索 Floaty")
        .inner_size(480.0, 560.0)
        .resizable(true)
        .decorations(true)
        .transparent(false)
        .always_on_top(true)
        .visible(true)
        .build()
        .map_err(|e| crate::error::AppError::Other(format!("search window: {}", e)))?;
    Ok(())
}

#[tauri::command]
pub async fn get_stats(db: State<'_, Db>) -> AppResult<StatsPayload> {
    let sticky_count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM stickies")
        .fetch_one(db.inner())
        .await?;
    let item_count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM items")
        .fetch_one(db.inner())
        .await?;
    let reminder_count: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM reminders WHERE fired_at IS NULL")
            .fetch_one(db.inner())
            .await?;
    Ok(StatsPayload {
        stickies: sticky_count.0,
        items: item_count.0,
        pending_reminders: reminder_count.0,
    })
}

#[tauri::command]
pub async fn get_setting(db: State<'_, Db>, key: String) -> AppResult<Option<String>> {
    let row: Option<(String,)> = sqlx::query_as("SELECT value FROM settings WHERE key = ?")
        .bind(&key)
        .fetch_optional(db.inner())
        .await?;
    Ok(row.map(|r| r.0))
}

#[tauri::command]
pub async fn set_setting(
    app: AppHandle,
    db: State<'_, Db>,
    key: String,
    value: String,
) -> AppResult<()> {
    eprintln!("[floaty] set_setting: key={} value={}", key, value);
    sqlx::query(
        "INSERT INTO settings (key, value) VALUES (?, ?)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
    )
    .bind(&key)
    .bind(&value)
    .execute(db.inner())
    .await
    .map_err(|e| {
        eprintln!("[floaty] set_setting DB write failed: {}", e);
        e
    })?;
    use tauri::Emitter;
    if let Err(e) = app.emit("settings-changed", &key) {
        eprintln!("[floaty] set_setting emit failed (non-fatal): {}", e);
    }
    Ok(())
}

#[tauri::command]
pub async fn get_data_dir(app: AppHandle) -> AppResult<String> {
    use tauri::Manager;
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| crate::error::AppError::Other(format!("app_data_dir: {}", e)))?;
    Ok(dir.to_string_lossy().to_string())
}

#[derive(Debug, serde::Serialize)]
pub struct StatsPayload {
    pub stickies: i64,
    pub items: i64,
    pub pending_reminders: i64,
}

/// 用 SQLite 的 `VACUUM INTO` 把整个数据库安全复制到 target_path（避免 WAL 不一致）。
/// 前端先用 dialog.save 让用户选位置。路径里若有单引号会被转义。
#[tauri::command]
pub async fn backup_database(db: State<'_, Db>, target_path: String) -> AppResult<String> {
    if target_path.is_empty() {
        return Err(crate::error::AppError::Other("target path is empty".into()));
    }
    // 父目录必须存在；若不存在就建一下（dialog 返回的是已存在目录下的新文件名，一般 OK）。
    if let Some(parent) = std::path::Path::new(&target_path).parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|e| {
                crate::error::AppError::Other(format!("create parent dir: {}", e))
            })?;
        }
    }
    // VACUUM INTO 不支持参数绑定，必须拼接。转义单引号（SQLite 里 '' 代表 '）。
    let escaped = target_path.replace('\'', "''");
    let sql = format!("VACUUM INTO '{}'", escaped);
    sqlx::query(&sql).execute(db.inner()).await?;
    Ok(target_path)
}
