# Design PRD — Window-Bound Notes

## Problem Statement

Floaty notes are currently independent floating windows. A note used for a task in a particular application can remain on screen after the user moves to another task, become lost among other notes, or require manual re-opening and placement. Users need a way to keep a note in the context of one specific work window without making it a global, distracting overlay.

The feature must bind a note to a *specific window instance*, not merely an application. For example, a note bound to one Chrome window must not attach itself to another Chrome window with the same title after the original closes.

## Solution & Value Proposition

Window-Bound Notes lets a user assign an existing Floaty note to one visible, normal macOS application window. The note becomes an auxiliary member of that window's work context:

- It keeps its own last screen position and size; it does not track the target window's movement or resizing.
- It appears above the target window, including when that window is full screen, but is not globally always-on-top.
- It remains in the target window's desktop or full-screen space and does not appear over unrelated apps or desktops.
- Its visibility follows the target window's availability unless the user has explicitly hidden the note.

The result is a lightweight, task-specific note that appears where it is useful and gets out of the way elsewhere.

## User Stories

1. As a Floaty user, I want to bind an existing note to a specific visible application window so that the note stays with my current task.
2. As a user with multiple Chrome windows, I want to choose the exact Chrome window so that a note never follows the wrong browser session.
3. As a user, I want the binding entry point in a note's header so that assigning a note starts from the object I already have in hand.
4. As a user, I want a compact card-based window picker showing app icons and window titles so that I can confidently select a window without granting screen-recording access.
5. As a privacy-conscious user, I want the picker to avoid showing other applications' contents so that Floaty does not read my screen.
6. As a user, I want only normal visible windows offered for binding so that I cannot accidentally bind to a transient dialog or system panel.
7. As a user, I want a bound note to remain at its own saved coordinates when I move or resize the target window so that my working layout is stable.
8. As a user, I want a bound note to be above the target window but behind unrelated foreground apps so that it does not distract me while I work elsewhere.
9. As a user working full screen, I want the bound note visible above the assigned full-screen window so that contextual information remains available.
10. As a user switching desktops or Spaces, I want the note to remain with the target window's Space so that it does not leak into other work contexts.
11. As a user, I want a target-window minimize operation to hide its bound notes and a restore operation to show notes that I have not manually hidden.
12. As a user, I want to manually hide a bound note without breaking its assignment so that I can declutter temporarily.
13. As a user who manually hid a note, I want it to remain hidden until I explicitly reopen it so that Floaty respects my choice.
14. As a user, I want a menu-bar list of bound notes so that I can reopen hidden notes and manage notes whose targets are unavailable.
15. As a user, I want a bound note's header to show only the assigned app icon so that its state is visible without wasting header space.
16. As a user, I want a tooltip or management menu on that icon to identify the target window and offer rebind or unbind actions.
17. As a user, I want a note to be bound to only one window at a time so that its behavior and location are predictable.
18. As a user, I want reassigning a note to replace the previous binding so that I can move the note to a new task without creating duplicates.
19. As a user, I want the existing pin state remembered when I bind a note so that unbinding restores my preference.
20. As a user, I want the pin control to be visibly active when pinned but not to consume header space with an “always on top” text tag.
21. As a user, I want binding to temporarily supersede global pinning so that a bound note cannot appear over unrelated applications.
22. As a user, I want a closed target window to leave the note intact but marked unavailable so that I never lose work or have Floaty silently bind it to a different window.
23. As a user, I want Floaty restart to mark existing bindings as needing reconnection rather than guessing which current window is the old target.
24. As a multi-monitor user, I want a note recovered from an unavailable display to move into a visible area so that it remains reachable.
25. As a user, I want a clear permission explanation before binding so that I understand why Floaty needs macOS Accessibility access.

## Interaction Framework & Information Architecture

### Entry points

The primary entry point is a binding icon in every note header. An unbound note opens the window picker. A bound note displays the assigned app's icon; clicking it opens a small management menu with the target name, **Rebind**, and **Unbind**.

The existing pin icon remains icon-only. It highlights when active. While a note is bound, pinning is controlled by the binding and the pin action is unavailable with an explanatory tooltip. The value that was active before binding is retained for restoration after unbinding.

The menu-bar tray contains an **Assigned Notes** section. It lists hidden bound notes and bindings whose target is unavailable. Choosing a hidden, active binding reopens the note above its target. Choosing an unavailable binding offers **Rebind** and **Unbind**.

### Bind flow

1. The user opens an existing note and selects its binding icon.
2. If Accessibility permission is missing, Floaty explains the requirement and directs the user to grant it. No window picker opens until permission is available.
3. Floaty opens a compact card panel containing only ordinary, visible, non-minimized windows on the current desktop, excluding Floaty and transient/system windows.
4. Each card shows an application icon, application name, and window title. It does not show captured window content.
5. The user selects one card. The panel closes, the note gains the app icon, and the note becomes an auxiliary window for that exact target window instance.
6. If the note had global pinning enabled, Floaty stores that preference and temporarily removes global pinning.

### Bound-window behavior

| Target state | Bound note behavior |
| --- | --- |
| Visible normal window | Visible above the target window, at the note's saved coordinates. |
| Target moves or resizes | Note does not move, resize, or scale. |
| Another app or desktop is foreground | Note remains in the target's window group/Space and is not visible over the unrelated context. |
| Target enters full screen | Note appears above the target in that full-screen Space. |
| Target minimizes | Note hides, unless it was already manually hidden. |
| Target restores | Note reappears only if it was not manually hidden. |
| User hides note | Association remains; note stays hidden until user explicitly opens it from the menu bar. |
| Target closes | Note remains intact and hidden; binding shows “target window closed.” |
| Floaty restarts | Binding shows “reconnection required”; Floaty does not auto-match a window. |

### Rebind and unbind

**Rebind** runs the same picker and replaces the previous target when the user chooses a new window. **Unbind** removes the association, restores the pre-binding pin preference, and returns the note to normal Floaty behavior. Deleting a note also removes its binding.

## UI/UX Edge Cases

- **No eligible windows:** Show an empty picker state explaining that Floaty found no visible normal windows. Provide a refresh action and dismiss action.
- **Accessibility permission denied or revoked:** Disable binding actions, retain any existing association as unavailable, explain the permission need, and provide a route to System Settings. Do not request screen-recording permission.
- **Window disappears while picker is open:** Disable or remove its card. If selection races with closure, keep the note unbound and show a brief non-blocking message.
- **Target title is missing or changes:** Use the app name and icon as the primary label. A title change does not alter the current runtime binding.
- **Target is minimized, closed, or changes Space:** The picker excludes it at selection time. An existing binding transitions to the relevant hidden or unavailable state without deleting the note.
- **Target refuses Accessibility inspection:** Treat it as ineligible in the picker; do not expose incomplete or system-owned UI.
- **Manual hide:** Manual hiding takes priority over target restore. The note does not reappear because the target gains focus, is restored, or re-enters full screen.
- **Several notes bound to one target:** All non-manually-hidden notes restore at their independent last positions. Floaty does not auto-arrange or stack them in this release.
- **Display removed:** Preserve coordinates when valid; if the saved rectangle is no longer visible, bring the note into a visible area on the primary display.
- **Binding target closed:** Preserve a visible unavailable state in the menu bar and header management affordance. Do not silently unbind, delete, or bind to a newly opened window.
- **Floaty restart:** Do not attempt identity recovery. Mark binding reconnect-required, even if a similar application window is currently visible.

## Accessibility & Visual Constraints

- macOS-only behavior is acceptable. The project prioritizes local, open-source macOS functionality over App Store compatibility.
- The picker must be fully keyboard accessible: focus opens predictably, cards have accessible names composed from app and window title, arrow keys navigate, Enter selects, and Escape dismisses.
- Header controls require accessible labels and tooltips. The assigned-app icon exposes the target description and management actions to assistive technologies.
- Use icon-only persistent status in the compact note header. The pin icon uses a high-contrast active state but never renders a “pinned” tag. The assigned-app icon is the only persistent binding marker.
- Window cards use app icon, title, and clear focus/selection states; they must not imply a live screenshot.
- No external window pixels, text contents, URLs, or document contents are collected, stored, or rendered by this feature.
- Respect Reduce Motion and avoid transition effects that obscure whether a note is visible, hidden, or unavailable.

## Acceptance Criteria

1. A user can bind an existing note to one eligible visible window from the note header.
2. The picker never requires or requests screen-recording permission and never displays application content.
3. Binding a note replaces global always-on-top behavior and preserves the prior pin preference for later restoration.
4. A bound note keeps its own coordinates while remaining associated with the target's window context and full-screen Space.
5. Bound notes are not visible over unrelated applications or desktops.
6. Minimize, restore, manual hide, window close, Floaty restart, and unavailable-display cases behave exactly as defined above.
7. Users can reopen hidden bound notes and rebind or unbind unavailable notes from the menu bar.
