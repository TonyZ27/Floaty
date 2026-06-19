use crate::db::{self, stickies::{Sticky, StickyPatch}, Db};
use crate::error::{AppError, AppResult};
use tauri::{AppHandle, Manager, State};

#[tauri::command]
pub async fn get_or_create_default_sticky(app: AppHandle, db: State<'_, Db>) -> AppResult<Sticky> {
    let visible = db::stickies::list_visible(&db).await?;
    if let Some(s) = visible.into_iter().next() {
        return Ok(s);
    }
    let s = db::stickies::create_default(&db).await?;
    crate::tray::refresh_menu(&app).await;
    Ok(s)
}

#[tauri::command]
pub async fn list_stickies(db: State<'_, Db>) -> AppResult<Vec<Sticky>> {
    db::stickies::list_visible(&db).await
}

#[tauri::command]
pub async fn list_all_stickies(db: State<'_, Db>) -> AppResult<Vec<Sticky>> {
    db::stickies::list_all(&db).await
}

#[tauri::command]
pub async fn get_sticky(db: State<'_, Db>, id: String) -> AppResult<Sticky> {
    db::stickies::get(&db, &id).await
}

#[tauri::command]
pub async fn create_sticky(app: AppHandle, db: State<'_, Db>) -> AppResult<Sticky> {
    let s = db::stickies::create_default(&db).await?;
    crate::tray::refresh_menu(&app).await;
    Ok(s)
}

#[tauri::command]
pub async fn update_sticky(
    app: AppHandle,
    db: State<'_, Db>,
    id: String,
    patch: StickyPatch,
) -> AppResult<Sticky> {
    let old_sticky = db::stickies::get(&db, &id).await?;
    let s = db::stickies::update(&db, &id, patch.clone()).await?;
    
    if let Some(collapsed) = patch.collapsed {
        if collapsed != old_sticky.collapsed {
            if let Some(w) = app.get_webview_window(&crate::windows::label(&id)) {
                if collapsed == 1 {
                    let _ = w.set_min_size(Some(tauri::LogicalSize::new(200.0, 36.0)));
                    let _ = w.set_max_size(Some(tauri::LogicalSize::new(1600.0, 36.0)));
                    let _ = w.set_size(tauri::LogicalSize::new(s.collapsed_w as f64, 36.0));
                } else {
                    let _ = w.set_min_size(Some(tauri::LogicalSize::new(200.0, 200.0)));
                    let _ = w.set_max_size(Some(tauri::LogicalSize::new(1600.0, 1600.0)));
                    let _ = w.set_size(tauri::LogicalSize::new(s.w as f64, s.h as f64));
                }
            }
        }
    }
    
    crate::tray::refresh_menu(&app).await;
    Ok(s)
}

#[tauri::command]
pub async fn delete_sticky(app: AppHandle, db: State<'_, Db>, id: String) -> AppResult<()> {
    // 软删：关闭窗口 + 设 deleted_at（items / reminders 暂留，等 purge 时再物理级联）
    if let Some(w) = app.get_webview_window(&crate::windows::label(&id)) {
        w.close()
            .map_err(|e| AppError::Other(format!("delete_sticky close: {}", e)))?;
    }
    db::stickies::delete(&db, &id).await?;
    crate::tray::refresh_menu(&app).await;
    Ok(())
}

#[tauri::command]
pub async fn list_trashed_stickies(db: State<'_, Db>) -> AppResult<Vec<Sticky>> {
    db::stickies::list_trashed(&db).await
}

#[tauri::command]
pub async fn restore_sticky(app: AppHandle, db: State<'_, Db>, id: String) -> AppResult<Sticky> {
    let s = db::stickies::restore(&db, &id).await?;
    // 还原后自动打开（像新建一样）
    let _ = crate::windows::open(&app, &s).await;
    crate::tray::refresh_menu(&app).await;
    Ok(s)
}

#[tauri::command]
pub async fn purge_sticky(app: AppHandle, db: State<'_, Db>, id: String) -> AppResult<()> {
    db::stickies::purge(&db, &id).await?;
    crate::tray::refresh_menu(&app).await;
    Ok(())
}

#[tauri::command]
pub async fn empty_trash(app: AppHandle, db: State<'_, Db>) -> AppResult<u64> {
    // 把所有回收站便签硬删：传一个"未来很远"的 cutoff
    let future = i64::MAX;
    let count = db::stickies::purge_older_than(&db, future).await?;
    crate::tray::refresh_menu(&app).await;
    Ok(count)
}
