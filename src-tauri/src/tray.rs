use crate::db::{self, stickies::Sticky, Db};
use tauri::{
    menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu},
    tray::TrayIconBuilder,
    AppHandle, Emitter, Manager,
};

/// sticky + 内容首行预览，用作菜单里的标题。
type StickyRow = (Sticky, Option<String>);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StickyOpenRoute {
    BindingCoordinator,
    ShowUnbound,
    FocusUnbound,
    OpenUnbound,
}

fn sticky_open_route(
    binding_state: Option<crate::assignment::BindingState>,
    is_hidden: bool,
    has_window: bool,
) -> StickyOpenRoute {
    if binding_state.is_some() {
        StickyOpenRoute::BindingCoordinator
    } else if is_hidden {
        StickyOpenRoute::ShowUnbound
    } else if has_window {
        StickyOpenRoute::FocusUnbound
    } else {
        StickyOpenRoute::OpenUnbound
    }
}

async fn load_all_with_preview(db: &Db) -> Vec<StickyRow> {
    let all = db::stickies::list_all(db).await.unwrap_or_default();
    let mut out = Vec::with_capacity(all.len());
    for s in all {
        let preview = db::items::first_content(db, &s.id)
            .await
            .ok()
            .flatten()
            .and_then(|c| extract_title(&c));
        out.push((s, preview));
    }
    out
}

async fn load_trashed_with_preview(db: &Db) -> Vec<StickyRow> {
    let trashed = db::stickies::list_trashed(db).await.unwrap_or_default();
    let mut out = Vec::with_capacity(trashed.len());
    for s in trashed {
        let preview = db::items::first_content(db, &s.id)
            .await
            .ok()
            .flatten()
            .and_then(|c| extract_title(&c));
        out.push((s, preview));
    }
    out
}

pub fn init(app: &AppHandle) -> tauri::Result<()> {
    let db = app.state::<Db>();
    let (all, trashed) = tauri::async_runtime::block_on(async {
        (
            load_all_with_preview(&db).await,
            load_trashed_with_preview(&db).await,
        )
    });
    let menu = build_menu(app, &all, &trashed)?;
    let icon = app
        .default_window_icon()
        .cloned()
        .expect("default window icon missing");

    TrayIconBuilder::with_id("floaty-tray")
        .icon(icon)
        .tooltip("Floaty")
        .menu(&menu)
        .show_menu_on_left_click(true)
        .on_menu_event(on_menu_event)
        .build(app)?;
    Ok(())
}

/// 从 IPC（async / tokio worker）调用。必须是 async，不能在里面 block_on。
pub async fn refresh_menu(app: &AppHandle) {
    let db = app.state::<Db>();
    let all = load_all_with_preview(&db).await;
    let trashed = load_trashed_with_preview(&db).await;
    if let Some(tray) = app.tray_by_id("floaty-tray") {
        match build_menu(app, &all, &trashed) {
            Ok(menu) => {
                if let Err(e) = tray.set_menu(Some(menu)) {
                    eprintln!("[floaty] tray refresh set_menu failed: {}", e);
                }
            }
            Err(e) => eprintln!("[floaty] tray refresh build_menu failed: {}", e),
        }
    }
}

fn build_menu(
    app: &AppHandle,
    all: &[StickyRow],
    trashed: &[StickyRow],
) -> tauri::Result<Menu<tauri::Wry>> {
    let menu = Menu::new(app)?;
    let heading = MenuItem::with_id(
        app,
        "heading",
        format!("便签 ({})", all.len()),
        false,
        None::<&str>,
    )?;
    menu.append(&heading)?;

    for (i, (s, preview)) in all.iter().enumerate() {
        let label = if s.hidden == 1 {
            format!("· 显示：{}", sticky_display_name(s, preview.as_deref(), i))
        } else {
            let pin = if s.pinned == 1 { "📌 " } else { "" };
            format!("{}{}", pin, sticky_display_name(s, preview.as_deref(), i))
        };
        let item = MenuItem::with_id(
            app,
            format!("sticky:{}", s.id),
            label,
            true,
            None::<&str>,
        )?;
        menu.append(&item)?;
    }

    menu.append(&PredefinedMenuItem::separator(app)?)?;

    let hidden_count = all.iter().filter(|(s, _)| s.hidden == 1).count();
    let label_text = if hidden_count > 0 {
        format!("显示全部（共 {} 张，{} 隐藏）", all.len(), hidden_count)
    } else {
        format!("显示全部（共 {} 张）", all.len())
    };
    let show_all_item = MenuItem::with_id(
        app,
        "show-all",
        label_text,
        !all.is_empty(),
        None::<&str>,
    )?;
    menu.append(&show_all_item)?;

    let visible_count = all.len() - hidden_count;
    let tile_item = MenuItem::with_id(
        app,
        "tile-all",
        format!("一键排版（{} 张可见）", visible_count),
        visible_count > 0,
        None::<&str>,
    )?;
    menu.append(&tile_item)?;

    let new_item = MenuItem::with_id(
        app,
        "new-sticky",
        "＋ 新建便签",
        true,
        Some("CmdOrCtrl+Shift+N"),
    )?;
    menu.append(&new_item)?;

    let search_item = MenuItem::with_id(
        app,
        "search",
        "🔍 搜索便签",
        !all.is_empty(),
        None::<&str>,
    )?;
    menu.append(&search_item)?;

    let help = MenuItem::with_id(app, "welcome", "📖 上手指南", true, None::<&str>)?;
    menu.append(&help)?;

    let prefs = MenuItem::with_id(app, "preferences", "⚙️ 偏好设置", true, None::<&str>)?;
    menu.append(&prefs)?;

    // 回收站：作为子菜单，始终出现（空的时候 disabled）。便签 "删除" 后进这里，30 天后自动清空。
    let trash_label = if trashed.is_empty() {
        "🗑️ 回收站（空）".to_string()
    } else {
        format!("🗑️ 回收站（{}）", trashed.len())
    };
    let trash_submenu = Submenu::with_id(app, "trash", trash_label, !trashed.is_empty())?;
    for (i, (s, preview)) in trashed.iter().enumerate() {
        let item = MenuItem::with_id(
            app,
            format!("trash-restore:{}", s.id),
            format!("恢复：{}", sticky_display_name(s, preview.as_deref(), i)),
            true,
            None::<&str>,
        )?;
        trash_submenu.append(&item)?;
    }
    if !trashed.is_empty() {
        trash_submenu.append(&PredefinedMenuItem::separator(app)?)?;
        let empty = MenuItem::with_id(
            app,
            "trash-empty",
            "清空回收站",
            true,
            None::<&str>,
        )?;
        trash_submenu.append(&empty)?;
    }
    menu.append(&trash_submenu)?;

    menu.append(&PredefinedMenuItem::separator(app)?)?;
    menu.append(&PredefinedMenuItem::quit(app, Some("退出"))?)?;
    Ok(menu)
}

#[cfg(test)]
mod tests {
    use super::{extract_title, sticky_open_route, StickyOpenRoute};
    use crate::assignment::BindingState;

    #[test]
    fn every_binding_state_routes_to_the_coordinator_before_window_state() {
        for state in [
            BindingState::Active,
            BindingState::TargetClosed,
            BindingState::ReconnectRequired,
            BindingState::Unavailable,
        ] {
            for (hidden, has_window) in
                [(false, false), (false, true), (true, false), (true, true)]
            {
                assert_eq!(
                    sticky_open_route(Some(state), hidden, has_window),
                    StickyOpenRoute::BindingCoordinator
                );
            }
        }
    }

    #[test]
    fn heading() {
        assert_eq!(extract_title("# 本周冲刺\n- 这里是内容"), Some("本周冲刺".into()));
    }

    #[test]
    fn task_item_with_due() {
        assert_eq!(
            extract_title("- [ ] 买牛奶 @due:2026-04-25T10:00:00Z"),
            Some("买牛奶".into())
        );
    }

    #[test]
    fn strips_marks_and_link() {
        assert_eq!(
            extract_title("- [ ] **重要** [点这里](https://x.com) 去办"),
            Some("重要 点这里 去办".into())
        );
    }

    #[test]
    fn truncates_long_text() {
        let long = "这是一段非常非常非常非常非常非常非常非常非常长的标题用来测试截断功能是否正常工作";
        let title = extract_title(long).unwrap();
        assert!(title.ends_with('…'));
        assert!(title.chars().count() <= 25);
    }

    #[test]
    fn none_for_empty() {
        assert_eq!(extract_title(""), None);
        assert_eq!(extract_title("   \n\n   "), None);
    }

    #[test]
    fn skips_empty_and_decoration_only_lines() {
        assert_eq!(extract_title("- [ ] \n- [ ] real"), Some("real".into()));
    }
}

fn sticky_display_name(s: &Sticky, preview: Option<&str>, index: usize) -> String {
    if !s.title.is_empty() {
        return s.title.clone();
    }
    if let Some(p) = preview {
        if !p.is_empty() {
            return p.to_string();
        }
    }
    format!("便签 #{}", index + 1)
}

/// 从 markdown 提取一个适合做便签标题的短字符串（<= 24 字符）。
/// 跳过 markdown 装饰（# / - [ ] / > / ** / * / ` / ~~ / [text](url) / @due:...）。
/// 返回 None 表示整段内容没有可读文字。
pub fn extract_title(md: &str) -> Option<String> {
    use regex::Regex;
    // 行首 block 装饰：task / heading / 有序无序列表 / quote
    let block_re = Regex::new(r"^(?:- \[[ xX]\]|#{1,6}|[-*]|\d+\.|>)\s*").ok()?;
    let due_re = Regex::new(r"@due:\S+").ok()?;
    let link_re = Regex::new(r"\[([^\]]+)\]\([^)]+\)").ok()?;
    let bold_re = Regex::new(r"\*\*(.+?)\*\*").ok()?;
    let italic_re = Regex::new(r"\*(.+?)\*").ok()?;
    let strike_re = Regex::new(r"~~(.+?)~~").ok()?;
    let code_re = Regex::new(r"`([^`]+)`").ok()?;

    for raw in md.lines() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        let stripped = block_re.replace(line, "").to_string();
        let mut s = stripped;
        s = due_re.replace_all(&s, "").to_string();
        s = link_re.replace_all(&s, "$1").to_string();
        s = bold_re.replace_all(&s, "$1").to_string();
        s = italic_re.replace_all(&s, "$1").to_string();
        s = strike_re.replace_all(&s, "$1").to_string();
        s = code_re.replace_all(&s, "$1").to_string();
        let trimmed = s.trim();
        if trimmed.is_empty() {
            continue;
        }
        let title: String = trimmed.chars().take(24).collect();
        if trimmed.chars().count() > 24 {
            return Some(format!("{}…", title));
        }
        return Some(title);
    }
    None
}

fn on_menu_event(app: &AppHandle, event: MenuEvent) {
    let id = event.id().as_ref().to_string();
    if id == "preferences" {
        let handle = app.clone();
        tauri::async_runtime::spawn(async move {
            if let Err(e) = crate::commands::windows::open_preferences(handle).await {
                eprintln!("[floaty] open_preferences failed: {}", e);
            }
        });
        return;
    }
    if id == "search" {
        let handle = app.clone();
        tauri::async_runtime::spawn(async move {
            if let Err(e) = crate::commands::windows::open_search(handle).await {
                eprintln!("[floaty] open_search failed: {}", e);
            }
        });
        return;
    }
    if id == "welcome" {
        let handle = app.clone();
        tauri::async_runtime::spawn(async move {
            let db = handle.state::<Db>();
            if let Err(e) = crate::windows::create_welcome(&handle, &db).await {
                eprintln!("[floaty] welcome open failed: {}", e);
            }
            refresh_menu(&handle).await;
        });
        return;
    }
    if id == "new-sticky" {
        let handle = app.clone();
        tauri::async_runtime::spawn(async move {
            let db = handle.state::<Db>();
            match db::stickies::create_default(&db).await {
                Ok(sticky) => {
                    if let Err(e) = crate::windows::open(&handle, &sticky).await {
                        eprintln!("[floaty] tray new-sticky open failed: {}", e);
                    }
                    refresh_menu(&handle).await;
                }
                Err(e) => eprintln!("[floaty] tray new-sticky create failed: {}", e),
            }
        });
    } else if id == "show-all" {
        let handle = app.clone();
        tauri::async_runtime::spawn(async move {
            let db = handle.state::<Db>();
            if let Err(e) = crate::windows::show_all(&handle, &db).await {
                eprintln!("[floaty] tray show-all failed: {}", e);
            }
            refresh_menu(&handle).await;
        });
    } else if id == "tile-all" {
        let handle = app.clone();
        tauri::async_runtime::spawn(async move {
            let db = handle.state::<Db>();
            if let Err(e) = crate::windows::tile_all(&handle, &db).await {
                eprintln!("[floaty] tray tile-all failed: {}", e);
            }
            refresh_menu(&handle).await;
        });
    } else if let Some(sticky_id) = id.strip_prefix("sticky:") {
        let handle = app.clone();
        let sticky_id = sticky_id.to_string();
        tauri::async_runtime::spawn(async move {
            let db = handle.state::<Db>();
            match db::stickies::get(&db, &sticky_id).await {
                Ok(s) => {
                    let binding_state = match db::window_bindings::get(&db, &sticky_id).await {
                        Ok(binding) => binding.map(|binding| binding.state),
                        Err(e) => {
                            eprintln!("[floaty] tray binding lookup failed: {}", e);
                            return;
                        }
                    };
                    let window =
                        handle.get_webview_window(&crate::windows::label(&sticky_id));
                    match sticky_open_route(binding_state, s.hidden == 1, window.is_some()) {
                        StickyOpenRoute::BindingCoordinator => {
                            let result = match handle.try_state::<
                                crate::assignment::coordinator::BindingCoordinator,
                            >() {
                                Some(coordinator) => coordinator.reopen(&sticky_id).await.map(|_| ()),
                                None => Err(crate::error::AppError::Other(
                                    "window binding coordinator unavailable".into(),
                                )),
                            };
                            if let Err(e) = result {
                                eprintln!("[floaty] tray bound-note reopen failed: {}", e);
                            } else if let Err(e) = handle.emit("binding-changed", &sticky_id) {
                                eprintln!("[floaty] tray binding-changed emit failed: {}", e);
                            }
                            refresh_menu(&handle).await;
                        }
                        StickyOpenRoute::ShowUnbound => {
                            if let Err(e) = crate::windows::show(&handle, &sticky_id, &db).await {
                                eprintln!("[floaty] tray show failed: {}", e);
                            }
                            refresh_menu(&handle).await;
                        }
                        StickyOpenRoute::FocusUnbound => {
                            if let Some(window) = window {
                                if let Err(e) = window.set_focus() {
                                    eprintln!("[floaty] tray focus failed: {}", e);
                                }
                            }
                        }
                        StickyOpenRoute::OpenUnbound => {
                            if let Err(e) = crate::windows::open(&handle, &s).await {
                                eprintln!("[floaty] tray reopen failed: {}", e);
                            }
                        }
                    }
                }
                Err(e) => eprintln!("[floaty] tray get failed: {}", e),
            }
        });
    } else if let Some(sticky_id) = id.strip_prefix("trash-restore:") {
        let handle = app.clone();
        let sticky_id = sticky_id.to_string();
        tauri::async_runtime::spawn(async move {
            let db = handle.state::<Db>();
            match db::stickies::restore(&db, &sticky_id).await {
                Ok(s) => {
                    if let Err(e) = crate::windows::open(&handle, &s).await {
                        eprintln!("[floaty] tray restore open failed: {}", e);
                    }
                    refresh_menu(&handle).await;
                }
                Err(e) => eprintln!("[floaty] tray restore failed: {}", e),
            }
        });
    } else if id == "trash-empty" {
        let handle = app.clone();
        tauri::async_runtime::spawn(async move {
            let db = handle.state::<Db>();
            match db::stickies::purge_older_than(&db, i64::MAX).await {
                Ok(n) => eprintln!("[floaty] trash emptied: purged {} stickies", n),
                Err(e) => eprintln!("[floaty] trash empty failed: {}", e),
            }
            refresh_menu(&handle).await;
        });
    }
}
