use std::future::Future;

use crate::db::{
    self,
    stickies::{Sticky, StickyPatch},
    Db,
};
use crate::error::{AppError, AppResult};
use tauri::{AppHandle, Emitter, Manager, State};

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
    let binding = db::window_bindings::get(&db, &id).await?;
    let was_bound = binding.is_some();
    if was_bound {
        let coordinator = app
            .try_state::<crate::assignment::coordinator::BindingCoordinator>()
            .ok_or_else(|| AppError::Other("window binding coordinator unavailable".into()))?;
        prepare_close_and_soft_delete(
            &db,
            &id,
            || coordinator.prepare_delete(&id),
            || close_sticky_window(&app, &id),
        )
        .await?;
    } else {
        close_sticky_window(&app, &id)?;
        db::stickies::delete(&db, &id).await?;
    }
    if was_bound {
        if let Err(error) = app.emit("binding-changed", &id) {
            eprintln!(
                "[floaty] emit binding-changed after delete failed for {}: {}",
                id, error
            );
        }
    }
    crate::tray::refresh_menu(&app).await;
    Ok(())
}

async fn prepare_close_and_soft_delete<F, Fut, C>(
    db: &Db,
    sticky_id: &str,
    prepare: F,
    close: C,
) -> AppResult<()>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = AppResult<()>>,
    C: FnOnce() -> AppResult<()>,
{
    prepare().await?;
    close()?;
    db::stickies::delete(db, sticky_id).await
}

fn close_sticky_window(app: &AppHandle, sticky_id: &str) -> AppResult<()> {
    if let Some(window) = app.get_webview_window(&crate::windows::label(sticky_id)) {
        window
            .close()
            .map_err(|error| AppError::Other(format!("delete_sticky close: {error}")))?;
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;
    use std::sync::{Arc, Mutex};

    async fn test_pool() -> Db {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        crate::db::migrations::run(&pool).await.unwrap();
        pool
    }

    async fn bind_test_window(db: &Db, sticky_id: &str) {
        db::window_bindings::bind(
            db,
            db::window_bindings::NewWindowBinding {
                sticky_id: sticky_id.into(),
                app_bundle_id: "com.example.Target".into(),
                app_name: "Target".into(),
                window_title: "Document".into(),
                runtime_pid: 42,
                runtime_window_id: 101,
            },
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn soft_delete_prepares_runtime_then_removes_binding_transactionally() {
        let db = test_pool().await;
        let sticky = db::stickies::create_default(&db).await.unwrap();
        bind_test_window(&db, &sticky.id).await;
        let order = Arc::new(Mutex::new(Vec::new()));

        prepare_close_and_soft_delete(
            &db,
            &sticky.id,
            {
                let order = order.clone();
                || async move {
                    order.lock().unwrap().push("prepare");
                    Ok(())
                }
            },
            {
                let order = order.clone();
                move || {
                    order.lock().unwrap().push("close");
                    Ok(())
                }
            },
        )
        .await
        .unwrap();

        assert_eq!(*order.lock().unwrap(), vec!["prepare", "close"]);
        assert!(db::window_bindings::get(&db, &sticky.id)
            .await
            .unwrap()
            .is_none());
        assert!(db::stickies::get(&db, &sticky.id)
            .await
            .unwrap()
            .deleted_at
            .is_some());
    }

    #[tokio::test]
    async fn soft_delete_failure_happens_after_prepare_and_keeps_binding_row() {
        let db = test_pool().await;
        let sticky = db::stickies::create_default(&db).await.unwrap();
        bind_test_window(&db, &sticky.id).await;
        sqlx::query(
            "CREATE TRIGGER fail_soft_delete
             BEFORE UPDATE OF deleted_at ON stickies
             WHEN NEW.deleted_at IS NOT NULL
             BEGIN SELECT RAISE(ABORT, 'blocked'); END",
        )
        .execute(&db)
        .await
        .unwrap();
        let prepared = Arc::new(Mutex::new(false));

        let closed = Arc::new(Mutex::new(false));
        let result = prepare_close_and_soft_delete(
            &db,
            &sticky.id,
            {
                let prepared = prepared.clone();
                || async move {
                    *prepared.lock().unwrap() = true;
                    Ok(())
                }
            },
            {
                let closed = closed.clone();
                move || {
                    *closed.lock().unwrap() = true;
                    Ok(())
                }
            },
        )
        .await;

        assert!(result.is_err());
        assert!(*prepared.lock().unwrap());
        assert!(*closed.lock().unwrap());
        assert!(db::window_bindings::get(&db, &sticky.id)
            .await
            .unwrap()
            .is_some());
        assert!(db::stickies::get(&db, &sticky.id)
            .await
            .unwrap()
            .deleted_at
            .is_none());
    }

    #[tokio::test]
    async fn close_failure_stops_before_soft_delete() {
        let db = test_pool().await;
        let sticky = db::stickies::create_default(&db).await.unwrap();
        bind_test_window(&db, &sticky.id).await;

        let result = prepare_close_and_soft_delete(
            &db,
            &sticky.id,
            || async { Ok(()) },
            || Err(AppError::Other("close failed".into())),
        )
        .await;

        assert_eq!(result.unwrap_err().to_string(), "close failed");
        assert!(db::window_bindings::get(&db, &sticky.id)
            .await
            .unwrap()
            .is_some());
        assert!(db::stickies::get(&db, &sticky.id)
            .await
            .unwrap()
            .deleted_at
            .is_none());
    }
}
