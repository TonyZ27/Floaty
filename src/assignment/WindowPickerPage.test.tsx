import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import App, { parseWindowLabel } from "../App";
import type { WindowDescriptor } from "../ipc/types";
import { WindowPickerPage } from "./WindowPickerPage";

const mocks = vi.hoisted(() => ({
  close: vi.fn(),
  getCurrentWindow: vi.fn(),
  ipc: {
    getAccessibilityStatus: vi.fn(),
    openAccessibilitySettings: vi.fn(),
    listEligibleWindows: vi.fn(),
    bindNoteToWindow: vi.fn(),
  },
}));

vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: mocks.getCurrentWindow,
}));

vi.mock("../ipc/client", () => ({
  ipc: mocks.ipc,
}));

const STICKY_ID = "01ARZ3NDEKTSV4RRFFQ69G5FAV";

function descriptor(
  appName: string,
  windowTitle: string,
  windowId: number,
  appIconDataUrl: string | null = "data:image/png;base64,aWNvbg==",
): WindowDescriptor {
  return {
    appBundleId: `com.example.${appName.toLowerCase()}`,
    appName,
    appIconDataUrl,
    pid: 100 + windowId,
    windowId,
    windowTitle,
  };
}

const chromeWindow = descriptor("Chrome", "Project Alpha", 1);
const figmaWindow = descriptor("Figma", "Design system", 2);

beforeEach(() => {
  vi.clearAllMocks();
  mocks.getCurrentWindow.mockReturnValue({
    label: `window-picker-${STICKY_ID}`,
    close: mocks.close,
  });
  mocks.ipc.getAccessibilityStatus.mockResolvedValue("granted");
  mocks.ipc.openAccessibilitySettings.mockResolvedValue(undefined);
  mocks.ipc.listEligibleWindows.mockResolvedValue([chromeWindow]);
  mocks.ipc.bindNoteToWindow.mockResolvedValue({});
  mocks.close.mockResolvedValue(undefined);
});

describe("WindowPickerPage", () => {
  it("renders app metadata and a fallback icon without window image content", async () => {
    mocks.ipc.listEligibleWindows.mockResolvedValue([
      chromeWindow,
      descriptor("Terminal", "", 3, null),
    ]);

    render(<WindowPickerPage stickyId={STICKY_ID} />);

    expect(
      await screen.findByRole("button", {
        name: "Chrome，Project Alpha",
      }),
    ).toBeVisible();
    expect(
      screen.getByRole("button", {
        name: "Terminal，未命名窗口",
      }),
    ).toBeVisible();
    expect(screen.queryByRole("img", { name: /window preview/i })).toBeNull();
    expect(document.querySelectorAll(".window-picker-app-icon")).toHaveLength(1);
    expect(screen.getByText("T", { selector: ".window-picker-icon-fallback" })).toBeVisible();
  });

  it("keeps opening settings separate from rechecking Accessibility status", async () => {
    mocks.ipc.getAccessibilityStatus
      .mockResolvedValueOnce("denied")
      .mockResolvedValueOnce("granted");
    mocks.ipc.listEligibleWindows.mockResolvedValue([chromeWindow]);

    render(<WindowPickerPage stickyId={STICKY_ID} />);

    expect(await screen.findByText("需要辅助功能权限")).toBeVisible();
    fireEvent.click(screen.getByRole("button", { name: "前往系统设置" }));
    await waitFor(() => {
      expect(mocks.ipc.openAccessibilitySettings).toHaveBeenCalledTimes(1);
    });
    expect(mocks.ipc.getAccessibilityStatus).toHaveBeenCalledTimes(1);

    fireEvent.click(screen.getByRole("button", { name: "重新检查" }));
    expect(
      await screen.findByRole("button", { name: "Chrome，Project Alpha" }),
    ).toBeVisible();
    expect(mocks.ipc.getAccessibilityStatus).toHaveBeenCalledTimes(2);
  });

  it("shows loading, then an empty state that can be refreshed", async () => {
    let resolveStatus: ((status: "granted") => void) | undefined;
    mocks.ipc.getAccessibilityStatus.mockReturnValueOnce(
      new Promise((resolve) => {
        resolveStatus = resolve;
      }),
    );
    mocks.ipc.listEligibleWindows
      .mockResolvedValueOnce([])
      .mockResolvedValueOnce([chromeWindow]);

    render(<WindowPickerPage stickyId={STICKY_ID} />);

    expect(screen.getByRole("status")).toHaveTextContent("正在查找可绑定窗口");
    resolveStatus?.("granted");
    expect(await screen.findByText("没有可绑定的窗口")).toBeVisible();

    fireEvent.click(screen.getByRole("button", { name: "刷新" }));
    expect(
      await screen.findByRole("button", { name: "Chrome，Project Alpha" }),
    ).toBeVisible();
    expect(mocks.ipc.listEligibleWindows).toHaveBeenCalledTimes(2);
  });

  it("moves through the real three-column grid and selects with Enter", async () => {
    const windows = [
      chromeWindow,
      figmaWindow,
      descriptor("Terminal", "Shell", 3),
      descriptor("Safari", "Reference", 4),
      descriptor("Notes", "Ideas", 5),
    ];
    mocks.ipc.listEligibleWindows.mockResolvedValue(windows);

    render(<WindowPickerPage stickyId={STICKY_ID} />);

    const first = await screen.findByRole("button", { name: "Chrome，Project Alpha" });
    const second = screen.getByRole("button", { name: "Figma，Design system" });
    const third = screen.getByRole("button", { name: "Terminal，Shell" });
    const fifth = screen.getByRole("button", { name: "Notes，Ideas" });
    await waitFor(() => expect(first).toHaveFocus());

    fireEvent.keyDown(first, { key: "ArrowLeft" });
    expect(first).toHaveFocus();
    fireEvent.keyDown(first, { key: "ArrowRight" });
    expect(second).toHaveFocus();
    fireEvent.keyDown(second, { key: "ArrowRight" });
    expect(third).toHaveFocus();
    fireEvent.keyDown(third, { key: "ArrowRight" });
    expect(third).toHaveFocus();
    fireEvent.keyDown(third, { key: "ArrowLeft" });
    expect(second).toHaveFocus();
    fireEvent.keyDown(second, { key: "ArrowDown" });
    expect(fifth).toHaveFocus();
    fireEvent.keyDown(fifth, { key: "ArrowUp" });
    expect(second).toHaveFocus();
    fireEvent.keyDown(second, { key: "ArrowDown" });
    expect(fifth).toHaveFocus();
    fireEvent.keyDown(fifth, { key: "Enter" });

    await waitFor(() => {
      expect(mocks.ipc.bindNoteToWindow).toHaveBeenCalledWith(STICKY_ID, windows[4]);
      expect(mocks.close).toHaveBeenCalledTimes(1);
    });
  });

  it("selects a window with Space", async () => {
    render(<WindowPickerPage stickyId={STICKY_ID} />);
    const card = await screen.findByRole("button", {
      name: "Chrome，Project Alpha",
    });

    fireEvent.keyDown(card, { key: " " });

    await waitFor(() => {
      expect(mocks.ipc.bindNoteToWindow).toHaveBeenCalledWith(STICKY_ID, chromeWindow);
    });
  });

  it("closes the picker with Escape", async () => {
    render(<WindowPickerPage stickyId={STICKY_ID} />);
    await screen.findByRole("button", { name: "Chrome，Project Alpha" });

    fireEvent.keyDown(document, { key: "Escape" });

    expect(mocks.close).toHaveBeenCalledTimes(1);
    expect(mocks.ipc.bindNoteToWindow).not.toHaveBeenCalled();
  });

  it("prevents duplicate selection while a bind is pending", async () => {
    let resolveBind: (() => void) | undefined;
    mocks.ipc.bindNoteToWindow.mockReturnValueOnce(
      new Promise((resolve) => {
        resolveBind = () => resolve({});
      }),
    );
    render(<WindowPickerPage stickyId={STICKY_ID} />);
    const card = await screen.findByRole("button", {
      name: "Chrome，Project Alpha",
    });

    fireEvent.click(card);
    fireEvent.click(card);
    fireEvent.keyDown(card, { key: "Enter" });

    expect(mocks.ipc.bindNoteToWindow).toHaveBeenCalledTimes(1);
    expect(card).toBeDisabled();
    resolveBind?.();
    await waitFor(() => expect(mocks.close).toHaveBeenCalledTimes(1));
  });

  it("consumes Escape while a bind is pending", async () => {
    let resolveBind: (() => void) | undefined;
    mocks.ipc.bindNoteToWindow.mockReturnValueOnce(
      new Promise((resolve) => {
        resolveBind = () => resolve({});
      }),
    );
    render(<WindowPickerPage stickyId={STICKY_ID} />);
    const card = await screen.findByRole("button", {
      name: "Chrome，Project Alpha",
    });

    fireEvent.click(card);
    fireEvent.keyDown(document, { key: "Escape" });

    expect(mocks.close).not.toHaveBeenCalled();
    resolveBind?.();
    await waitFor(() => expect(mocks.close).toHaveBeenCalledTimes(1));
  });

  it("does not update or close after unmounting during a pending bind", async () => {
    let resolveBind: (() => void) | undefined;
    mocks.ipc.bindNoteToWindow.mockReturnValueOnce(
      new Promise((resolve) => {
        resolveBind = () => resolve({});
      }),
    );
    const view = render(<WindowPickerPage stickyId={STICKY_ID} />);
    const card = await screen.findByRole("button", {
      name: "Chrome，Project Alpha",
    });

    fireEvent.click(card);
    view.unmount();
    await act(async () => {
      resolveBind?.();
      await Promise.resolve();
    });
    expect(mocks.ipc.bindNoteToWindow).toHaveBeenCalledTimes(1);
    expect(mocks.close).not.toHaveBeenCalled();
  });

  it("does not permit another bind when closing fails after success", async () => {
    mocks.close.mockRejectedValueOnce("close failed");
    render(<WindowPickerPage stickyId={STICKY_ID} />);
    const card = await screen.findByRole("button", {
      name: "Chrome，Project Alpha",
    });

    fireEvent.click(card);

    expect(
      await screen.findByText("绑定成功，请手动关闭此窗口。"),
    ).toBeVisible();
    expect(card).toBeDisabled();
    fireEvent.click(card);
    expect(mocks.ipc.bindNoteToWindow).toHaveBeenCalledTimes(1);
  });

  it("removes only a stale target and explains the selection race", async () => {
    mocks.ipc.listEligibleWindows.mockResolvedValue([chromeWindow, figmaWindow]);
    mocks.ipc.bindNoteToWindow.mockRejectedValue(
      "selected target window is unavailable",
    );
    render(<WindowPickerPage stickyId={STICKY_ID} />);
    const stale = await screen.findByRole("button", {
      name: "Chrome，Project Alpha",
    });

    fireEvent.click(stale);

    expect(
      await screen.findByText("该窗口已关闭，请重新选择"),
    ).toBeVisible();
    expect(
      screen.queryByRole("button", { name: "Chrome，Project Alpha" }),
    ).toBeNull();
    expect(
      screen.getByRole("button", { name: "Figma，Design system" }),
    ).toBeVisible();
    expect(
      screen.getByRole("button", { name: "Figma，Design system" }),
    ).toHaveFocus();
    expect(mocks.close).not.toHaveBeenCalled();
  });

  it("keeps cards available after a generic binding error", async () => {
    const windows = [
      chromeWindow,
      figmaWindow,
      descriptor("Terminal", "Shell", 3),
      descriptor("Safari", "Reference", 4),
      descriptor("Notes", "Ideas", 5),
    ];
    mocks.ipc.listEligibleWindows.mockResolvedValue(windows);
    mocks.ipc.bindNoteToWindow.mockRejectedValue("native attachment failed");
    render(<WindowPickerPage stickyId={STICKY_ID} />);
    const selected = await screen.findByRole("button", {
      name: "Notes，Ideas",
    });

    selected.focus();
    fireEvent.click(selected);

    expect(await screen.findByText("绑定失败，请重试。")).toBeVisible();
    expect(selected).toBeVisible();
    expect(selected).toHaveFocus();
    expect(
      screen.getByRole("button", { name: "Figma，Design system" }),
    ).toBeVisible();
  });
});

describe("window picker routing", () => {
  it("accepts only a canonical ULID picker label", () => {
    expect(parseWindowLabel(`window-picker-${STICKY_ID}`)).toEqual({
      kind: "window-picker",
      stickyId: STICKY_ID,
    });
    for (const label of [
      "window-picker-not-an-id",
      "window-picker-01arz3ndektsv4rrffq69g5fav",
      `window-picker-${STICKY_ID}-suffix`,
      `sticky-window-picker-${STICKY_ID}`,
    ]) {
      expect(parseWindowLabel(label)).toEqual({ kind: "unknown" });
    }
    expect(parseWindowLabel(`sticky-${STICKY_ID}`)).toEqual({
      kind: "sticky",
      stickyId: STICKY_ID,
    });
    expect(parseWindowLabel("sticky-not-an-id")).toEqual({ kind: "unknown" });
  });

  it("falls back safely for an invalid picker label without loading sticky data", async () => {
    mocks.getCurrentWindow.mockReturnValue({
      label: "window-picker-invalid",
      close: mocks.close,
    });

    render(<App />);

    expect(await screen.findByText("Unknown window label.")).toBeVisible();
    expect(mocks.ipc.getAccessibilityStatus).not.toHaveBeenCalled();
    expect(mocks.ipc.listEligibleWindows).not.toHaveBeenCalled();
  });
});
