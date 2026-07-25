import { invoke } from "@tauri-apps/api/core";
import type {
  AccessibilityStatus,
  Item,
  ItemUpsert,
  Sticky,
  StickyPatch,
  WindowBindingView,
  WindowDescriptor,
} from "./types";

export const ipc = {
  getOrCreateDefaultSticky: (): Promise<Sticky> =>
    invoke("get_or_create_default_sticky"),

  listStickies: (): Promise<Sticky[]> =>
    invoke("list_stickies"),

  listAllStickies: (): Promise<Sticky[]> =>
    invoke("list_all_stickies"),

  getSticky: (id: string): Promise<Sticky> =>
    invoke("get_sticky", { id }),

  createSticky: (): Promise<Sticky> =>
    invoke("create_sticky"),

  updateSticky: (id: string, patch: StickyPatch): Promise<Sticky> =>
    invoke("update_sticky", { id, patch }),

  deleteSticky: (id: string): Promise<void> =>
    invoke("delete_sticky", { id }),

  listItems: (stickyId: string): Promise<Item[]> =>
    invoke("list_items", { stickyId }),

  upsertItem: (input: ItemUpsert): Promise<Item> =>
    invoke("upsert_item", { input }),

  toggleItem: (id: string): Promise<Item> =>
    invoke("toggle_item", { id }),

  deleteItem: (id: string): Promise<void> =>
    invoke("delete_item", { id }),

  openStickyWindow: (stickyId: string): Promise<void> =>
    invoke("open_sticky_window", { stickyId }),

  hideSticky: (stickyId: string): Promise<void> =>
    invoke("hide_sticky", { stickyId }),

  showSticky: (stickyId: string): Promise<void> =>
    invoke("show_sticky", { stickyId }),

  togglePin: (stickyId: string): Promise<boolean> =>
    invoke("toggle_pin", { stickyId }),

  getAccessibilityStatus: (): Promise<AccessibilityStatus> =>
    invoke("get_accessibility_status"),

  requestAccessibilityPermission: (): Promise<AccessibilityStatus> =>
    invoke("request_accessibility_permission"),

  openAccessibilitySettings: (): Promise<void> =>
    invoke("open_accessibility_settings"),

  listEligibleWindows: (): Promise<WindowDescriptor[]> =>
    invoke("list_eligible_windows"),

  getWindowBinding: (stickyId: string): Promise<WindowBindingView | null> =>
    invoke("get_window_binding", { stickyId }),

  listWindowBindings: (): Promise<WindowBindingView[]> =>
    invoke("list_window_bindings"),

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

  newStickyWindow: (): Promise<string> =>
    invoke("new_sticky_window"),

  syncReminders: (
    stickyId: string,
    entries: { item_index: number; text_preview: string; fire_at: number }[],
  ): Promise<void> => invoke("sync_reminders", { stickyId, entries }),

  snoozeReminder: (id: string, minutes: number): Promise<void> =>
    invoke("snooze_reminder", { id, minutes }),
};
