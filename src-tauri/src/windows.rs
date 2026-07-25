use crate::db::{self, stickies::Sticky, Db};
use crate::error::{AppError, AppResult};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

/// 标签前缀 + sticky id 构成窗口 label
pub fn label(sticky_id: &str) -> String {
    format!("sticky-{}", sticky_id)
}

/// 从 label 反解 sticky id
pub fn id_from_label(label: &str) -> Option<&str> {
    label.strip_prefix("sticky-")
}

/// 打开一个便签窗口（已存在则 show + focus）。成功返回 WebviewWindow。
pub async fn open(app: &AppHandle, sticky: &Sticky) -> AppResult<WebviewWindow> {
    open_with_mode(app, sticky, false).await
}

/// Ensures a manually hidden Note has a native window for attachment without
/// revealing it in the current Space.
pub async fn open_for_binding(
    app: &AppHandle,
    sticky: &Sticky,
) -> AppResult<WebviewWindow> {
    open_with_mode(app, sticky, true).await
}

async fn open_with_mode(
    app: &AppHandle,
    sticky: &Sticky,
    keep_hidden: bool,
) -> AppResult<WebviewWindow> {
    let lbl = label(&sticky.id);
    if let Some(w) = app.get_webview_window(&lbl) {
        if keep_hidden {
            let _ = w.set_always_on_top(false);
            w.set_position(tauri::LogicalPosition::new(-32000.0, -32000.0))
                .map_err(|e| {
                    AppError::Other(format!("window set_position binding hidden: {}", e))
                })?;
            return Ok(w);
        }
        w.show().ok();
        w.set_focus().ok();
        return Ok(w);
    }

    let url = format!("index.html#/sticky/{}", sticky.id);
    // 防御式 clamp：如果过去因 bug 存了异常大的"逻辑尺寸"，重置为默认
    let (w, h) = sanitize_size(sticky.w, sticky.h);
    let is_collapsed = sticky.collapsed == 1;
    let final_w = if is_collapsed { (sticky.collapsed_w).max(200).min(1600) as f64 } else { w as f64 };
    let final_h = if is_collapsed { 36.0 } else { h as f64 };

    let mut builder = WebviewWindowBuilder::new(app, &lbl, WebviewUrl::App(url.into()))
        .title("Floaty")
        .inner_size(final_w, final_h)
        .resizable(true)
        .decorations(false)
        .transparent(true)
        .always_on_top(sticky.pinned == 1)
        .visible(true);

    if is_collapsed {
        builder = builder
            .min_inner_size(200.0, 36.0)
            .max_inner_size(1600.0, 36.0);
    } else {
        builder = builder
            .min_inner_size(200.0, 200.0)
            .max_inner_size(1600.0, 1600.0);
    }

    if keep_hidden {
        builder = builder.position(-32000.0, -32000.0);
    } else if let (Some(x), Some(y)) = (sticky.x, sticky.y) {
        if let Some((sx, sy)) = sanitize_position(x, y) {
            // 再检查一遍矩形是否在任一显示器可见区域内
            // （用户拔副屏 / 当前桌面空间不同 / dev restart 时显示器刚连上时）
            if position_visible_on_any_monitor(app, sx, sy, w, h) {
                builder = builder.position(sx as f64, sy as f64);
            } else {
                eprintln!(
                    "[floaty] sticky {} stored pos ({},{}) offscreen → using default",
                    &sticky.id[..8],
                    sx,
                    sy
                );
            }
        }
    }

    let window = builder
        .build()
        .map_err(|e| AppError::Other(format!("window build failed: {}", e)))?;

    attach_geometry_listener(&window, sticky.id.clone())?;
    Ok(window)
}

/// 关闭（隐藏）便签窗口。策略：
/// 1) DB 标记 hidden=1
/// 2) 不调 w.hide()（会触发 macOS key-window cascade → 另一张 sticky 被拉前）
///    也不调 w.close()（会 destroy + 触发全 app 窗口重排）
///    而是把窗口移到屏幕外（-32000, -32000）+ 设为非 always_on_top，
///    macOS 认为窗口还"可见"于某处，不做 key-window 重新挑选，
///    其它 sticky 的 z-order 完全不变。
///    重新显示时把位置恢复并（如 pinned）重新置顶。
pub async fn hide(app: &AppHandle, sticky_id: &str, db: &Db) -> AppResult<()> {
    db::stickies::update(
        db,
        sticky_id,
        db::stickies::StickyPatch {
            hidden: Some(1),
            ..Default::default()
        },
    )
    .await?;
    if let Some(w) = app.get_webview_window(&label(sticky_id)) {
        // 先取消 always_on_top，否则即使在屏幕外也会在桌面切换时闪现
        let _ = w.set_always_on_top(false);
        w.set_position(tauri::LogicalPosition::new(-32000.0, -32000.0))
            .map_err(|e| AppError::Other(format!("window set_position offscreen: {}", e)))?;
    }
    Ok(())
}

/// 从菜单栏恢复一个已隐藏的便签：
/// 若窗口已存在（被 hide 挪到屏幕外）→ 恢复位置 + 重新 pinned 状态
/// 不存在 → 当作冷启动打开
pub async fn show(app: &AppHandle, sticky_id: &str, db: &Db) -> AppResult<WebviewWindow> {
    let sticky = db::stickies::update(
        db,
        sticky_id,
        db::stickies::StickyPatch {
            hidden: Some(0),
            ..Default::default()
        },
    )
    .await?;
    if let Some(w) = app.get_webview_window(&label(sticky_id)) {
        // 从 offscreen 恢复位置
        let (x, y) = match (sticky.x, sticky.y) {
            (Some(x), Some(y)) => sanitize_position(x, y).unwrap_or((200, 200)),
            _ => (200, 200),
        };
        w.set_position(tauri::LogicalPosition::new(x as f64, y as f64))
            .map_err(|e| AppError::Other(format!("window set_position restore: {}", e)))?;
        let _ = w.set_always_on_top(sticky.pinned == 1);
        w.set_focus().ok();
        return Ok(w);
    }
    open(app, &sticky).await
}

/// Restores a bound Note's runtime position without changing its persisted
/// manual-hidden flag. The coordinator commits that flag only after native
/// target validation and attachment have succeeded.
pub async fn reveal_for_binding(
    app: &AppHandle,
    sticky_id: &str,
    db: &Db,
) -> AppResult<WebviewWindow> {
    let sticky = db::stickies::get(db, sticky_id).await?;
    if let Some(window) = app.get_webview_window(&label(sticky_id)) {
        let (x, y) = match (sticky.x, sticky.y) {
            (Some(x), Some(y)) => sanitize_position(x, y).unwrap_or((200, 200)),
            _ => (200, 200),
        };
        window
            .set_position(tauri::LogicalPosition::new(x as f64, y as f64))
            .map_err(|error| {
                AppError::Other(format!("window set_position binding restore: {error}"))
            })?;
        window
            .set_always_on_top(false)
            .map_err(|error| AppError::Other(format!("disable global pinning: {error}")))?;
        return Ok(window);
    }
    open(app, &sticky).await
}

/// 切换 always-on-top。返回新的 pinned 状态。
pub async fn toggle_pin(app: &AppHandle, sticky_id: &str, db: &Db) -> AppResult<bool> {
    ensure_pin_is_user_managed(db, sticky_id).await?;
    let s = db::stickies::get(db, sticky_id).await?;
    let next = if s.pinned == 1 { 0 } else { 1 };
    db::stickies::update(
        db,
        sticky_id,
        db::stickies::StickyPatch {
            pinned: Some(next),
            ..Default::default()
        },
    )
    .await?;
    if let Some(w) = app.get_webview_window(&label(sticky_id)) {
        w.set_always_on_top(next == 1)
            .map_err(|e| AppError::Other(format!("set_always_on_top: {}", e)))?;
    }
    Ok(next == 1)
}

async fn ensure_pin_is_user_managed(db: &Db, sticky_id: &str) -> AppResult<()> {
    if db::window_bindings::get(db, sticky_id).await?.is_some() {
        return Err(AppError::Other(
            "pin is managed by the active window binding".into(),
        ));
    }
    Ok(())
}

#[cfg(target_os = "macos")]
pub fn suspend_for_binding(
    app: &AppHandle,
    sticky_id: &str,
    spaces: &dyn crate::platform::macos::window_spaces::WindowSpaceBackend,
) -> AppResult<()> {
    let window = app
        .get_webview_window(&label(sticky_id))
        .ok_or(AppError::NotFound)?;
    let note_window_id =
        crate::platform::macos::window_spaces::note_window_id(&window)?;
    spaces.order_out(note_window_id)
}

#[cfg(target_os = "macos")]
pub fn resume_for_binding(
    app: &AppHandle,
    sticky_id: &str,
    target_window_id: u32,
    spaces: &dyn crate::platform::macos::window_spaces::WindowSpaceBackend,
) -> AppResult<()> {
    let window = app
        .get_webview_window(&label(sticky_id))
        .ok_or(AppError::NotFound)?;
    let note_window_id =
        crate::platform::macos::window_spaces::note_window_id(&window)?;
    spaces.order_front(target_window_id, note_window_id)
}

/// 一键排版：把所有可见便签按网格重新摆放（320x420 + 40px 间距，4 列 wrap）。
/// 隐藏的便签不动，等显示时再排。
pub async fn tile_all(app: &AppHandle, db: &Db) -> AppResult<usize> {
    let visible = db::stickies::list_visible_unbound(db).await?;
    let start_x = 50_i64;
    let start_y = 50_i64;
    let cell_w = 320_i64 + 40;
    let cell_h = 420_i64 + 40;
    let cols = 4_usize;
    let mut count = 0;
    for (i, s) in visible.iter().enumerate() {
        let col = (i % cols) as i64;
        let row = (i / cols) as i64;
        let x = start_x + col * cell_w;
        let y = start_y + row * cell_h;
        
        let is_collapsed = s.collapsed == 1;
        let patch = if is_collapsed {
            db::stickies::StickyPatch {
                x: Some(x),
                y: Some(y),
                collapsed_w: Some(320),
                ..Default::default()
            }
        } else {
            db::stickies::StickyPatch {
                x: Some(x),
                y: Some(y),
                w: Some(320),
                h: Some(420),
                ..Default::default()
            }
        };
        // 1) 写回 DB
        db::stickies::update(db, &s.id, patch).await?;

        let target_w = 320.0;
        let target_h = if is_collapsed { 36.0 } else { 420.0 };

        // 2) 立即应用到运行中的窗口
        if let Some(w) = app.get_webview_window(&label(&s.id)) {
            let _ = w.set_position(tauri::LogicalPosition::new(x as f64, y as f64));
            let _ = w.set_size(tauri::LogicalSize::new(target_w, target_h));
            let _ = w.show();
            let _ = w.set_focus();
        } else {
            let refreshed = db::stickies::get(db, &s.id).await?;
            let _ = open(app, &refreshed).await;
        }
        count += 1;
    }
    Ok(count)
}

/// "Bring all to front"：把所有便签都推到桌面最前。
/// - hidden=1：先 un-hide（DB + webview）+ open 窗口
/// - hidden=0 有 window：show + set_focus 提到前面
/// - hidden=0 无 window（异常）：open 重建
/// 返回被操作的便签数。
pub async fn show_all(app: &AppHandle, db: &Db) -> AppResult<usize> {
    let all = db::stickies::list_all_unbound(db).await?;
    let mut shown = 0;
    for s in &all {
        if s.hidden == 1 {
            if let Err(e) = show(app, &s.id, db).await {
                eprintln!("[floaty] show_all (hidden): {} failed: {}", s.id, e);
                continue;
            }
        } else if let Some(w) = app.get_webview_window(&label(&s.id)) {
            let _ = w.show();
            let _ = w.set_focus();
        } else if let Err(e) = open(app, s).await {
            eprintln!("[floaty] show_all (reopen): {} failed: {}", s.id, e);
            continue;
        }
        shown += 1;
    }
    Ok(shown)
}

/// 冷启动：打开所有 hidden=0 的便签。
/// 仅当**全库为空**（首次安装）时才自动创建一张默认便签；
/// 有便签但都 hidden=1 时，只启动 tray，让用户通过菜单栏恢复，避免每次重启都冒出空白新便签。
pub async fn restore_on_startup(app: &AppHandle, db: &Db) -> AppResult<()> {
    let visible = db::stickies::list_visible_unbound(db).await?;
    eprintln!("[floaty] restore_on_startup: found {} visible stickies", visible.len());
    if !visible.is_empty() {
        for s in visible {
            eprintln!(
                "[floaty] restore: opening sticky {} at ({:?},{:?}) {}x{} pinned={}",
                &s.id[..8],
                s.x,
                s.y,
                s.w,
                s.h,
                s.pinned
            );
            match open(app, &s).await {
                Ok(w) => {
                    let pos = w.outer_position().ok();
                    let size = w.inner_size().ok();
                    eprintln!(
                        "[floaty] restore: opened → actual pos={:?} size={:?}",
                        pos, size
                    );
                }
                Err(e) => eprintln!("[floaty] restore: open failed for {}: {}", &s.id[..8], e),
            }
        }
    } else {
        let all = db::stickies::list_all(db).await?;
        if all.is_empty() {
            create_welcome(app, db).await?;
        }
    }
    // tray::init 是同步 block_on 的，restore 是 spawn 的；如果 restore 里建了 welcome，
    // tray 菜单是"空 DB"状态下构建的，对不上。这里无论走哪条分支都刷一次。
    crate::tray::refresh_menu(app).await;
    Ok(())
}

/// 新建一张欢迎便签，塞入 WELCOME_MARKDOWN 内容并打开。
/// 可从菜单栏 / 偏好设置按钮触发，用于老用户首次看到上手指南。
pub async fn create_welcome(app: &AppHandle, db: &Db) -> AppResult<Sticky> {
    let fresh = db::stickies::create_default(db).await?;
    let _ = db::items::upsert(
        db,
        db::items::ItemUpsert {
            id: None,
            sticky_id: fresh.id.clone(),
            content_md: WELCOME_MARKDOWN.to_string(),
            due_at: None,
            sort_order: 0,
        },
    )
    .await;
    open(app, &fresh).await?;
    Ok(fresh)
}

#[cfg(target_os = "macos")]
const WELCOME_MARKDOWN: &str = r#"# 欢迎使用 Floaty 👋

这是你的第一张便签。随便改。

## 加一条待办（不懂 markdown 也能用）

**最简单**：光标放到空白行 → 行尾弹出小工具栏 → 点「☐ 待办」。

- [ ] 这是一个待办。点左边的 ☐ 打勾
- [x] 勾上的会变灰 + 划线
- [ ] 回车继续写下一条；连按两次回车退出待办

## 加截止时间

- [ ] 输入 `@` 打开时间选择器（快捷选项 + 倒计时 + 自定义）
- [ ] 或者直接打 "今天 18 点" / "明天下午 3 点" / "30 分钟后" / "下周一"，按 Tab 接受
- [ ] 时间自动染色：逾期红 / 1h 内红 / 今天橙 / 本周黄 / 更晚灰
- [ ] 到点系统通知；点 pill 可重新选时间

## 加粗/斜/链接（选中文字）

**选中**一段文字 → 自动弹出格式工具栏：
粗 · 斜 · 删除线 · 行内码 · H1 / H2 · 🔗 链接

## 窗口操作

- 拖顶部条移动；右下角拖拽 resize
- `📌` 置顶到所有窗口之上；再点取消
- `✕` 只隐藏（菜单栏随时找回），`⌘⌫` 才是彻底删
- `⚙️` 改背景色 / 透明度 / 字号 / 字色

## 菜单栏（屏幕右上角 📋 图标）

新建 · 显示全部 · **一键排版**（重新摆成网格） · **📖 上手指南**（再打开这份）· 偏好设置

## 快捷键（macOS）

- **⌘⇧N** 全局新建（任何 app 都能用）
- **⌘N** 新建 · **⌘W** 隐藏 · **⌘⌫** 删除 · **⌘⇧P** 切换置顶 · **⌘,** 外观设置
- 编辑：**⌘B/⌘I** 粗/斜 · **⌘Z / ⌘⇧Z** 撤销/重做 · **Tab** 接受时间 ghost

想关掉浮动工具栏？菜单栏 → 偏好设置 → 编辑器。
"#;

#[cfg(not(target_os = "macos"))]
const WELCOME_MARKDOWN: &str = r#"# 欢迎使用 Floaty 👋

这是你的第一张便签。随便改。

## 加一条待办（不懂 markdown 也能用）

**最简单**：光标放到空白行 → 行尾弹出小工具栏 → 点「☐ 待办」。

- [ ] 这是一个待办。点左边的 ☐ 打勾
- [x] 勾上的会变灰 + 划线
- [ ] 回车继续写下一条；连按两次回车退出待办

## 加截止时间

- [ ] 输入 `@` 打开时间选择器（快捷选项 + 倒计时 + 自定义）
- [ ] 或者直接打 "今天 18 点" / "明天下午 3 点" / "30 分钟后" / "下周一"，按 Tab 接受
- [ ] 时间自动染色：逾期红 / 1h 内红 / 今天橙 / 本周黄 / 更晚灰
- [ ] 到点系统通知；点 pill 可重新选时间

## 加粗/斜/链接（选中文字）

**选中**一段文字 → 自动弹出格式工具栏：
粗 · 斜 · 删除线 · 行内码 · H1 / H2 · 🔗 链接

## 窗口操作

- 拖顶部条移动；右下角拖拽 resize
- `📌` 置顶到所有窗口之上；再点取消
- `✕` 只隐藏（托盘随时找回），Ctrl+Backspace 才是彻底删
- `⚙️` 改背景色 / 透明度 / 字号 / 字色

## 系统托盘（屏幕右下角 📋 图标）

新建 · 显示全部 · **一键排版**（重新摆成网格） · **📖 上手指南**（再打开这份）· 偏好设置

## 快捷键（Windows / Linux）

- **Ctrl+Shift+N** 全局新建（任何 app 都能用）
- **Ctrl+N** 新建 · **Ctrl+W** 隐藏 · **Ctrl+Backspace** 删除 · **Ctrl+Shift+P** 切换置顶 · **Ctrl+,** 外观设置
- 编辑：**Ctrl+B / Ctrl+I** 粗/斜 · **Ctrl+Z / Ctrl+Shift+Z** 撤销/重做 · **Tab** 接受时间 ghost

想关掉浮动工具栏？托盘 → 偏好设置 → 编辑器。
"#;


/// 合理性 clamp：sticky 宽高应在 [200, 1600] 之间，否则重置为默认 320×420
fn sanitize_size(w: i64, h: i64) -> (i64, i64) {
    let w_ok = (200..=1600).contains(&w);
    let h_ok = (200..=1600).contains(&h);
    (if w_ok { w } else { 320 }, if h_ok { h } else { 420 })
}

/// 合理性 clamp：保守屏幕范围（含多显示器）。离谱坐标视为 stale bug 数据。
fn sanitize_position(x: i64, y: i64) -> Option<(i64, i64)> {
    if (-3000..=5000).contains(&x) && (-500..=3000).contains(&y) {
        Some((x, y))
    } else {
        None
    }
}

/// 检查 (x,y,w,h) 矩形是否与任一已连接显示器的可见区域有重叠。
/// 没有重叠（即窗口完全在所有屏幕之外）→ 返回 false，让窗口用默认位置。
fn position_visible_on_any_monitor(app: &AppHandle, x: i64, y: i64, w: i64, h: i64) -> bool {
    // 至少要露出 40 像素宽 + 30 像素高才算"可见"
    let min_overlap_w = 40_i64;
    let min_overlap_h = 30_i64;
    let win_right = x + w;
    let win_bottom = y + h;

    let monitors = match app.available_monitors() {
        Ok(m) => m,
        Err(_) => return true, // 无法取显示器信息时，保守放行
    };
    // 早期启动时 available_monitors() 可能返回空（系统还没枚举完）→ 保守放行，
    // 避免把合法位置误判为 offscreen 导致用户的便签被重置到默认位置。
    if monitors.is_empty() {
        return true;
    }
    for mon in monitors {
        let scale = mon.scale_factor();
        let pos = mon.position();
        let size = mon.size();
        // 显示器坐标是物理像素；转换为逻辑像素（与 window.position 同单位）
        let mx = (pos.x as f64 / scale) as i64;
        let my = (pos.y as f64 / scale) as i64;
        let mw = (size.width as f64 / scale) as i64;
        let mh = (size.height as f64 / scale) as i64;
        let mon_right = mx + mw;
        let mon_bottom = my + mh;

        let overlap_w = win_right.min(mon_right) - x.max(mx);
        let overlap_h = win_bottom.min(mon_bottom) - y.max(my);
        if overlap_w >= min_overlap_w && overlap_h >= min_overlap_h {
            return true;
        }
    }
    false
}

/// 绑定 Move/Resize → 写回 DB。关键：Tauri 窗口 getter 返回 **物理像素**，
/// 但 WebviewWindowBuilder 接收 **逻辑像素**。保存前必须除以 scale_factor，
/// 否则 Retina 屏下 DB 存成 2× 大小，下次打开窗口变巨大/跑屏幕外。
fn attach_geometry_listener(window: &WebviewWindow, sticky_id: String) -> AppResult<()> {
    let app = window.app_handle().clone();
    let label = window.label().to_string();
    // Debounce：每次事件递增 seq；200ms 后若 seq 仍是最新则 commit。
    // 解决 macOS 拖动/resize 连续 Moved → 多个 async task 乱序完成 → 旧值覆盖新值的
    // race（症状：关掉便签再恢复后位置"乱跳"）。
    let seq = Arc::new(AtomicU64::new(0));
    // 窗口 build 后的前 1 秒内，macOS / tao 会先发 Moved(default) / Resized(default) 等
    // 中间过渡事件，把 builder 设好的位置/尺寸覆盖掉。忽略这段时间的事件。
    let startup_at = Instant::now();
    const STARTUP_IGNORE: Duration = Duration::from_millis(1000);
    window.on_window_event(move |event| {
        use tauri::WindowEvent;
        if startup_at.elapsed() < STARTUP_IGNORE {
            return;
        }
        let Some(win) = app.get_webview_window(&label) else { return; };
        let scale = win.scale_factor().unwrap_or(1.0);
        let (x, y, w, h) = match event {
            WindowEvent::Moved(pos) => {
                let logical_pos = pos.to_logical::<f64>(scale);
                let size = win.inner_size().ok().map(|s| s.to_logical::<f64>(scale));
                let (lw, lh) = size
                    .map(|s| (s.width as i64, s.height as i64))
                    .unwrap_or((320, 420));
                (logical_pos.x as i64, logical_pos.y as i64, lw, lh)
            }
            WindowEvent::Resized(size) => {
                let logical_size = size.to_logical::<f64>(scale);
                let pos = win.outer_position().ok().map(|p| p.to_logical::<f64>(scale));
                let (lx, ly) = pos
                    .map(|p| (p.x as i64, p.y as i64))
                    .unwrap_or((0, 0));
                (lx, ly, logical_size.width as i64, logical_size.height as i64)
            }
            _ => return,
        };
        // -32000 的"伪隐藏"位置：直接忽略
        if sanitize_position(x, y).is_none() {
            return;
        }
        // 再加一层保护：极小尺寸几乎肯定是建窗时的过渡值，不写回
        if w < 100 || h < 20 {
            return;
        }
        let my_seq = seq.fetch_add(1, Ordering::SeqCst) + 1;
        let seq_shared = seq.clone();
        let sticky_id = sticky_id.clone();
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(Duration::from_millis(200)).await;
            // 200ms 内如果有更新的事件进来，seq_shared > my_seq，就跳过这次
            if seq_shared.load(Ordering::SeqCst) != my_seq {
                return;
            }
            if let Some(db) = app.try_state::<Db>() {
                if let Ok(sticky) = db::stickies::get(&db, &sticky_id).await {
                    let patch = if sticky.collapsed == 1 {
                        db::stickies::StickyPatch {
                            x: Some(x),
                            y: Some(y),
                            collapsed_w: Some(w),
                            ..Default::default()
                        }
                    } else {
                        db::stickies::StickyPatch {
                            x: Some(x),
                            y: Some(y),
                            w: Some(w),
                            h: Some(h),
                            ..Default::default()
                        }
                    };
                    let _ = db::stickies::update(&db, &sticky_id, patch).await;
                }
            }
        });
    });
    Ok(())
}

#[cfg(test)]
mod binding_policy_tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    async fn test_pool() -> Db {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        crate::db::migrations::run(&pool).await.unwrap();
        pool
    }

    #[tokio::test]
    async fn pin_is_rejected_for_every_persisted_binding_state() {
        for state in [
            crate::assignment::BindingState::Active,
            crate::assignment::BindingState::TargetClosed,
            crate::assignment::BindingState::ReconnectRequired,
            crate::assignment::BindingState::Unavailable,
        ] {
            let db = test_pool().await;
            let sticky = db::stickies::create_default(&db).await.unwrap();
            db::window_bindings::bind(
                &db,
                db::window_bindings::NewWindowBinding {
                    sticky_id: sticky.id.clone(),
                    app_bundle_id: "com.example.Target".into(),
                    app_name: "Target".into(),
                    window_title: "Document".into(),
                    runtime_pid: 42,
                    runtime_window_id: 101,
                },
            )
            .await
            .unwrap();
            db::window_bindings::set_state_and_clear_runtime(&db, &sticky.id, state)
                .await
                .unwrap();

            let error = ensure_pin_is_user_managed(&db, &sticky.id)
                .await
                .unwrap_err();

            assert_eq!(
                error.to_string(),
                "pin is managed by the active window binding"
            );
        }
    }
}
