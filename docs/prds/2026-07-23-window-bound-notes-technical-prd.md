# Technical PRD — Window-Bound Notes

## Architecture & Deep Modules

### 1. macOS Window Discovery

Provides a privacy-preserving list of eligible external windows for the binding picker. It owns Accessibility authorization, enumerates normal visible windows, and exposes only the metadata needed for selection: application identity, process identity, runtime window identity, display label, and title. It excludes Floaty, minimized windows, transient dialogs, system panels, and inaccessible windows.

### 2. Binding Registry

Owns the one-to-one relationship between a Floaty note and one target window instance. It persists user intent and user-visible target metadata, tracks availability state, preserves the note's prior pin value, and guarantees that a new bind replaces the previous relationship atomically. It does not infer that a different or later window is the prior target.

### 3. Runtime Binding Coordinator

Owns the lifecycle of active bindings. It observes target-window accessibility events, translates those events into a small binding state machine, and asks the window-space controller to show, hide, attach, or mark a note unavailable. It isolates per-application observers so that event failures in one external application do not disrupt other Floaty notes.

### 4. macOS Window-Space Controller

Owns native behavior for Floaty note windows: non-global ordering above the target, inclusion in the target's full-screen Space, minimize/restore visibility, and removal of global always-on-top while bound. It does not change a note's stored coordinates when the target moves or resizes. This module is explicitly macOS-specific and is responsible for safely degrading a binding to unavailable if native attachment cannot be maintained.

### 5. Binding UI and Tray Management

Owns the note-header binding control, the metadata-only window-picker panel, empty/permission/error states, the assigned app icon, and the menu-bar Assigned Notes section. It consumes typed data from the registry and discovery modules; it does not perform macOS Accessibility calls directly.

### 6. Permission Gate

Owns checking and requesting macOS Accessibility permission, presentation of the user explanation, and transition back into the bind flow after authorization. It explicitly does not request or depend on screen-recording permission.

## Implementation Decisions

### Platform boundary

This release targets macOS native behavior only. It may use macOS Accessibility and native window-space APIs appropriate for an open-source, locally deployed application. App Store compatibility and cross-platform parity are not requirements.

### Target identity and binding lifecycle

- A binding identifies one live external window instance, not an application or title pattern.
- Runtime identity includes the target process and a window identity supplied by the Accessibility/windowing layer. It is valid only for the active Floaty process lifetime.
- Persisted data includes user-visible target metadata (application bundle identity/name, app icon reference when available, last title) only for display and management. It is never used to auto-match a new target.
- On target close, the binding becomes `target_closed` and the note hides. The relationship remains until the user rebinds, unbinds, or deletes the note.
- On Floaty launch, all surviving persisted bindings become `reconnect_required`. They are not reactivated automatically.
- On a successful rebind, the existing record is replaced with the new target and returns to `active`.

### Data model

Add a persistent binding record with one row per bound note. The record includes:

- note identifier as the unique owner of the association;
- target app metadata for UI display;
- transient process/window identifiers for the active runtime session;
- binding availability state (`active`, `target_closed`, `reconnect_required`, or `unavailable`);
- the pre-binding global pin preference; and
- timestamps for auditing/update ordering.

The note model remains the source of truth for the note's independent position, dimensions, hidden state, deletion state, and current pin preference. Removing a note must remove its binding record. Rebinding must be transactional so no note can have two active targets.

### Visibility and pin policy

- A binding is an auxiliary window-space association, not an application-wide focus rule and not a global always-on-top flag.
- During an active binding, the window-space controller prevents global always-on-top. The original user preference is retained by the binding record.
- On unbind, restore the retained pin preference. On note deletion, no restoration is needed.
- Manual note hide persists as the note's user-controlled hidden state. It wins over automatic visibility caused by target restore.
- Target minimize hides active bound notes without converting them into user-hidden notes. Target restore reveals only notes not manually hidden.
- A position validation pass runs before revealing a note; if its stored rectangle is off every current display, position it in a visible primary-display region without otherwise changing its saved layout policy.

### Permissions and privacy

- Accessibility permission is the only requested system permission for this feature. It is required before target discovery or event observation.
- The app must not invoke screen-capture APIs, request Screen Recording permission, capture thumbnails, store screenshots, or send window content to any service.
- The picker receives only metadata necessary to choose a window. Application content, URLs, document text, and pixels are out of scope.

### Typed boundaries and data flow

1. The UI requests a list of eligible windows from discovery after the permission gate approves access.
2. The user selects a window descriptor; the registry persists the replacement binding and gives the coordinator an active runtime target.
3. The coordinator starts observation for that target and asks the window-space controller to apply its native state.
4. Native events update the coordinator; it updates binding availability and note visibility through the registry/window controller.
5. The header and tray read binding state through an IPC contract and render status, rebind, unbind, and reopen actions.
6. Startup migration marks previous live bindings reconnect-required before any note is restored.

The IPC contract should expose operations to list eligible windows, bind, rebind, unbind, list assigned notes, reopen a manually hidden assigned note, and query binding status. It should return domain-level outcomes such as permission-required, no-eligible-window, target-unavailable, and native-attachment-failed rather than raw macOS errors.

## Testing Decisions

### General principles

Tests verify externally observable behavior and state transitions, not native implementation details. Native macOS calls sit behind small interfaces so module tests can use deterministic fakes. No test fixture contains real external window content.

### Unit tests

- Binding registry: one binding per note; rebind replaces the old target; unbind restores prior pin state; note delete removes binding; startup converts active persisted records to reconnect-required.
- Visibility state machine: manual hide wins over target restore; minimize and restore preserve manual-hide choice; target close yields unavailable without unbinding; no automatic matching happens after close or restart.
- Eligibility filter: excludes Floaty, minimized, transient, system-owned, inaccessible, and non-current-Space windows; preserves normal visible application windows.
- Position recovery: valid saved coordinates remain untouched; an off-screen note is restored to a visible display region.
- Permission gate: absence or revocation of Accessibility access returns a clear blocked state and never initiates screen capture.
- Tray/header view models: assigned icon state, unavailable labels, and menu actions map correctly from binding state.

### Integration tests

- Database migrations add binding persistence without affecting existing note, item, reminder, soft-delete, or hidden-note behavior.
- IPC requests preserve typed success and failure outcomes across the frontend/backend boundary.
- Binding a pinned note disables global pinning for the duration of the binding and restores it on unbind.
- Hiding and reopening a bound note via the menu bar retains its assignment and saved geometry.

### Manual macOS acceptance tests

- Bind notes to two separately titled and identically titled windows of the same app; confirm the selected window alone owns each note.
- Move and resize the target; confirm the note's screen coordinates do not change.
- Switch between unrelated applications, desktops, and a full-screen target; confirm the note appears only in the selected target's context.
- Minimize/restore, close, and reopen an external target; confirm there is no new-window auto-match.
- Restart Floaty with a target still open; confirm reconnect-required rather than silent restoration.
- Revoke Accessibility permission while bindings exist; confirm safe unavailable presentation and no crash.
- Disconnect an external display containing a note; confirm recovery to a visible region.

## Out of Scope

- Binding a note to more than one window at once.
- Binding by application name, title matching, URL, document, browser tab, or any heuristic that could auto-attach a different window.
- Screen recording, live thumbnails, screenshots, OCR, window-content indexing, or any persistence of external-app content.
- Windows, Linux, mobile, and web-platform behavior.
- App Store compatibility requirements.
- Automatic layout, collision avoidance, stacking, or a note shelf for multiple notes bound to the same window.
- Synchronizing a note's position or dimensions with the target window's geometry.
- Automatically reopening a manually hidden note when its target becomes active again.
- Restoring a binding automatically after Floaty restart or target-window closure.

## Further Notes

- The existing note-window model already separates note windows, persisted geometry, manual hidden state, tray access, and global pinning. The feature should extend those concepts rather than replace them.
- Native full-screen Space behavior is the primary technical risk. It must be proven with a small macOS spike before relying on it for the release. If an arbitrary external window cannot retain Floaty's note in the desired Space, the product must surface a clear unavailable/unsupported state rather than fall back to global always-on-top.
- The header's current pin status treatment should be simplified as part of this feature: icon highlight only, no pin text tag. This is a direct visual requirement, not a general header redesign.
