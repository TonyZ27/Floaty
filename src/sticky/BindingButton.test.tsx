import {
  act,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { Sticky, WindowBindingView } from "../ipc/types";
import { BindingButton } from "./BindingButton";
import { StickyPage } from "./StickyPage";

type BindingEventHandler = (event: { payload: string }) => void;

const mocks = vi.hoisted(() => ({
  listeners: new Set<BindingEventHandler>(),
  listen: vi.fn(),
  unlisten: vi.fn(),
  ipc: {
    getWindowBinding: vi.fn(),
    openWindowPicker: vi.fn(),
    unbindNoteFromWindow: vi.fn(),
    getSticky: vi.fn(),
    listItems: vi.fn(),
    togglePin: vi.fn(),
    hideSticky: vi.fn(),
    newStickyWindow: vi.fn(),
    updateSticky: vi.fn(),
    deleteSticky: vi.fn(),
    upsertItem: vi.fn(),
    syncReminders: vi.fn(),
  },
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: mocks.listen,
}));

vi.mock("../ipc/client", () => ({
  ipc: mocks.ipc,
}));

vi.mock("../editor/Editor", () => ({
  Editor: () => <div data-testid="editor" />,
}));

const STICKY_ID = "01ARZ3NDEKTSV4RRFFQ69G5FAV";

const activeBinding: WindowBindingView = {
  stickyId: STICKY_ID,
  appBundleId: "com.google.Chrome",
  appName: "Chrome",
  appIconDataUrl: "data:image/png;base64,aWNvbg==",
  windowTitle: "Project Alpha",
  state: "active",
  manuallyHidden: false,
};

function sticky(pinned: number): Sticky {
  return {
    id: STICKY_ID,
    title: "Task note",
    x: 10,
    y: 20,
    w: 320,
    h: 300,
    pinned,
    bg_color: "#FFDC96",
    opacity: 0.85,
    font_size: 14,
    font_color: null,
    z_order: 0,
    hidden: 0,
    collapsed: 0,
    collapsed_w: 240,
    created_at: 1,
    updated_at: 1,
    deleted_at: null,
  };
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((promiseResolve, promiseReject) => {
    resolve = promiseResolve;
    reject = promiseReject;
  });
  return { promise, resolve, reject };
}

async function emitBindingChanged(stickyId: string) {
  await act(async () => {
    for (const listener of [...mocks.listeners]) {
      listener({ payload: stickyId });
    }
    await Promise.resolve();
  });
}

beforeEach(() => {
  vi.clearAllMocks();
  mocks.listeners.clear();
  mocks.listen.mockImplementation(
    async (_eventName: string, handler: BindingEventHandler) => {
      mocks.listeners.add(handler);
      return () => {
        mocks.listeners.delete(handler);
        mocks.unlisten();
      };
    },
  );
  mocks.ipc.getWindowBinding.mockResolvedValue(null);
  mocks.ipc.openWindowPicker.mockResolvedValue(undefined);
  mocks.ipc.unbindNoteFromWindow.mockResolvedValue(undefined);
  mocks.ipc.getSticky.mockResolvedValue(sticky(0));
  mocks.ipc.listItems.mockResolvedValue([]);
  mocks.ipc.togglePin.mockResolvedValue(true);
  mocks.ipc.hideSticky.mockResolvedValue(undefined);
  mocks.ipc.newStickyWindow.mockResolvedValue(STICKY_ID);
  mocks.ipc.updateSticky.mockImplementation(
    async (_stickyId: string, patch: Partial<Sticky>) => ({
      ...sticky(0),
      ...patch,
    }),
  );
  mocks.ipc.deleteSticky.mockResolvedValue(undefined);
  mocks.ipc.upsertItem.mockResolvedValue({});
  mocks.ipc.syncReminders.mockResolvedValue(undefined);
});

describe("BindingButton", () => {
  it("opens the picker from the generic unbound window-link icon", async () => {
    render(<BindingButton stickyId={STICKY_ID} />);

    fireEvent.click(
      await screen.findByRole("button", { name: "绑定到窗口" }),
    );

    expect(mocks.ipc.openWindowPicker).toHaveBeenCalledWith(STICKY_ID);
    expect(screen.queryByText("已绑定")).toBeNull();
  });

  it("shows unbound picker pending and error feedback", async () => {
    const picker = deferred<void>();
    mocks.ipc.openWindowPicker.mockReturnValueOnce(picker.promise);
    render(<BindingButton stickyId={STICKY_ID} />);

    fireEvent.click(
      await screen.findByRole("button", { name: "绑定到窗口" }),
    );
    expect(
      screen.getByRole("button", { name: "正在打开窗口选择器" }),
    ).toBeDisabled();

    picker.reject("picker failed");
    expect(
      await screen.findByRole("button", {
        name: "无法打开窗口选择器，请重试",
      }),
    ).toBeEnabled();
  });

  it("shows only the assigned app icon while bound", async () => {
    mocks.ipc.getWindowBinding.mockResolvedValue(activeBinding);
    render(<BindingButton stickyId={STICKY_ID} />);

    const trigger = await screen.findByRole("button", {
      name: "已绑定到 Chrome：Project Alpha",
    });
    expect(trigger.querySelector("img")).toHaveAttribute(
      "src",
      activeBinding.appIconDataUrl,
    );
    expect(screen.queryByText("已绑定")).toBeNull();
    expect(screen.queryByText("Chrome")).toBeNull();
  });

  it("uses a compact generic fallback when the app icon is unavailable", async () => {
    mocks.ipc.getWindowBinding.mockResolvedValue({
      ...activeBinding,
      appIconDataUrl: null,
    });
    render(<BindingButton stickyId={STICKY_ID} />);

    const trigger = await screen.findByRole("button", {
      name: "已绑定到 Chrome：Project Alpha",
    });
    expect(trigger.querySelector("img")).toBeNull();
    expect(trigger.querySelector(".lucide-app-window")).not.toBeNull();
    expect(screen.queryByText("已绑定")).toBeNull();
  });

  it.each([
    ["unavailable", "目标窗口不可用"],
    ["target_closed", "目标窗口已关闭"],
    ["reconnect_required", "需要重新绑定"],
  ] as const)("adds only a warning dot for %s", async (state, label) => {
    mocks.ipc.getWindowBinding.mockResolvedValue({
      ...activeBinding,
      state,
    });
    render(<BindingButton stickyId={STICKY_ID} />);

    const trigger = await screen.findByRole("button", {
      name: `已绑定到 Chrome：Project Alpha，${label}`,
    });
    expect(trigger.querySelector('[data-testid="binding-warning"]')).not.toBeNull();
    expect(screen.queryByText(label)).toBeNull();
  });

  it("opens a keyboard-accessible management menu and restores focus", async () => {
    mocks.ipc.getWindowBinding.mockResolvedValue(activeBinding);
    render(<BindingButton stickyId={STICKY_ID} />);
    const trigger = await screen.findByRole("button", {
      name: "已绑定到 Chrome：Project Alpha",
    });

    fireEvent.click(trigger);
    const rebind = screen.getByRole("menuitem", { name: "重新绑定" });
    expect(screen.getByRole("menu", { name: "窗口绑定管理" })).toBeVisible();
    expect(screen.getByText("Chrome")).toBeVisible();
    expect(screen.getByText("Project Alpha")).toBeVisible();
    expect(rebind).toHaveFocus();

    fireEvent.keyDown(document, { key: "Escape" });
    expect(screen.queryByRole("menu")).toBeNull();
    expect(trigger).toHaveFocus();

    fireEvent.click(trigger);
    fireEvent.mouseDown(document.body);
    expect(screen.queryByRole("menu")).toBeNull();
  });

  it("prevents duplicate rebind actions and shows an opening error", async () => {
    const picker = deferred<void>();
    mocks.ipc.getWindowBinding.mockResolvedValue(activeBinding);
    mocks.ipc.openWindowPicker.mockReturnValueOnce(picker.promise);
    render(<BindingButton stickyId={STICKY_ID} />);
    fireEvent.click(
      await screen.findByRole("button", {
        name: "已绑定到 Chrome：Project Alpha",
      }),
    );
    const rebind = screen.getByRole("menuitem", { name: "重新绑定" });
    const unbind = screen.getByRole("menuitem", { name: "取消绑定" });

    fireEvent.click(rebind);
    fireEvent.click(rebind);
    expect(mocks.ipc.openWindowPicker).toHaveBeenCalledTimes(1);
    expect(rebind).toBeDisabled();
    expect(unbind).toBeDisabled();

    picker.reject("picker failed");
    expect(
      await screen.findByRole("alert"),
    ).toHaveTextContent("无法打开窗口选择器，请重试。");
    expect(rebind).toBeEnabled();
  });

  it("shows unbind pending and failure states without duplicate requests", async () => {
    const request = deferred<void>();
    mocks.ipc.getWindowBinding.mockResolvedValue(activeBinding);
    mocks.ipc.unbindNoteFromWindow.mockReturnValueOnce(request.promise);
    render(<BindingButton stickyId={STICKY_ID} />);
    fireEvent.click(
      await screen.findByRole("button", {
        name: "已绑定到 Chrome：Project Alpha",
      }),
    );
    const unbind = screen.getByRole("menuitem", { name: "取消绑定" });

    fireEvent.click(unbind);
    fireEvent.click(unbind);
    expect(mocks.ipc.unbindNoteFromWindow).toHaveBeenCalledTimes(1);
    expect(
      screen.getByRole("menuitem", { name: "正在取消…" }),
    ).toBeDisabled();

    request.reject("runtime restore failed");
    expect(
      await screen.findByRole("alert"),
    ).toHaveTextContent("取消绑定失败，请重试。");
  });

  it("finishes a successful unbind and refreshes parent sticky state", async () => {
    const onBindingMutation = vi.fn();
    mocks.ipc.getWindowBinding
      .mockResolvedValueOnce(activeBinding)
      .mockResolvedValueOnce(null);
    render(
      <BindingButton
        stickyId={STICKY_ID}
        onBindingMutation={onBindingMutation}
      />,
    );
    fireEvent.click(
      await screen.findByRole("button", {
        name: "已绑定到 Chrome：Project Alpha",
      }),
    );

    fireEvent.click(screen.getByRole("menuitem", { name: "取消绑定" }));

    expect(
      await screen.findByRole("button", { name: "绑定到窗口" }),
    ).toBeVisible();
    expect(mocks.ipc.unbindNoteFromWindow).toHaveBeenCalledWith(STICKY_ID);
    expect(onBindingMutation).toHaveBeenCalledTimes(1);
  });

  it("filters binding events and prevents an old fetch from overwriting a new result", async () => {
    const initial = deferred<WindowBindingView | null>();
    mocks.ipc.getWindowBinding
      .mockReturnValueOnce(initial.promise)
      .mockResolvedValueOnce(activeBinding);
    render(<BindingButton stickyId={STICKY_ID} />);
    await waitFor(() => expect(mocks.listen).toHaveBeenCalledTimes(1));

    await emitBindingChanged("another-sticky");
    expect(mocks.ipc.getWindowBinding).toHaveBeenCalledTimes(1);
    await emitBindingChanged(STICKY_ID);
    expect(
      await screen.findByRole("button", {
        name: "已绑定到 Chrome：Project Alpha",
      }),
    ).toBeVisible();

    initial.resolve(null);
    await act(async () => {
      await initial.promise;
      await Promise.resolve();
    });
    expect(
      screen.getByRole("button", {
        name: "已绑定到 Chrome：Project Alpha",
      }),
    ).toBeVisible();
  });

  it("does not update after unmount while a refresh is pending", async () => {
    const request = deferred<WindowBindingView | null>();
    mocks.ipc.getWindowBinding.mockReturnValueOnce(request.promise);
    const onBindingChange = vi.fn();
    const view = render(
      <BindingButton
        stickyId={STICKY_ID}
        onBindingChange={onBindingChange}
      />,
    );
    await waitFor(() => expect(mocks.listen).toHaveBeenCalledTimes(1));

    view.unmount();
    request.resolve(activeBinding);
    await act(async () => {
      await request.promise;
      await Promise.resolve();
    });
    expect(onBindingChange).toHaveBeenCalledTimes(1);
    expect(onBindingChange).toHaveBeenLastCalledWith(undefined);
    await waitFor(() => expect(mocks.unlisten).toHaveBeenCalledTimes(1));
  });

  it("keeps a successful binding visible when event subscription fails", async () => {
    const errorSpy = vi.spyOn(console, "error").mockImplementation(() => {});
    mocks.listen.mockRejectedValueOnce("listen failed");
    mocks.ipc.getWindowBinding.mockResolvedValue(activeBinding);
    const view = render(<BindingButton stickyId={STICKY_ID} />);

    expect(
      await screen.findByRole("button", {
        name: "已绑定到 Chrome：Project Alpha，状态更新暂时不可用",
      }),
    ).toBeVisible();
    expect(screen.getByTestId("binding-warning")).toBeVisible();
    view.unmount();
    await act(async () => {
      await Promise.resolve();
    });
    expect(errorSpy).toHaveBeenCalled();
    errorSpy.mockRestore();
  });

  it("uses a safe retry affordance when the initial binding query fails", async () => {
    mocks.ipc.getWindowBinding
      .mockRejectedValueOnce("database unavailable")
      .mockResolvedValueOnce(null);
    render(<BindingButton stickyId={STICKY_ID} />);

    const retry = await screen.findByRole("button", {
      name: "无法加载窗口绑定，点击重试",
    });
    fireEvent.click(retry);

    expect(
      await screen.findByRole("button", { name: "绑定到窗口" }),
    ).toBeVisible();
    expect(mocks.ipc.openWindowPicker).not.toHaveBeenCalled();
    expect(mocks.ipc.unbindNoteFromWindow).not.toHaveBeenCalled();
  });
});

describe("StickyPage binding and pin status", () => {
  it("explains why pinning is unavailable while binding status is unknown", async () => {
    const bindingRequest = deferred<WindowBindingView | null>();
    mocks.ipc.getWindowBinding.mockReturnValueOnce(bindingRequest.promise);
    render(<StickyPage stickyId={STICKY_ID} />);

    const pin = await screen.findByRole("button", {
      name: "无法确认窗口绑定，置顶暂不可用",
    });
    expect(pin).toBeDisabled();
    expect(pin).toHaveAttribute(
      "title",
      "无法确认窗口绑定，置顶暂不可用",
    );

    bindingRequest.resolve(null);
  });

  it("keeps ordinary pinning highlighted without a persistent tag", async () => {
    mocks.ipc.getSticky.mockResolvedValue(sticky(1));
    mocks.ipc.getWindowBinding.mockResolvedValue(null);
    render(<StickyPage stickyId={STICKY_ID} />);

    const pin = await screen.findByRole("button", {
      name: "已置顶（点击取消）",
    });
    await waitFor(() => expect(pin).toBeEnabled());
    expect(pin).toHaveClass("bg-red-500/20");
    expect(screen.queryByText("已置顶")).toBeNull();
    expect(screen.queryByText("已绑定")).toBeNull();
  });

  it("disables pinning in every binding state with an explanatory tooltip", async () => {
    mocks.ipc.getSticky.mockResolvedValue(sticky(1));
    mocks.ipc.getWindowBinding.mockResolvedValue({
      ...activeBinding,
      state: "target_closed",
    });
    render(<StickyPage stickyId={STICKY_ID} />);

    const pin = await screen.findByRole("button", {
      name: "已绑定到窗口时，置顶由绑定关系管理",
    });
    expect(pin).toBeDisabled();
    expect(pin).toHaveAttribute(
      "title",
      "已绑定到窗口时，置顶由绑定关系管理",
    );
    expect(pin).toHaveClass("bg-red-500/20");
    expect(screen.queryByText("已置顶")).toBeNull();
    expect(screen.queryByText("已绑定")).toBeNull();

    fireEvent.keyDown(window, {
      key: "p",
      metaKey: true,
      shiftKey: true,
    });
    expect(mocks.ipc.togglePin).not.toHaveBeenCalled();
  });

  it("refreshes restored pin state when unbind rejects after binding removal", async () => {
    const unbindRequest = deferred<void>();
    mocks.ipc.getWindowBinding
      .mockResolvedValueOnce(activeBinding)
      .mockResolvedValueOnce(null);
    mocks.ipc.getSticky
      .mockResolvedValueOnce(sticky(0))
      .mockResolvedValue(sticky(1));
    mocks.ipc.unbindNoteFromWindow.mockReturnValueOnce(unbindRequest.promise);
    render(<StickyPage stickyId={STICKY_ID} />);
    fireEvent.click(
      await screen.findByRole("button", {
        name: "已绑定到 Chrome：Project Alpha",
      }),
    );
    fireEvent.click(screen.getByRole("menuitem", { name: "取消绑定" }));

    await emitBindingChanged(STICKY_ID);
    const restoredPin = await screen.findByRole("button", {
      name: "已置顶（点击取消）",
    });
    expect(restoredPin).toBeEnabled();
    expect(restoredPin).toHaveClass("bg-red-500/20");

    unbindRequest.reject("runtime pin restoration failed");
    await act(async () => {
      await Promise.resolve();
    });
    expect(screen.queryByText("取消绑定失败，请重试。")).toBeNull();
    expect(
      screen.getByRole("button", { name: "绑定到窗口" }),
    ).toBeVisible();
  });

  it("does not retain a false unbind error when rejection wins the event race", async () => {
    const unbindRequest = deferred<void>();
    mocks.ipc.getWindowBinding
      .mockResolvedValueOnce(activeBinding)
      .mockResolvedValue(null);
    mocks.ipc.getSticky
      .mockResolvedValueOnce(sticky(0))
      .mockResolvedValue(sticky(1));
    mocks.ipc.unbindNoteFromWindow.mockReturnValueOnce(unbindRequest.promise);
    render(<StickyPage stickyId={STICKY_ID} />);
    fireEvent.click(
      await screen.findByRole("button", {
        name: "已绑定到 Chrome：Project Alpha",
      }),
    );
    fireEvent.click(screen.getByRole("menuitem", { name: "取消绑定" }));

    unbindRequest.reject("runtime pin restoration failed");
    const restoredPin = await screen.findByRole("button", {
      name: "已置顶（点击取消）",
    });
    expect(restoredPin).toBeEnabled();
    expect(
      screen.getByRole("button", { name: "绑定到窗口" }),
    ).toBeVisible();
    expect(screen.queryByText("取消绑定失败，请重试。")).toBeNull();

    await emitBindingChanged(STICKY_ID);
    expect(
      screen.getByRole("button", { name: "绑定到窗口" }),
    ).toBeVisible();
    expect(
      screen.queryByRole("button", {
        name: "无法打开窗口选择器，请重试",
      }),
    ).toBeNull();
  });

  it("recovers partial unbind by direct refresh when event subscription failed", async () => {
    const errorSpy = vi.spyOn(console, "error").mockImplementation(() => {});
    mocks.listen.mockRejectedValueOnce("listen failed");
    mocks.ipc.getWindowBinding
      .mockResolvedValueOnce(activeBinding)
      .mockResolvedValueOnce(null);
    mocks.ipc.getSticky
      .mockResolvedValueOnce(sticky(0))
      .mockResolvedValue(sticky(1));
    mocks.ipc.unbindNoteFromWindow.mockRejectedValueOnce(
      "runtime pin restoration failed",
    );
    render(<StickyPage stickyId={STICKY_ID} />);
    fireEvent.click(
      await screen.findByRole("button", {
        name: /已绑定到 Chrome：Project Alpha/,
      }),
    );

    fireEvent.click(screen.getByRole("menuitem", { name: "取消绑定" }));

    const restoredPin = await screen.findByRole("button", {
      name: "已置顶（点击取消）",
    });
    expect(restoredPin).toBeEnabled();
    expect(
      screen.getByRole("button", {
        name: "绑定到窗口，状态自动更新暂不可用",
      }),
    ).toBeVisible();
    expect(screen.queryByText("取消绑定失败，请重试。")).toBeNull();
    errorSpy.mockRestore();
  });
});
