use std::collections::HashSet;
use std::sync::{Mutex, OnceLock};

use crate::assignment::BindingState;
use crate::db::{self, stickies::Sticky, Db};
use crate::error::{AppError, AppResult};
use async_trait::async_trait;
use tauri::{
    menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu},
    tray::TrayIconBuilder,
    AppHandle, Emitter, Manager,
};

/// sticky + 内容首行预览，用作菜单里的标题。
type StickyRow = (Sticky, Option<String>);

const ASSIGNED_ACTION_PREFIX: &str = "assigned";
const ASSIGNED_NOTE_TITLE_LIMIT: usize = 32;
const ASSIGNED_APP_NAME_LIMIT: usize = 24;
const ASSIGNED_WINDOW_TITLE_LIMIT: usize = 48;

static TRAY_REFRESH_LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
static PENDING_ASSIGNED_ACTIONS: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();

#[derive(Debug, Clone, PartialEq, Eq)]
struct AssignedNoteRow {
    sticky_id: String,
    note_display_title: String,
    app_name: String,
    window_title: String,
    state: BindingState,
    manually_hidden: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum AssignedPrimaryItem {
    Show { label: String },
    Status { label: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AssignedMenuItem {
    sticky_id: String,
    note_label: String,
    target_label: String,
    primary: AssignedPrimaryItem,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AssignedSectionModel {
    label: String,
    items: Vec<AssignedMenuItem>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AssignedActionKind {
    Show,
    Rebind,
    Unbind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AssignedMenuAction {
    kind: AssignedActionKind,
    sticky_id: String,
}

impl AssignedMenuAction {
    fn new(kind: AssignedActionKind, sticky_id: &str) -> Option<Self> {
        let parsed = ulid::Ulid::from_string(sticky_id).ok()?;
        if parsed.to_string() != sticky_id {
            return None;
        }
        Some(Self {
            kind,
            sticky_id: sticky_id.to_owned(),
        })
    }

    fn id(&self) -> String {
        let action = match self.kind {
            AssignedActionKind::Show => "show",
            AssignedActionKind::Rebind => "rebind",
            AssignedActionKind::Unbind => "unbind",
        };
        format!("{ASSIGNED_ACTION_PREFIX}:{action}:{}", self.sticky_id)
    }

    fn parse(id: &str) -> Option<Self> {
        let mut parts = id.split(':');
        if parts.next()? != ASSIGNED_ACTION_PREFIX {
            return None;
        }
        let kind = match parts.next()? {
            "show" => AssignedActionKind::Show,
            "rebind" => AssignedActionKind::Rebind,
            "unbind" => AssignedActionKind::Unbind,
            _ => return None,
        };
        let sticky_id = parts.next()?;
        if parts.next().is_some() {
            return None;
        }
        Self::new(kind, sticky_id)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct AssignedBindingSnapshot {
    state: BindingState,
    manually_hidden: bool,
}

fn assigned_menu_items(rows: &[AssignedNoteRow]) -> Vec<AssignedMenuItem> {
    rows.iter()
        .map(|row| {
            let note_label = truncate_menu_text(
                &row.note_display_title,
                ASSIGNED_NOTE_TITLE_LIMIT,
                "未命名便签",
            );
            let app_name = truncate_menu_text(&row.app_name, ASSIGNED_APP_NAME_LIMIT, "未知应用");
            let window_title =
                truncate_menu_text(&row.window_title, ASSIGNED_WINDOW_TITLE_LIMIT, "未命名窗口");
            let primary = match (row.state, row.manually_hidden) {
                (BindingState::Active, true) => AssignedPrimaryItem::Show {
                    label: "显示".into(),
                },
                (BindingState::Active, false) => AssignedPrimaryItem::Status {
                    label: format!("随 {app_name} 显示"),
                },
                (BindingState::TargetClosed, _) => AssignedPrimaryItem::Status {
                    label: "目标窗口已关闭".into(),
                },
                (BindingState::ReconnectRequired, _) => AssignedPrimaryItem::Status {
                    label: "需要重新绑定".into(),
                },
                (BindingState::Unavailable, _) => AssignedPrimaryItem::Status {
                    label: "目标窗口不可用".into(),
                },
            };
            AssignedMenuItem {
                sticky_id: row.sticky_id.clone(),
                note_label,
                target_label: format!("{app_name} · {window_title}"),
                primary,
            }
        })
        .collect()
}

fn assigned_section_model(rows: Option<&[AssignedNoteRow]>) -> AssignedSectionModel {
    match rows {
        Some(rows) => {
            let items = assigned_menu_items(rows);
            AssignedSectionModel {
                label: format!("已绑定 Notes（{}）", items.len()),
                items,
            }
        }
        None => AssignedSectionModel {
            label: "已绑定 Notes（读取失败）".into(),
            items: Vec::new(),
        },
    }
}

fn truncate_menu_text(value: &str, limit: usize, fallback: &str) -> String {
    let value = value.trim();
    if value.is_empty() {
        return fallback.to_owned();
    }
    let mut chars = value.chars();
    let prefix: String = chars.by_ref().take(limit).collect();
    if chars.next().is_some() {
        format!("{prefix}…")
    } else {
        prefix
    }
}

#[async_trait]
trait AssignedActionRuntime: Send + Sync {
    async fn binding_snapshot(&self, sticky_id: &str)
        -> AppResult<Option<AssignedBindingSnapshot>>;
    async fn show(&self, sticky_id: &str) -> AppResult<()>;
    async fn rebind(&self, sticky_id: &str) -> AppResult<()>;
    async fn unbind(&self, sticky_id: &str) -> AppResult<()>;
    async fn refresh(&self);
}

async fn dispatch_assigned_action(
    runtime: &dyn AssignedActionRuntime,
    action: &AssignedMenuAction,
) -> AppResult<()> {
    let result = async {
        match action.kind {
            AssignedActionKind::Show => match runtime.binding_snapshot(&action.sticky_id).await? {
                Some(AssignedBindingSnapshot {
                    state: BindingState::Active,
                    manually_hidden: true,
                }) => runtime.show(&action.sticky_id).await,
                Some(_) => Err(AppError::Other(
                    "bound note cannot be shown in its current state".into(),
                )),
                None => Err(AppError::NotFound),
            },
            AssignedActionKind::Rebind => {
                if runtime.binding_snapshot(&action.sticky_id).await?.is_none() {
                    Err(AppError::NotFound)
                } else {
                    runtime.rebind(&action.sticky_id).await
                }
            }
            // Always route unbind to the coordinator. A prior partial unbind may have
            // removed the DB row but still need a retry to restore runtime pinning.
            AssignedActionKind::Unbind => runtime.unbind(&action.sticky_id).await,
        }
    }
    .await;
    runtime.refresh().await;
    result
}

struct TrayAssignedActionRuntime {
    app: AppHandle,
}

#[async_trait]
impl AssignedActionRuntime for TrayAssignedActionRuntime {
    async fn binding_snapshot(
        &self,
        sticky_id: &str,
    ) -> AppResult<Option<AssignedBindingSnapshot>> {
        let db = self.app.state::<Db>();
        let Some(binding) = db::window_bindings::get(&db, sticky_id).await? else {
            return Ok(None);
        };
        let sticky = db::stickies::get(&db, sticky_id).await?;
        Ok(Some(AssignedBindingSnapshot {
            state: binding.state,
            manually_hidden: sticky.hidden == 1,
        }))
    }

    async fn show(&self, sticky_id: &str) -> AppResult<()> {
        let coordinator = self
            .app
            .try_state::<crate::assignment::coordinator::BindingCoordinator>()
            .ok_or_else(|| AppError::Other("window binding coordinator unavailable".into()))?;
        coordinator.reopen(sticky_id).await.map(|_| ())
    }

    async fn rebind(&self, sticky_id: &str) -> AppResult<()> {
        let db = self.app.state::<Db>();
        crate::commands::assignment::open_window_picker_for_sticky(&self.app, &db, sticky_id).await
    }

    async fn unbind(&self, sticky_id: &str) -> AppResult<()> {
        let coordinator = self
            .app
            .try_state::<crate::assignment::coordinator::BindingCoordinator>()
            .ok_or_else(|| AppError::Other("window binding coordinator unavailable".into()))?;
        coordinator.unbind(sticky_id).await
    }

    async fn refresh(&self) {
        refresh_menu(&self.app).await;
    }
}

struct PendingAssignedAction {
    sticky_id: String,
}

impl PendingAssignedAction {
    fn begin(sticky_id: &str) -> Option<Self> {
        let mut pending = PENDING_ASSIGNED_ACTIONS
            .get_or_init(|| Mutex::new(HashSet::new()))
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if !pending.insert(sticky_id.to_owned()) {
            return None;
        }
        Some(Self {
            sticky_id: sticky_id.to_owned(),
        })
    }
}

impl Drop for PendingAssignedAction {
    fn drop(&mut self) {
        PENDING_ASSIGNED_ACTIONS
            .get_or_init(|| Mutex::new(HashSet::new()))
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(&self.sticky_id);
    }
}

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
    let all = match db::stickies::list_all_unbound(db).await {
        Ok(all) => all,
        Err(error) => {
            eprintln!("[floaty] tray unbound-note query failed: {error}");
            return Vec::new();
        }
    };
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

async fn load_assigned_rows(db: &Db) -> AppResult<Vec<AssignedNoteRow>> {
    let bindings = db::window_bindings::list(db).await?;
    let mut rows = Vec::with_capacity(bindings.len());
    for binding in bindings {
        let sticky = db::stickies::get(db, &binding.sticky_id).await?;
        if sticky.deleted_at.is_some() {
            continue;
        }
        let preview = db::items::first_content(db, &sticky.id)
            .await
            .ok()
            .flatten()
            .and_then(|content| extract_title(&content));
        let note_display_title = if !sticky.title.trim().is_empty() {
            sticky.title.trim().to_owned()
        } else {
            preview.unwrap_or_else(|| "未命名便签".into())
        };
        rows.push(AssignedNoteRow {
            sticky_id: sticky.id,
            note_display_title,
            app_name: binding.app_name,
            window_title: binding.window_title,
            state: binding.state,
            manually_hidden: sticky.hidden == 1,
        });
    }
    Ok(rows)
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
    let (all, assigned, trashed) = tauri::async_runtime::block_on(async {
        (
            load_all_with_preview(&db).await,
            load_assigned_rows(&db).await,
            load_trashed_with_preview(&db).await,
        )
    });
    let menu = build_menu(app, &all, assigned.as_deref(), &trashed)?;
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
    let _refresh_guard = TRAY_REFRESH_LOCK
        .get_or_init(|| tokio::sync::Mutex::new(()))
        .lock()
        .await;
    let db = app.state::<Db>();
    let all = load_all_with_preview(&db).await;
    let assigned = load_assigned_rows(&db).await;
    let trashed = load_trashed_with_preview(&db).await;
    if let Some(tray) = app.tray_by_id("floaty-tray") {
        match build_menu(app, &all, assigned.as_deref(), &trashed) {
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
    assigned: Result<&[AssignedNoteRow], &AppError>,
    trashed: &[StickyRow],
) -> tauri::Result<Menu<tauri::Wry>> {
    let menu = Menu::new(app)?;
    let heading = MenuItem::with_id(
        app,
        "heading",
        format!("普通便签 ({})", all.len()),
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
        let item = MenuItem::with_id(app, format!("sticky:{}", s.id), label, true, None::<&str>)?;
        menu.append(&item)?;
    }

    let has_searchable_notes =
        !all.is_empty() || assigned.map(|rows| !rows.is_empty()).unwrap_or(true);
    if let Err(error) = assigned {
        eprintln!("[floaty] tray assigned-note query failed: {error}");
    }
    let assigned_section = assigned_section_model(assigned.ok());
    let assigned_submenu = Submenu::with_id(
        app,
        "assigned-notes",
        assigned_section.label,
        !assigned_section.items.is_empty(),
    )?;
    for item in assigned_section.items {
        let (Some(show_action), Some(rebind_action), Some(unbind_action)) = (
            AssignedMenuAction::new(AssignedActionKind::Show, &item.sticky_id),
            AssignedMenuAction::new(AssignedActionKind::Rebind, &item.sticky_id),
            AssignedMenuAction::new(AssignedActionKind::Unbind, &item.sticky_id),
        ) else {
            eprintln!(
                "[floaty] tray skipped assigned note with invalid sticky id: {}",
                item.sticky_id
            );
            continue;
        };
        let note_submenu = Submenu::with_id(
            app,
            format!("assigned-note:{}", item.sticky_id),
            item.note_label,
            true,
        )?;
        let target = MenuItem::with_id(
            app,
            format!("assigned-target:{}", item.sticky_id),
            item.target_label,
            false,
            None::<&str>,
        )?;
        note_submenu.append(&target)?;
        match item.primary {
            AssignedPrimaryItem::Show { label } => {
                let show = MenuItem::with_id(app, show_action.id(), label, true, None::<&str>)?;
                note_submenu.append(&show)?;
            }
            AssignedPrimaryItem::Status { label } => {
                let status = MenuItem::with_id(
                    app,
                    format!("assigned-status:{}", item.sticky_id),
                    label,
                    false,
                    None::<&str>,
                )?;
                note_submenu.append(&status)?;
            }
        }
        note_submenu.append(&PredefinedMenuItem::separator(app)?)?;
        let rebind = MenuItem::with_id(app, rebind_action.id(), "重新绑定", true, None::<&str>)?;
        note_submenu.append(&rebind)?;
        let unbind = MenuItem::with_id(app, unbind_action.id(), "取消绑定", true, None::<&str>)?;
        note_submenu.append(&unbind)?;
        assigned_submenu.append(&note_submenu)?;
    }
    menu.append(&assigned_submenu)?;

    menu.append(&PredefinedMenuItem::separator(app)?)?;

    let hidden_count = all.iter().filter(|(s, _)| s.hidden == 1).count();
    let label_text = if hidden_count > 0 {
        format!("显示全部（共 {} 张，{} 隐藏）", all.len(), hidden_count)
    } else {
        format!("显示全部（共 {} 张）", all.len())
    };
    let show_all_item =
        MenuItem::with_id(app, "show-all", label_text, !all.is_empty(), None::<&str>)?;
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
        has_searchable_notes,
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
        let empty = MenuItem::with_id(app, "trash-empty", "清空回收站", true, None::<&str>)?;
        trash_submenu.append(&empty)?;
    }
    menu.append(&trash_submenu)?;

    menu.append(&PredefinedMenuItem::separator(app)?)?;
    menu.append(&PredefinedMenuItem::quit(app, Some("退出"))?)?;
    Ok(menu)
}

// The production event handlers intentionally follow this focused view-model test
// module; moving the large existing test block would create a noisy, unrelated diff.
#[allow(clippy::items_after_test_module)]
#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    use super::*;
    use crate::db::window_bindings::NewWindowBinding;
    use sqlx::sqlite::SqlitePoolOptions;

    #[derive(Clone, Copy)]
    enum SnapshotMode {
        Value(Option<AssignedBindingSnapshot>),
        Error,
    }

    struct MockAssignedRuntime {
        snapshot: SnapshotMode,
        fail_action: Option<AssignedActionKind>,
        show_calls: AtomicUsize,
        rebind_calls: AtomicUsize,
        unbind_calls: AtomicUsize,
        refresh_calls: AtomicUsize,
        binding_present: AtomicBool,
        binding_present_at_refresh: Mutex<Vec<bool>>,
    }

    impl MockAssignedRuntime {
        fn new(snapshot: SnapshotMode) -> Self {
            Self {
                snapshot,
                fail_action: None,
                show_calls: AtomicUsize::new(0),
                rebind_calls: AtomicUsize::new(0),
                unbind_calls: AtomicUsize::new(0),
                refresh_calls: AtomicUsize::new(0),
                binding_present: AtomicBool::new(true),
                binding_present_at_refresh: Mutex::new(Vec::new()),
            }
        }

        fn failing(mut self, action: AssignedActionKind) -> Self {
            self.fail_action = Some(action);
            self
        }
    }

    #[async_trait]
    impl AssignedActionRuntime for MockAssignedRuntime {
        async fn binding_snapshot(
            &self,
            _sticky_id: &str,
        ) -> AppResult<Option<AssignedBindingSnapshot>> {
            match self.snapshot {
                SnapshotMode::Value(snapshot) => Ok(snapshot),
                SnapshotMode::Error => Err(AppError::Other("snapshot failed".into())),
            }
        }

        async fn show(&self, _sticky_id: &str) -> AppResult<()> {
            self.show_calls.fetch_add(1, Ordering::SeqCst);
            if self.fail_action == Some(AssignedActionKind::Show) {
                Err(AppError::Other("show failed".into()))
            } else {
                Ok(())
            }
        }

        async fn rebind(&self, _sticky_id: &str) -> AppResult<()> {
            self.rebind_calls.fetch_add(1, Ordering::SeqCst);
            if self.fail_action == Some(AssignedActionKind::Rebind) {
                Err(AppError::Other("rebind failed".into()))
            } else {
                Ok(())
            }
        }

        async fn unbind(&self, _sticky_id: &str) -> AppResult<()> {
            self.unbind_calls.fetch_add(1, Ordering::SeqCst);
            self.binding_present.store(false, Ordering::SeqCst);
            if self.fail_action == Some(AssignedActionKind::Unbind) {
                Err(AppError::Other("partial unbind".into()))
            } else {
                Ok(())
            }
        }

        async fn refresh(&self) {
            self.refresh_calls.fetch_add(1, Ordering::SeqCst);
            self.binding_present_at_refresh
                .lock()
                .unwrap()
                .push(self.binding_present.load(Ordering::SeqCst));
        }
    }

    fn assigned_row(
        sticky_id: &str,
        note_title: &str,
        app_name: &str,
        window_title: &str,
        state: BindingState,
        manually_hidden: bool,
    ) -> AssignedNoteRow {
        AssignedNoteRow {
            sticky_id: sticky_id.into(),
            note_display_title: note_title.into(),
            app_name: app_name.into(),
            window_title: window_title.into(),
            state,
            manually_hidden,
        }
    }

    fn action(kind: AssignedActionKind) -> AssignedMenuAction {
        AssignedMenuAction::new(kind, "01ARZ3NDEKTSV4RRFFQ69G5FAV").unwrap()
    }

    #[test]
    fn assigned_note_menu_reflects_active_hidden_visible_and_terminal_states() {
        let rows = vec![
            assigned_row(
                "s1",
                "Research",
                "Chrome",
                "Project",
                BindingState::Active,
                true,
            ),
            assigned_row(
                "s2",
                "Draft",
                "Chrome",
                "Project",
                BindingState::Active,
                false,
            ),
            assigned_row(
                "s3",
                "Meeting",
                "Zoom",
                "Room",
                BindingState::TargetClosed,
                false,
            ),
            assigned_row(
                "s4",
                "Plan",
                "Figma",
                "Board",
                BindingState::ReconnectRequired,
                true,
            ),
            assigned_row(
                "s5",
                "Inbox",
                "Mail",
                "Inbox",
                BindingState::Unavailable,
                false,
            ),
        ];

        let items = assigned_menu_items(&rows);

        assert_eq!(
            items
                .iter()
                .map(|item| item.primary.clone())
                .collect::<Vec<_>>(),
            vec![
                AssignedPrimaryItem::Show {
                    label: "显示".into()
                },
                AssignedPrimaryItem::Status {
                    label: "随 Chrome 显示".into()
                },
                AssignedPrimaryItem::Status {
                    label: "目标窗口已关闭".into()
                },
                AssignedPrimaryItem::Status {
                    label: "需要重新绑定".into()
                },
                AssignedPrimaryItem::Status {
                    label: "目标窗口不可用".into()
                },
            ]
        );
        assert_eq!(items[0].target_label, "Chrome · Project");
    }

    #[test]
    fn assigned_note_menu_handles_empty_long_and_unicode_labels() {
        assert!(assigned_menu_items(&[]).is_empty());
        let items = assigned_menu_items(&[
            assigned_row("s1", "", "", "", BindingState::Unavailable, false),
            assigned_row(
                "s2",
                &"研".repeat(80),
                &"浏".repeat(80),
                &"窗".repeat(100),
                BindingState::Active,
                false,
            ),
        ]);

        assert_eq!(items[0].note_label, "未命名便签");
        assert_eq!(items[0].target_label, "未知应用 · 未命名窗口");
        assert_eq!(
            items[1].note_label.chars().count(),
            ASSIGNED_NOTE_TITLE_LIMIT + 1
        );
        let (app, window) = items[1].target_label.split_once(" · ").unwrap();
        assert_eq!(app.chars().count(), ASSIGNED_APP_NAME_LIMIT + 1);
        assert_eq!(window.chars().count(), ASSIGNED_WINDOW_TITLE_LIMIT + 1);
        assert!(items[1].note_label.ends_with('…'));
    }

    #[test]
    fn assigned_note_menu_empty_and_query_failure_degrade_without_actions() {
        assert_eq!(
            assigned_section_model(Some(&[])),
            AssignedSectionModel {
                label: "已绑定 Notes（0）".into(),
                items: Vec::new(),
            }
        );
        assert_eq!(
            assigned_section_model(None),
            AssignedSectionModel {
                label: "已绑定 Notes（读取失败）".into(),
                items: Vec::new(),
            }
        );
    }

    #[test]
    fn assigned_note_menu_action_ids_are_exact_and_canonical() {
        for kind in [
            AssignedActionKind::Show,
            AssignedActionKind::Rebind,
            AssignedActionKind::Unbind,
        ] {
            let action = action(kind);
            assert_eq!(AssignedMenuAction::parse(&action.id()), Some(action));
        }
        for invalid in [
            "assigned:show:01arz3ndektsv4rrffq69g5fav",
            "assigned:show:01ARZ3NDEKTSV4RRFFQ69G5FAV:extra",
            "assigned:show:../sticky",
            "assigned:show:Research",
            "assigned:open:01ARZ3NDEKTSV4RRFFQ69G5FAV",
            "sticky:01ARZ3NDEKTSV4RRFFQ69G5FAV",
        ] {
            assert!(AssignedMenuAction::parse(invalid).is_none(), "{invalid}");
        }
    }

    #[tokio::test]
    async fn assigned_note_menu_blocks_show_for_visible_and_terminal_bindings() {
        for state in [
            BindingState::Active,
            BindingState::TargetClosed,
            BindingState::ReconnectRequired,
            BindingState::Unavailable,
        ] {
            let runtime =
                MockAssignedRuntime::new(SnapshotMode::Value(Some(AssignedBindingSnapshot {
                    state,
                    manually_hidden: false,
                })));
            assert!(
                dispatch_assigned_action(&runtime, &action(AssignedActionKind::Show))
                    .await
                    .is_err()
            );
            assert_eq!(runtime.show_calls.load(Ordering::SeqCst), 0);
            assert_eq!(runtime.refresh_calls.load(Ordering::SeqCst), 1);
        }
    }

    #[tokio::test]
    async fn assigned_note_menu_routes_show_rebind_and_unbind() {
        let hidden = AssignedBindingSnapshot {
            state: BindingState::Active,
            manually_hidden: true,
        };
        let show = MockAssignedRuntime::new(SnapshotMode::Value(Some(hidden)));
        dispatch_assigned_action(&show, &action(AssignedActionKind::Show))
            .await
            .unwrap();
        assert_eq!(show.show_calls.load(Ordering::SeqCst), 1);

        let rebind = MockAssignedRuntime::new(SnapshotMode::Value(Some(hidden)));
        dispatch_assigned_action(&rebind, &action(AssignedActionKind::Rebind))
            .await
            .unwrap();
        assert_eq!(rebind.rebind_calls.load(Ordering::SeqCst), 1);

        let unbind = MockAssignedRuntime::new(SnapshotMode::Value(None));
        dispatch_assigned_action(&unbind, &action(AssignedActionKind::Unbind))
            .await
            .unwrap();
        assert_eq!(unbind.unbind_calls.load(Ordering::SeqCst), 1);
        assert_eq!(unbind.refresh_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn assigned_note_menu_refreshes_after_snapshot_errors() {
        for kind in [AssignedActionKind::Show, AssignedActionKind::Rebind] {
            let runtime = MockAssignedRuntime::new(SnapshotMode::Error);
            assert!(dispatch_assigned_action(&runtime, &action(kind))
                .await
                .is_err());
            assert_eq!(runtime.refresh_calls.load(Ordering::SeqCst), 1);
        }
    }

    #[tokio::test]
    async fn assigned_note_menu_partial_unbind_refreshes_removed_db_state() {
        let runtime =
            MockAssignedRuntime::new(SnapshotMode::Value(None)).failing(AssignedActionKind::Unbind);

        assert!(
            dispatch_assigned_action(&runtime, &action(AssignedActionKind::Unbind))
                .await
                .is_err()
        );

        assert_eq!(runtime.unbind_calls.load(Ordering::SeqCst), 1);
        assert_eq!(runtime.refresh_calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            *runtime.binding_present_at_refresh.lock().unwrap(),
            vec![false]
        );
    }

    #[tokio::test]
    async fn assigned_note_menu_ordinary_query_excludes_every_persisted_binding() {
        let db = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        db::migrations::run(&db).await.unwrap();
        let unbound = db::stickies::create_default(&db).await.unwrap();
        let bound = db::stickies::create_default(&db).await.unwrap();
        db::window_bindings::bind(
            &db,
            NewWindowBinding {
                sticky_id: bound.id.clone(),
                app_bundle_id: "com.example.Target".into(),
                app_name: "Target".into(),
                window_title: "Document".into(),
                runtime_pid: 42,
                runtime_window_id: 101,
            },
        )
        .await
        .unwrap();

        let ordinary = load_all_with_preview(&db).await;

        assert_eq!(ordinary.len(), 1);
        assert_eq!(ordinary[0].0.id, unbound.id);
        assert_ne!(ordinary[0].0.id, bound.id);
    }

    #[test]
    fn every_binding_state_routes_to_the_coordinator_before_window_state() {
        for state in [
            BindingState::Active,
            BindingState::TargetClosed,
            BindingState::ReconnectRequired,
            BindingState::Unavailable,
        ] {
            for (hidden, has_window) in [(false, false), (false, true), (true, false), (true, true)]
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
        assert_eq!(
            extract_title("# 本周冲刺\n- 这里是内容"),
            Some("本周冲刺".into())
        );
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
        let long =
            "这是一段非常非常非常非常非常非常非常非常非常长的标题用来测试截断功能是否正常工作";
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
    if let Some(action) = AssignedMenuAction::parse(&id) {
        let Some(pending) = PendingAssignedAction::begin(&action.sticky_id) else {
            return;
        };
        let handle = app.clone();
        tauri::async_runtime::spawn(async move {
            let runtime = TrayAssignedActionRuntime { app: handle };
            if let Err(error) = dispatch_assigned_action(&runtime, &action).await {
                eprintln!(
                    "[floaty] assigned-note menu action failed for {}: {}",
                    action.sticky_id, error
                );
            }
            drop(pending);
        });
        return;
    }
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
                    let window = handle.get_webview_window(&crate::windows::label(&sticky_id));
                    match sticky_open_route(binding_state, s.hidden == 1, window.is_some()) {
                        StickyOpenRoute::BindingCoordinator => {
                            let result = match handle
                                .try_state::<crate::assignment::coordinator::BindingCoordinator>(
                            ) {
                                Some(coordinator) => {
                                    coordinator.reopen(&sticky_id).await.map(|_| ())
                                }
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
