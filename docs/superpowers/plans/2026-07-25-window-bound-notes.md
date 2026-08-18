# Window-Bound Notes Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让一条 Floaty Note 绑定到一个明确的 macOS 窗口实例，在保留自身坐标的同时进入目标窗口的层级与全屏 Space，并提供隐私友好的选择、恢复、重绑和解绑体验。

**Architecture:** 后端以独立的 Binding Registry 保存用户意图，以 macOS Window Catalog 发现可绑定窗口，以 Runtime Coordinator 管理目标窗口事件，再由 Window Space Backend 负责原生层级与 Space 附着。React 只通过类型化 IPC 消费窗口元数据和绑定状态；不调用屏幕捕获接口，也不把目标标题当成身份匹配依据。

**Tech Stack:** Tauri 2、Rust、Tokio、sqlx/SQLite、macOS Accessibility/ApplicationServices、AppKit/SkyLight 动态符号、React 19、TypeScript、Vitest、Testing Library。

---

## Scope gate

本功能横跨持久化、macOS 原生窗口、运行时协调和 UI，但它们共同交付一个不可拆分的用户闭环。Task 1 是硬性技术门槛：若无法在实机上稳定实现“只进入目标窗口 Space、目标全屏可见、其他 Space 不可见、Note 坐标不随目标移动”，立即停止后续任务并回到 PRD 讨论降级方案。不得用全局 `always_on_top` 冒充通过。

## File map

### New backend files

- `src-tauri/migrations/0008_window_bindings.sql` — 一条 Note 对一个目标窗口的持久化结构。
- `src-tauri/src/db/window_bindings.rs` — 绑定记录 CRUD、事务化重绑、启动失效。
- `src-tauri/src/assignment/mod.rs` — 绑定领域类型和公共入口。
- `src-tauri/src/assignment/state.rs` — 纯状态机；区分用户隐藏与目标窗口临时不可见。
- `src-tauri/src/assignment/coordinator.rs` — 运行时观察器、事件路由和 Note 显隐协调。
- `src-tauri/src/platform/mod.rs` — 平台分发。
- `src-tauri/src/platform/macos/mod.rs` — macOS 子模块。
- `src-tauri/src/platform/macos/accessibility.rs` — Accessibility 授权与 AX 观察器。
- `src-tauri/src/platform/macos/window_catalog.rs` — 窗口枚举、过滤和 App 图标元数据。
- `src-tauri/src/platform/macos/window_spaces.rs` — SkyLight/AppKit 附着、排序、Space 与全屏行为。
- `src-tauri/src/commands/assignment.rs` — 类型化 Tauri commands。

### New frontend files

- `src/assignment/WindowPickerPage.tsx` — 元数据卡片式选择器。
- `src/assignment/WindowPickerPage.test.tsx` — 选择器行为测试。
- `src/sticky/BindingButton.tsx` — Note 顶部栏绑定入口及状态菜单。
- `src/sticky/BindingButton.test.tsx` — 图标、禁用和操作测试。

### Modified files

- `src-tauri/Cargo.toml`, `src-tauri/Cargo.lock` — 仅加入 macOS 动态符号/系统桥接所需依赖。
- `src-tauri/src/db/migrations.rs`, `src-tauri/src/db/mod.rs` — 注册 migration 和 binding repository。
- `src-tauri/src/commands/mod.rs`, `src-tauri/src/lib.rs` — 注册 commands、平台状态和 coordinator。
- `src-tauri/src/commands/stickies.rs` — 软删除时清理绑定；恢复时保持未绑定。
- `src-tauri/src/windows.rs` — 增加运行时 suspend/resume，绑定时禁止全局置顶。
- `src-tauri/src/tray.rs` — “已绑定 Notes”子菜单和管理动作。
- `src-tauri/tauri.conf.json`, `src-tauri/capabilities/default.json` — 注册窗口选择器窗口与必要能力。
- `src/ipc/types.ts`, `src/ipc/client.ts` — 前后端绑定契约。
- `src/App.tsx` — 解析 `window-picker-*` 路由。
- `src/sticky/StickyPage.tsx` — 移除置顶文字 tag，接入 BindingButton。
- `docs/USER_GUIDE.md` — 权限、绑定、隐藏、失效和重连说明。

## Task 1: Prove native window/Space attachment

**Files:**

- Create: `src-tauri/src/platform/mod.rs`
- Create: `src-tauri/src/platform/macos/mod.rs`
- Create: `src-tauri/src/platform/macos/window_spaces.rs`
- Modify: `src-tauri/src/lib.rs`
- Modify: `src-tauri/Cargo.toml`

- [ ] **Step 1: Add a failing contract test for attachment results**

In `window_spaces.rs`, define a backend boundary that makes false success impossible:

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowAttachment {
    pub target_window_id: u32,
    pub note_window_id: u32,
    pub target_space_id: u64,
    pub note_space_id: u64,
}

impl WindowAttachment {
    pub fn is_same_space(&self) -> bool {
        self.target_space_id != 0 && self.target_space_id == self.note_space_id
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attachment_only_passes_for_the_same_nonzero_space() {
        assert!(WindowAttachment {
            target_window_id: 10,
            note_window_id: 20,
            target_space_id: 30,
            note_space_id: 30,
        }.is_same_space());
        assert!(!WindowAttachment {
            target_window_id: 10,
            note_window_id: 20,
            target_space_id: 30,
            note_space_id: 31,
        }.is_same_space());
    }
}
```

- [ ] **Step 2: Run the focused test and verify module registration is still missing**

Run: `cd src-tauri && cargo test window_spaces::tests::attachment_only_passes_for_the_same_nonzero_space`

Expected: FAIL because `platform`/`window_spaces` has not yet been registered.

- [ ] **Step 3: Register the platform module and implement a macOS-only probe**

Add `mod platform;` to the crate root. Add a macOS backend that dynamically resolves `SLSMainConnectionID`, `SLSCopySpacesForWindows`, `SLSMoveWindowsToManagedSpace`, and `SLSOrderWindow`; use AppKit to set the Floaty Note window collection behavior to `fullScreenAuxiliary`. The probe must:

1. receive a selected external `CGWindowID` and Floaty's `NSWindow.windowNumber`;
2. read both managed Space IDs;
3. move only the Floaty window to the target Space;
4. order the Floaty window immediately above the target;
5. read both Space IDs again and return `WindowAttachment`;
6. return `AppError::Other("native window-space attachment unavailable")` if any symbol or ID is unavailable.

The public boundary is:

```rust
pub trait WindowSpaceBackend: Send + Sync {
    fn attach(&self, target_window_id: u32, note_window_id: u32)
        -> crate::error::AppResult<WindowAttachment>;
    fn detach(&self, note_window_id: u32) -> crate::error::AppResult<()>;
    fn order_out(&self, note_window_id: u32) -> crate::error::AppResult<()>;
    fn order_front(&self, target_window_id: u32, note_window_id: u32)
        -> crate::error::AppResult<()>;
}

pub fn note_window_id(
    window: &tauri::WebviewWindow,
) -> crate::error::AppResult<u32>;
```

`note_window_id` obtains the underlying `NSWindow.windowNumber` through Tauri's macOS window handle. Keep all unsafe calls inside `window_spaces.rs`; no caller receives a raw Objective-C or Core Foundation pointer. Add `libloading = "0.8"` under the macOS target dependencies so private symbols are resolved at runtime.

- [ ] **Step 4: Run automated validation**

Run: `cd src-tauri && cargo test window_spaces`

Expected: PASS.

Run: `cd src-tauri && cargo check`

Expected: PASS without unresolved SkyLight symbols; dynamic loading must fail at runtime, not link time.

- [ ] **Step 5: Run the mandatory manual gate**

Run: `npm run tauri dev`.

Use a temporary debug-only command to select a visible Chrome/Safari window ID and attach one Note. Verify all four cases:

1. target normal window: Note is above the target and retains its original coordinates;
2. move/resize target: Note coordinates do not change;
3. target full screen: Note remains in that full-screen Space;
4. switch to another desktop/full-screen window: Note is absent there.

Expected: all four pass. Remove the debug-only command before committing. If any case fails, stop the implementation and record the exact failure in the PRD; do not proceed to Task 2.

- [ ] **Step 6: Commit the proven backend boundary**

```bash
git add src-tauri/Cargo.toml src-tauri/Cargo.lock src-tauri/src/lib.rs src-tauri/src/platform
git commit -m "spike: prove macOS window space attachment"
```

## Task 2: Add binding persistence

**Files:**

- Create: `src-tauri/migrations/0008_window_bindings.sql`
- Create: `src-tauri/src/db/window_bindings.rs`
- Modify: `src-tauri/src/db/migrations.rs`
- Modify: `src-tauri/src/db/mod.rs`
- Modify: `src-tauri/src/db/stickies.rs`

- [ ] **Step 1: Write failing repository tests**

Cover one binding per Note, transactional replacement, pin restoration data, startup invalidation, and soft-delete cleanup:

```rust
#[tokio::test]
async fn replacing_binding_keeps_one_row_and_original_pin() {
    let db = test_pool().await;
    let sticky = crate::db::stickies::create_default(&db).await.unwrap();
    sqlx::query("UPDATE stickies SET pinned = 1 WHERE id = ?")
        .bind(&sticky.id).execute(&db).await.unwrap();

    bind(&db, NewWindowBinding::test(&sticky.id, 101)).await.unwrap();
    bind(&db, NewWindowBinding::test(&sticky.id, 202)).await.unwrap();

    let saved = get(&db, &sticky.id).await.unwrap().unwrap();
    assert_eq!(saved.runtime_window_id, Some(202));
    assert_eq!(saved.prior_pinned, 1);
    let count: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM window_bindings WHERE sticky_id = ?"
    ).bind(&sticky.id).fetch_one(&db).await.unwrap();
    assert_eq!(count.0, 1);
}

#[tokio::test]
async fn startup_never_reuses_runtime_identity() {
    let db = test_pool().await;
    let sticky = crate::db::stickies::create_default(&db).await.unwrap();
    bind(&db, NewWindowBinding::test(&sticky.id, 101)).await.unwrap();

    mark_all_reconnect_required(&db).await.unwrap();
    let saved = get(&db, &sticky.id).await.unwrap().unwrap();
    assert_eq!(saved.state, BindingState::ReconnectRequired);
    assert_eq!(saved.runtime_pid, None);
    assert_eq!(saved.runtime_window_id, None);
}
```

- [ ] **Step 2: Run the repository tests and verify failure**

Run: `cd src-tauri && cargo test window_bindings`

Expected: FAIL because the migration and repository do not exist.

- [ ] **Step 3: Add migration 0008**

Use a separate table so transient window state does not inflate the existing `stickies` row:

```sql
CREATE TABLE window_bindings (
  sticky_id TEXT PRIMARY KEY REFERENCES stickies(id) ON DELETE CASCADE,
  app_bundle_id TEXT NOT NULL,
  app_name TEXT NOT NULL,
  window_title TEXT NOT NULL DEFAULT '',
  runtime_pid INTEGER,
  runtime_window_id INTEGER,
  state TEXT NOT NULL CHECK (
    state IN ('active', 'target_closed', 'reconnect_required', 'unavailable')
  ),
  prior_pinned INTEGER NOT NULL CHECK (prior_pinned IN (0, 1)),
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
);

CREATE INDEX idx_window_bindings_state ON window_bindings(state);
```

Register `0008_window_bindings` after `0007_collapse_w`.

- [ ] **Step 4: Implement repository operations**

Define `WindowBinding`, `NewWindowBinding`, and:

```rust
pub async fn bind(db: &Db, input: NewWindowBinding) -> AppResult<WindowBinding>;
pub async fn get(db: &Db, sticky_id: &str) -> AppResult<Option<WindowBinding>>;
pub async fn list(db: &Db) -> AppResult<Vec<WindowBinding>>;
pub async fn set_state(
    db: &Db,
    sticky_id: &str,
    state: BindingState,
) -> AppResult<WindowBinding>;
pub async fn mark_all_reconnect_required(db: &Db) -> AppResult<u64>;
pub async fn unbind(db: &Db, sticky_id: &str) -> AppResult<Option<WindowBinding>>;
pub async fn remove_for_deleted_sticky(db: &Db, sticky_id: &str) -> AppResult<()>;
```

`bind` starts a transaction, reads `prior_pinned` from an existing binding when rebinding or from `stickies.pinned` on first bind, sets `stickies.pinned = 0`, then upserts the binding. `unbind` deletes the row and restores `stickies.pinned = prior_pinned` in the same transaction. Soft delete removes the binding without restoring pin because a later trash restore must return as an ordinary unbound Note.

- [ ] **Step 5: Run repository and existing database tests**

Run: `cd src-tauri && cargo test window_bindings`

Expected: PASS.

Run: `cd src-tauri && cargo test db::stickies`

Expected: PASS.

- [ ] **Step 6: Commit persistence**

```bash
git add src-tauri/migrations/0008_window_bindings.sql src-tauri/src/db/migrations.rs src-tauri/src/db/mod.rs src-tauri/src/db/window_bindings.rs src-tauri/src/db/stickies.rs
git commit -m "feat: persist one window binding per note"
```

## Task 3: Implement the binding state machine

**Files:**

- Create: `src-tauri/src/assignment/mod.rs`
- Create: `src-tauri/src/assignment/state.rs`
- Modify: `src-tauri/src/lib.rs`

- [ ] **Step 1: Write failing state-transition tests**

```rust
#[test]
fn manual_hide_wins_over_target_restore() {
    let state = reduce(
        RuntimeState::Visible,
        BindingEvent::UserHidNote,
    );
    assert_eq!(state, RuntimeState::ManuallyHidden);
    assert_eq!(
        reduce(state, BindingEvent::TargetRestored),
        RuntimeState::ManuallyHidden
    );
}

#[test]
fn target_minimize_is_runtime_only() {
    assert_eq!(
        reduce(RuntimeState::Visible, BindingEvent::TargetMinimized),
        RuntimeState::Suspended
    );
    assert_eq!(
        reduce(RuntimeState::Suspended, BindingEvent::TargetRestored),
        RuntimeState::Visible
    );
}

#[test]
fn target_close_never_auto_reconnects() {
    assert_eq!(
        reduce(RuntimeState::Visible, BindingEvent::TargetClosed),
        RuntimeState::TargetClosed
    );
    assert_eq!(
        reduce(RuntimeState::TargetClosed, BindingEvent::SimilarWindowOpened),
        RuntimeState::TargetClosed
    );
}
```

- [ ] **Step 2: Run tests and verify failure**

Run: `cd src-tauri && cargo test assignment::state`

Expected: FAIL because the state machine does not exist.

- [ ] **Step 3: Implement pure domain types**

Create these serializable types in `assignment/mod.rs`:

```rust
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BindingState {
    Active,
    TargetClosed,
    ReconnectRequired,
    Unavailable,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeState {
    Visible,
    Suspended,
    ManuallyHidden,
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
```

Implement `reduce` as a total match. `UserOpenedNote` may leave `TargetClosed`, `ReconnectRequired`, or `Unavailable` unchanged; those states require rebind. No transition may infer a target from `SimilarWindowOpened`.

- [ ] **Step 4: Run tests**

Run: `cd src-tauri && cargo test assignment::state`

Expected: PASS.

- [ ] **Step 5: Commit the domain state machine**

```bash
git add src-tauri/src/assignment src-tauri/src/lib.rs
git commit -m "feat: define window binding lifecycle"
```

## Task 4: Implement privacy-preserving window discovery and permission gating

**Files:**

- Create: `src-tauri/src/platform/macos/accessibility.rs`
- Create: `src-tauri/src/platform/macos/window_catalog.rs`
- Modify: `src-tauri/src/platform/macos/mod.rs`
- Modify: `src-tauri/src/assignment/mod.rs`
- Modify: `src-tauri/Cargo.toml`

- [ ] **Step 1: Write failing eligibility tests around a platform-neutral descriptor**

```rust
#[test]
fn catalog_only_keeps_visible_normal_external_windows() {
    let windows = vec![
        candidate("app.floaty.desktop", "Floaty", 1, Role::Normal, true, false),
        candidate("com.google.Chrome", "Chrome", 2, Role::Normal, true, false),
        candidate("com.apple.finder", "Open", 3, Role::Dialog, true, false),
        candidate("com.tinyspeck.slackmacgap", "Slack", 4, Role::Normal, false, true),
    ];
    let eligible = filter_eligible(windows, "app.floaty.desktop");
    assert_eq!(
        eligible.iter().map(|w| w.window_id).collect::<Vec<_>>(),
        vec![2]
    );
}
```

- [ ] **Step 2: Run the test and verify failure**

Run: `cd src-tauri && cargo test window_catalog`

Expected: FAIL because catalog types and filtering do not exist.

- [ ] **Step 3: Implement Accessibility permission API**

Expose:

```rust
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AccessibilityStatus {
    Granted,
    Denied,
}

pub fn status() -> AccessibilityStatus;
pub fn request_with_system_prompt() -> AccessibilityStatus;
pub fn open_system_settings() -> crate::error::AppResult<()>;
```

Use `AXIsProcessTrustedWithOptions` only when the user explicitly requests authorization. Checking status must not show a prompt. `open_system_settings` opens the macOS Accessibility privacy pane. Do not reference ScreenCaptureKit, `CGWindowListCreateImage`, screenshots, OCR, or window pixel buffers.

- [ ] **Step 4: Implement window catalog**

Return:

```rust
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
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
```

Use Accessibility data for role, subrole, minimized state and title; use Core Graphics metadata only to obtain stable runtime `CGWindowID` and process ownership. Resolve the application icon from `NSRunningApplication`/`NSWorkspace`, encode only that icon as PNG data URL, and sort cards by front-to-back window order. Exclude Floaty, hidden/minimized windows, dialogs, sheets, popovers, utility panels, system-owned processes, and windows not currently on screen.

- [ ] **Step 5: Run focused and full backend checks**

Run: `cd src-tauri && cargo test window_catalog`

Expected: PASS.

Run: `cd src-tauri && cargo check`

Expected: PASS on macOS; non-macOS builds compile with an unsupported backend that returns an empty list and `AppError::Other("window binding is available on macOS only")`.

- [ ] **Step 6: Commit discovery and permission support**

```bash
git add src-tauri/Cargo.toml src-tauri/Cargo.lock src-tauri/src/assignment/mod.rs src-tauri/src/platform/macos
git commit -m "feat: discover eligible macOS windows privately"
```

## Task 5: Implement runtime coordination and native visibility

**Files:**

- Create: `src-tauri/src/assignment/coordinator.rs`
- Modify: `src-tauri/src/assignment/mod.rs`
- Modify: `src-tauri/src/platform/macos/accessibility.rs`
- Modify: `src-tauri/src/platform/macos/window_spaces.rs`
- Modify: `src-tauri/src/db/stickies.rs`
- Modify: `src-tauri/src/windows.rs`
- Modify: `src-tauri/src/lib.rs`

- [ ] **Step 1: Write coordinator tests with fake native backends**

```rust
#[tokio::test]
async fn minimizing_target_suspends_without_marking_note_hidden() {
    let harness = Harness::active_binding(false);
    harness.coordinator.handle(
        &harness.sticky_id,
        BindingEvent::TargetMinimized,
    ).await.unwrap();

    assert_eq!(harness.native.calls(), vec![NativeCall::OrderOut]);
    assert_eq!(harness.sticky().await.hidden, 0);
    assert_eq!(harness.runtime_state(), RuntimeState::Suspended);
}

#[tokio::test]
async fn manual_hide_prevents_restore() {
    let harness = Harness::active_binding(true);
    harness.coordinator.handle(
        &harness.sticky_id,
        BindingEvent::TargetRestored,
    ).await.unwrap();
    assert!(harness.native.calls().is_empty());
}

#[tokio::test]
async fn closing_target_preserves_binding_and_clears_runtime_ids() {
    let harness = Harness::active_binding(false);
    harness.coordinator.handle(
        &harness.sticky_id,
        BindingEvent::TargetClosed,
    ).await.unwrap();
    let binding = harness.binding().await.unwrap();
    assert_eq!(binding.state, BindingState::TargetClosed);
    assert_eq!(binding.runtime_window_id, None);
}

#[tokio::test]
async fn target_space_change_reattaches_without_changing_note_geometry() {
    let harness = Harness::active_binding(false);
    let original_geometry = harness.sticky_geometry().await;

    harness.coordinator.handle(
        &harness.sticky_id,
        BindingEvent::TargetSpaceChanged,
    ).await.unwrap();

    assert_eq!(harness.native.calls(), vec![NativeCall::Attach]);
    assert_eq!(harness.sticky_geometry().await, original_geometry);
}
```

- [ ] **Step 2: Run tests and verify failure**

Run: `cd src-tauri && cargo test assignment::coordinator`

Expected: FAIL because coordinator and fakes do not exist.

- [ ] **Step 3: Add runtime-only Note visibility operations**

In `windows.rs`, preserve the existing manual `hide`/`show` behavior and add distinct operations:

```rust
pub fn suspend_for_binding(
    app: &AppHandle,
    sticky_id: &str,
    spaces: &dyn WindowSpaceBackend,
) -> AppResult<()> {
    let window = app.get_webview_window(&label(sticky_id))
        .ok_or(AppError::NotFound)?;
    let note_window_id =
        crate::platform::macos::window_spaces::note_window_id(&window)?;
    spaces.order_out(note_window_id)
}

pub fn resume_for_binding(
    app: &AppHandle,
    sticky_id: &str,
    target_window_id: u32,
    spaces: &dyn WindowSpaceBackend,
) -> AppResult<()> {
    let window = app.get_webview_window(&label(sticky_id))
        .ok_or(AppError::NotFound)?;
    let note_window_id =
        crate::platform::macos::window_spaces::note_window_id(&window)?;
    spaces.order_front(target_window_id, note_window_id)
}
```

These functions must not write `stickies.hidden`, move the Note offscreen, focus Floaty, or change saved geometry. Manual close continues to set `hidden = 1`.

- [ ] **Step 4: Implement coordinator**

`BindingCoordinator` owns repository, window catalog, native Space backend, and active AX observers. It must:

- attach immediately after validated selection;
- subscribe to destroyed, minimized, restored, window-resized/full-screen, and permission-loss signals;
- on a target full-screen/Space transition, retry the same exact runtime window ID after transient Space lookup failures and call `attach` again once its new managed Space is available;
- keep target process/window identity only for the current Floaty runtime;
- derive target-close from a destroyed AX element, not from title disappearance;
- update the binding state and tray/header event after every durable transition;
- never react to target move/resize with Note geometry changes;
- never show a Note whose persisted `hidden == 1`;
- detach observers before rebind/unbind/delete.

Expose:

```rust
pub async fn bind(
    &self,
    app: &AppHandle,
    sticky_id: &str,
    target: WindowDescriptor,
) -> AppResult<WindowBindingView>;
pub async fn unbind(
    &self,
    app: &AppHandle,
    sticky_id: &str,
) -> AppResult<()>;
pub async fn reopen(
    &self,
    app: &AppHandle,
    sticky_id: &str,
) -> AppResult<WindowBindingView>;
pub async fn handle(
    &self,
    sticky_id: &str,
    event: BindingEvent,
) -> AppResult<()>;
```

- [ ] **Step 5: Make startup safe**

During setup, call `mark_all_reconnect_required` before `restore_on_startup`. Bound Notes with reconnect-required bindings must not be opened as ordinary floating windows even when `hidden == 0`; they remain discoverable through the tray.

Add `list_visible_unbound` with an explicit anti-join:

```rust
pub async fn list_visible_unbound(db: &Db) -> AppResult<Vec<Sticky>> {
    sqlx::query_as::<_, Sticky>(
        "SELECT s.* FROM stickies s
         WHERE s.hidden = 0
           AND s.deleted_at IS NULL
           AND NOT EXISTS (
             SELECT 1 FROM window_bindings b WHERE b.sticky_id = s.id
           )
         ORDER BY s.z_order ASC"
    ).fetch_all(db).await.map_err(Into::into)
}
```

Use this query in startup restoration. Do not modify the persisted `hidden` value merely to keep reconnect-required Notes offscreen.

- [ ] **Step 6: Run coordinator and regression tests**

Run: `cd src-tauri && cargo test assignment::coordinator`

Expected: PASS.

Run: `cd src-tauri && cargo test --lib`

Expected: all existing and new backend tests PASS.

- [ ] **Step 7: Commit runtime behavior**

```bash
git add src-tauri/src/assignment src-tauri/src/platform/macos src-tauri/src/windows.rs src-tauri/src/lib.rs
git commit -m "feat: coordinate bound note window lifecycle"
```

## Task 6: Expose typed IPC and integrate note lifecycle

**Files:**

- Create: `src-tauri/src/commands/assignment.rs`
- Modify: `src-tauri/src/commands/mod.rs`
- Modify: `src-tauri/src/commands/stickies.rs`
- Modify: `src-tauri/src/lib.rs`
- Modify: `src-tauri/src/windows.rs`
- Modify: `src-tauri/tauri.conf.json`
- Modify: `src-tauri/capabilities/default.json`
- Modify: `src/ipc/types.ts`
- Modify: `src/ipc/client.ts`

- [ ] **Step 1: Add failing TypeScript contract coverage**

Add compile-time contract types:

```ts
export type BindingState =
  | "active"
  | "target_closed"
  | "reconnect_required"
  | "unavailable";

export interface WindowDescriptor {
  appBundleId: string;
  appName: string;
  appIconDataUrl: string | null;
  pid: number;
  windowId: number;
  windowTitle: string;
}

export interface WindowBindingView {
  stickyId: string;
  appBundleId: string;
  appName: string;
  appIconDataUrl: string | null;
  windowTitle: string;
  state: BindingState;
  manuallyHidden: boolean;
}
```

Run: `npm run build`

Expected: FAIL until IPC wrappers and backend payloads match.

- [ ] **Step 2: Add commands**

Register:

```rust
get_accessibility_status
request_accessibility_permission
open_accessibility_settings
list_eligible_windows
get_window_binding
list_window_bindings
bind_note_to_window
unbind_note_from_window
reopen_bound_note
open_window_picker
```

`bind_note_to_window` receives the complete selected `WindowDescriptor` and revalidates `(pid, window_id)` through the catalog before modifying the database. `open_window_picker` creates a `window-picker-{sticky_id}` Tauri window sized for a compact 3-column card grid.

- [ ] **Step 3: Wire sticky delete and pin behavior**

Before soft delete, coordinator detaches and removes the binding. `toggle_pin` checks for a binding and returns `AppError::Other("pin is managed by the active window binding")` instead of changing state. `show_all` and `tile_all` skip bound Notes so these global actions cannot pull them into unrelated Spaces.

- [ ] **Step 4: Add frontend IPC wrappers**

```ts
getAccessibilityStatus: (): Promise<"granted" | "denied"> =>
  invoke("get_accessibility_status"),
listEligibleWindows: (): Promise<WindowDescriptor[]> =>
  invoke("list_eligible_windows"),
getWindowBinding: (stickyId: string): Promise<WindowBindingView | null> =>
  invoke("get_window_binding", { stickyId }),
bindNoteToWindow: (
  stickyId: string,
  target: WindowDescriptor,
): Promise<WindowBindingView> =>
  invoke("bind_note_to_window", { stickyId, target }),
unbindNoteFromWindow: (stickyId: string): Promise<void> =>
  invoke("unbind_note_from_window", { stickyId }),
reopenBoundNote: (stickyId: string): Promise<WindowBindingView> =>
  invoke("reopen_bound_note", { stickyId }),
openWindowPicker: (stickyId: string): Promise<void> =>
  invoke("open_window_picker", { stickyId }),
```

- [ ] **Step 5: Verify contracts**

Run: `npm run build`

Expected: PASS.

Run: `cd src-tauri && cargo test --lib`

Expected: PASS.

- [ ] **Step 6: Commit IPC integration**

```bash
git add src-tauri/src/commands src-tauri/src/lib.rs src-tauri/src/windows.rs src-tauri/tauri.conf.json src-tauri/capabilities/default.json src/ipc
git commit -m "feat: expose window binding commands"
```

## Task 7: Build the metadata-only window picker

**Files:**

- Create: `src/assignment/WindowPickerPage.tsx`
- Create: `src/assignment/WindowPickerPage.test.tsx`
- Modify: `src/App.tsx`
- Modify: `src/styles.css`

- [ ] **Step 1: Write failing interaction tests**

```tsx
it("renders app metadata without window image content", async () => {
  mockIpc.getAccessibilityStatus.mockResolvedValue("granted");
  mockIpc.listEligibleWindows.mockResolvedValue([chromeWindow]);
  render(<WindowPickerPage stickyId="sticky-1" />);

  expect(await screen.findByRole("button", {
    name: /Chrome.*Project Alpha/,
  })).toBeVisible();
  expect(screen.queryByRole("img", { name: /window preview/i })).toBeNull();
});

it("supports keyboard selection and escape", async () => {
  render(<WindowPickerPage stickyId="sticky-1" />);
  const card = await screen.findByRole("button", { name: /Chrome/ });
  card.focus();
  await userEvent.keyboard("{Enter}");
  expect(mockIpc.bindNoteToWindow).toHaveBeenCalledWith(
    "sticky-1",
    chromeWindow,
  );
});
```

Also test permission denied, no eligible windows, refresh, selection race failure, and Escape close.

- [ ] **Step 2: Run tests and verify failure**

Run: `npm test -- WindowPickerPage.test.tsx`

Expected: FAIL because the page does not exist.

- [ ] **Step 3: Implement picker states**

Implement four explicit states:

- permission denied: explanation plus “前往系统设置” and “重新检查”;
- loading: compact progress state;
- empty: “没有可绑定的窗口” plus refresh;
- ready: 3-column cards ordered by backend order.

Each card contains only app icon, app name, and truncated window title. The accessible name is `${appName}，${windowTitle || "未命名窗口"}`. Arrow keys move focus; Enter binds; Escape closes. A failed revalidation leaves the Note unbound, removes the stale card, and shows “该窗口已关闭，请重新选择”。

- [ ] **Step 4: Register the route**

Extend `Route` with:

```ts
| { kind: "window-picker"; stickyId: string }
```

Parse labels beginning `window-picker-` and render `WindowPickerPage`.

- [ ] **Step 5: Run picker tests and build**

Run: `npm test -- WindowPickerPage.test.tsx`

Expected: PASS.

Run: `npm run build`

Expected: PASS.

- [ ] **Step 6: Commit picker UI**

```bash
git add src/assignment src/App.tsx src/styles.css
git commit -m "feat: add private window picker"
```

## Task 8: Add the header binding control and simplify pin status

**Files:**

- Create: `src/sticky/BindingButton.tsx`
- Create: `src/sticky/BindingButton.test.tsx`
- Modify: `src/sticky/StickyPage.tsx`

- [ ] **Step 1: Write failing header tests**

```tsx
it("shows a generic bind icon for an unbound note", async () => {
  mockIpc.getWindowBinding.mockResolvedValue(null);
  render(<BindingButton stickyId="s1" />);
  await userEvent.click(await screen.findByRole("button", {
    name: "绑定到窗口",
  }));
  expect(mockIpc.openWindowPicker).toHaveBeenCalledWith("s1");
});

it("shows only the assigned app icon while bound", async () => {
  mockIpc.getWindowBinding.mockResolvedValue(activeChromeBinding);
  render(<BindingButton stickyId="s1" />);
  expect(await screen.findByRole("button", {
    name: /已绑定到 Chrome/,
  })).toBeVisible();
  expect(screen.queryByText("已绑定")).toBeNull();
});
```

Test rebind, unbind, unavailable label, and binding-changed event refresh.

- [ ] **Step 2: Run tests and verify failure**

Run: `npm test -- BindingButton.test.tsx`

Expected: FAIL because the component does not exist.

- [ ] **Step 3: Implement BindingButton**

Unbound state uses a generic window-link icon. Bound state uses only `appIconDataUrl`; when unavailable, add a small warning dot without text in the titlebar. Clicking the icon opens a compact menu containing read-only target app/window text plus “重新绑定” and “取消绑定”.

- [ ] **Step 4: Integrate the header**

Remove the existing `已置顶` tag entirely. Preserve the pin button's highlighted state when ordinary pinning is active. While bound, disable the pin button and set its tooltip to “已绑定到窗口时，置顶由绑定关系管理”. Insert `BindingButton` before the pin control so the newly freed titlebar space belongs to the binding feature.

- [ ] **Step 5: Run focused tests and frontend regression**

Run: `npm test -- BindingButton.test.tsx`

Expected: PASS.

Run: `npm test`

Expected: all frontend tests PASS.

Run: `npm run build`

Expected: PASS.

- [ ] **Step 6: Commit Note header behavior**

```bash
git add src/sticky/BindingButton.tsx src/sticky/BindingButton.test.tsx src/sticky/StickyPage.tsx
git commit -m "feat: manage window binding from note header"
```

## Task 9: Add the menu-bar Assigned Notes section

**Files:**

- Modify: `src-tauri/src/tray.rs`
- Modify: `src-tauri/src/commands/assignment.rs`

- [ ] **Step 1: Extract and test tray view-model construction**

```rust
#[test]
fn assigned_note_menu_reflects_hidden_and_unavailable_states() {
    let rows = vec![
        row("s1", "Research", "Chrome", BindingState::Active, true),
        row("s2", "Meeting", "Zoom", BindingState::TargetClosed, false),
    ];
    assert_eq!(
        assigned_menu_items(&rows),
        vec![
            AssignedMenuItem::Show {
                id: "s1".into(),
                label: "显示：Research · Chrome".into(),
            },
            AssignedMenuItem::Unavailable {
                id: "s2".into(),
                label: "Meeting · Zoom（目标窗口已关闭）".into(),
            },
        ]
    );
}
```

- [ ] **Step 2: Run the test and verify failure**

Run: `cd src-tauri && cargo test assigned_note_menu`

Expected: FAIL because the view model does not exist.

- [ ] **Step 3: Build the submenu**

Add an `已绑定 Notes` submenu after the ordinary Note list. Each Note gets a nested submenu:

- active + manually hidden: `显示`;
- active + visible: disabled status line;
- target closed/reconnect required/unavailable: disabled status line;
- every binding: `重新绑定` and `取消绑定`.

Menu events call coordinator operations or open the picker. Ordinary tray Note clicks continue to handle unbound Notes; they must not accidentally show an unavailable bound Note as a global floating window.

- [ ] **Step 4: Verify tray tests and backend suite**

Run: `cd src-tauri && cargo test assigned_note_menu`

Expected: PASS.

Run: `cd src-tauri && cargo test --lib`

Expected: PASS.

- [ ] **Step 5: Commit tray management**

```bash
git add src-tauri/src/tray.rs src-tauri/src/commands/assignment.rs
git commit -m "feat: manage assigned notes from menu bar"
```

## Task 10: Complete regression, privacy, and user documentation

**Files:**

- Modify: `docs/USER_GUIDE.md`
- Modify: `README.md`
- Test: all new and existing frontend/backend tests

- [ ] **Step 1: Add user-facing documentation**

Document:

- binding from the Note header;
- why Accessibility permission is required;
- explicit statement that Floaty does not request Screen Recording permission or capture window content;
- exact minimize, full-screen, Space, manual-hide, target-close, restart, rebind, and unbind behavior;
- bound Notes being macOS-only;
- pin state being temporarily managed while bound.

- [ ] **Step 2: Run static and automated verification**

Run: `npm test`

Expected: all frontend tests PASS.

Run: `npm run build`

Expected: TypeScript and Vite build PASS.

Run: `cd src-tauri && cargo fmt --check`

Expected: PASS.

Run: `cd src-tauri && cargo test --lib`

Expected: all backend tests PASS.

Run: `cd src-tauri && cargo clippy --all-targets -- -D warnings`

Expected: PASS with no warnings.

- [ ] **Step 3: Perform macOS acceptance matrix**

Manually verify:

1. two same-title Chrome windows remain distinguishable by runtime window ID;
2. picker displays metadata and app icons only;
3. binding a pinned Note removes global pin and unbind restores it;
4. target move/resize does not move/resize Note;
5. target full-screen and desktop switching isolate Note to the target Space;
6. target minimize suspends Note without changing user-hidden state;
7. manual hide persists across target restore and requires tray reopening;
8. target close becomes “目标窗口已关闭” without auto-match;
9. Floaty restart becomes “需要重新连接” without auto-match;
10. display disconnection restores an offscreen Note into a visible region;
11. Accessibility revocation produces a safe unavailable state;
12. `显示全部` and `一键排版` do not pull bound Notes into the current Space.

Expected: all twelve pass.

- [ ] **Step 4: Review the final diff for accidental scope**

Run: `git diff --check`

Expected: no whitespace errors.

Run: `git status --short`

Expected: only files named in this plan plus the already-existing user-owned lockfile/workspace changes. Do not stage unrelated `package-lock.json`, `pnpm-lock.yaml`, or `pnpm-workspace.yaml` changes unless this feature actually changed dependency management.

- [ ] **Step 5: Commit documentation and final verification**

```bash
git add README.md docs/USER_GUIDE.md
git commit -m "docs: explain window-bound notes"
```

## Self-review

- **Spec coverage:** Every Design PRD acceptance criterion maps to Tasks 4–10. The native full-screen/Space risk is tested before persistence or UI work in Task 1.
- **Prior-art correction:** The historical assignment branch used AppleScript polling, title fallback, position offsets, and automatic collapse. This plan deliberately excludes all four because the approved PRD requires exact runtime window identity, fixed Note coordinates, manual-hidden precedence, and no auto-match.
- **Privacy:** No task imports a screen-capture API or stores target pixels/content. Only app icons are encoded for UI display.
- **Type consistency:** `BindingState`, `WindowDescriptor`, and `WindowBindingView` use the same serialized names in Rust and TypeScript.
- **Lifecycle consistency:** Persistent `stickies.hidden` means user intent; automatic target minimize uses runtime `Suspended` and never writes `hidden`.
- **Placeholder scan:** The plan contains no deferred implementation markers. Every failure branch has a defined user-visible or domain result.
