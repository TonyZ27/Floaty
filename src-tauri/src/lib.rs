use tauri::Manager;

mod assignment;
mod commands;
mod db;
mod error;
mod platform;
mod reminders;
mod tray;
mod windows;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_shortcuts(["CmdOrCtrl+Shift+N"])
                .expect("register global shortcut")
                .with_handler(|app, shortcut, event| {
                    use tauri_plugin_global_shortcut::ShortcutState;
                    let sc_str = shortcut.into_string();
                    eprintln!(
                        "[floaty] global shortcut fired: '{}' state={:?}",
                        sc_str,
                        event.state()
                    );
                    if event.state() != ShortcutState::Pressed {
                        return;
                    }
                    // 实际字符串是 'shift+super+KeyN'，只匹配 keyN 即可
                    if sc_str.to_lowercase().contains("keyn") {
                        let handle = app.clone();
                        tauri::async_runtime::spawn(async move {
                            let db = handle.state::<crate::db::Db>();
                            let enabled: Option<(String,)> =
                                sqlx::query_as("SELECT value FROM settings WHERE key = 'global_shortcut_enabled'")
                                    .fetch_optional(db.inner())
                                    .await
                                    .unwrap_or(None);
                            if enabled.as_ref().map(|(v,)| v.as_str()) == Some("false") {
                                eprintln!("[floaty] global shortcut disabled via settings, ignoring");
                                return;
                            }
                            match crate::db::stickies::create_default(&db).await {
                                Ok(sticky) => {
                                    let _ = crate::windows::open(&handle, &sticky).await;
                                    crate::tray::refresh_menu(&handle).await;
                                    eprintln!("[floaty] global ⌘⇧N: new sticky {}", &sticky.id[..8]);
                                }
                                Err(e) => eprintln!("[floaty] global ⌘⇧N create failed: {}", e),
                            }
                        });
                    }
                })
                .build(),
        )
        .setup(|app| {
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            let handle = app.handle().clone();
            let data_dir = handle.path().app_data_dir().expect("app_data_dir");
            let pool = tauri::async_runtime::block_on({
                let data_dir = data_dir.clone();
                async move { db::init(data_dir).await.expect("db init") }
            });
            app.manage(pool.clone());
            tauri::async_runtime::block_on(
                db::window_bindings::mark_all_reconnect_required(&pool),
            )
            .expect("mark bindings reconnect-required");

            use std::sync::Arc;
            #[cfg(target_os = "macos")]
            let raw_window_catalog: Arc<dyn assignment::WindowCatalog> = Arc::new(
                platform::macos::window_catalog::MacOSWindowCatalog::default(),
            );
            #[cfg(not(target_os = "macos"))]
            let raw_window_catalog: Arc<dyn assignment::WindowCatalog> =
                Arc::new(assignment::UnsupportedWindowCatalog);
            let window_catalog = Arc::new(
                assignment::coordinator::MainThreadWindowCatalog::new(
                    handle.clone(),
                    raw_window_catalog,
                ),
            );
            app.manage(window_catalog.clone());

            #[cfg(target_os = "macos")]
            {
                let repository = Arc::new(
                    assignment::coordinator::DbBindingRepository::new(pool.clone()),
                );
                let spaces = Arc::new(
                    platform::macos::window_spaces::MacOSWindowSpaceBackend::default(),
                );
                let native = Arc::new(
                    assignment::coordinator::TauriNativeWindowRuntime::new(
                        handle.clone(),
                        spaces,
                    ),
                );
                let note_windows = Arc::new(
                    assignment::coordinator::TauriNoteWindowRuntime::new(
                        handle.clone(),
                        pool.clone(),
                    ),
                );
                let observers = Arc::new(
                    platform::macos::accessibility::MacOSAccessibilityObserverFactory::new(
                        handle.clone(),
                    ),
                );
                app.manage(assignment::coordinator::BindingCoordinator::new(
                    repository,
                    window_catalog,
                    native,
                    note_windows,
                    observers,
                ));
            }

            // 自动快照：不阻塞启动，失败也不影响。保留最近 10 份。
            {
                let snap_pool = pool.clone();
                let snap_dir = data_dir.clone();
                tauri::async_runtime::spawn(async move {
                    match db::snapshots::take(&snap_pool, &snap_dir).await {
                        Ok(path) => eprintln!("[floaty] startup snapshot → {}", path.display()),
                        Err(e) => eprintln!("[floaty] startup snapshot failed: {}", e),
                    }
                });
            }

            // 自动清理回收站：硬删 deleted_at 超过 30 天的便签。
            {
                let purge_pool = pool.clone();
                tauri::async_runtime::spawn(async move {
                    let cutoff = chrono::Utc::now().timestamp_millis()
                        - 30 * 24 * 60 * 60 * 1000;
                    match db::stickies::purge_older_than(&purge_pool, cutoff).await {
                        Ok(0) => {}
                        Ok(n) => eprintln!("[floaty] auto-purged {} stickies older than 30d", n),
                        Err(e) => eprintln!("[floaty] auto-purge failed: {}", e),
                    }
                });
            }

            let scheduler = reminders::Scheduler::new();
            app.manage(scheduler.clone());
            scheduler.spawn_loop(app.handle().clone());

            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                if let Err(e) = windows::restore_on_startup(&handle, &pool).await {
                    eprintln!("[floaty] restore_on_startup failed: {}", e);
                }
            });

            tray::init(app.handle()).expect("tray init");

            // 确认全局快捷键注册状态
            use tauri_plugin_global_shortcut::GlobalShortcutExt;
            let registered = app.global_shortcut().is_registered("CmdOrCtrl+Shift+N");
            eprintln!("[floaty] global shortcut 'CmdOrCtrl+Shift+N' registered: {}", registered);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::stickies::get_or_create_default_sticky,
            commands::stickies::list_stickies,
            commands::stickies::list_all_stickies,
            commands::stickies::get_sticky,
            commands::stickies::create_sticky,
            commands::stickies::update_sticky,
            commands::stickies::delete_sticky,
            commands::stickies::list_trashed_stickies,
            commands::stickies::restore_sticky,
            commands::stickies::purge_sticky,
            commands::stickies::empty_trash,
            commands::assignment::get_accessibility_status,
            commands::assignment::request_accessibility_permission,
            commands::assignment::open_accessibility_settings,
            commands::assignment::list_eligible_windows,
            commands::assignment::get_window_binding,
            commands::assignment::list_window_bindings,
            commands::assignment::bind_note_to_window,
            commands::assignment::unbind_note_from_window,
            commands::assignment::reopen_bound_note,
            commands::assignment::open_window_picker,
            commands::items::list_items,
            commands::items::upsert_item,
            commands::items::toggle_item,
            commands::items::delete_item,
            commands::windows::open_sticky_window,
            commands::windows::hide_sticky,
            commands::windows::show_sticky,
            commands::windows::toggle_pin,
            commands::windows::show_all_stickies,
            commands::windows::tile_all_stickies,
            commands::windows::new_sticky_window,
            commands::windows::open_welcome,
            commands::windows::open_preferences,
            commands::windows::open_search,
            commands::search::search_stickies,
            commands::windows::get_stats,
            commands::windows::get_data_dir,
            commands::windows::get_setting,
            commands::windows::set_setting,
            commands::windows::backup_database,
            commands::reminders::sync_reminders,
            commands::reminders::snooze_reminder,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|_app_handle, event| {
            // 便签全部关掉时不退出；保持 tray 常驻，用户可随时从菜单栏恢复/新建
            if let tauri::RunEvent::ExitRequested { api, code, .. } = event {
                if code.is_none() {
                    api.prevent_exit();
                }
            }
        });
}
