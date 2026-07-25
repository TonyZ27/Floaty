import { listen } from "@tauri-apps/api/event";
import {
  AlertCircle,
  AppWindow,
  Link2,
  RefreshCw,
  Unlink,
} from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import { ipc } from "../ipc/client";
import type { BindingState, WindowBindingView } from "../ipc/types";

type BindingLoadResult = WindowBindingView | null | undefined;

interface BindingButtonProps {
  stickyId: string;
  onBindingChange?: (binding: BindingLoadResult) => void;
  onBindingMutation?: () => void | Promise<void>;
}

type BindingLoadState =
  | { kind: "loading" }
  | { kind: "error" }
  | { kind: "ready"; binding: WindowBindingView | null };

type PendingAction = "rebind" | "unbind" | null;

const WARNING_LABELS: Partial<Record<BindingState, string>> = {
  target_closed: "目标窗口已关闭",
  reconnect_required: "需要重新绑定",
  unavailable: "目标窗口不可用",
};

function targetDescription(binding: WindowBindingView): string {
  const title = binding.windowTitle.trim();
  return title ? `${binding.appName}：${title}` : binding.appName;
}

export function BindingButton({
  stickyId,
  onBindingChange,
  onBindingMutation,
}: BindingButtonProps) {
  const [loadState, setLoadState] = useState<BindingLoadState>({
    kind: "loading",
  });
  const [menuOpen, setMenuOpen] = useState(false);
  const [pendingAction, setPendingAction] = useState<PendingAction>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const [eventSyncError, setEventSyncError] = useState(false);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const menuRef = useRef<HTMLDivElement>(null);
  const firstActionRef = useRef<HTMLButtonElement>(null);
  const mountedRef = useRef(false);
  const requestGenerationRef = useRef(0);
  const actionInFlightRef = useRef(false);
  const bindingRef = useRef<BindingLoadResult>(undefined);
  const onBindingChangeRef = useRef(onBindingChange);
  const onBindingMutationRef = useRef(onBindingMutation);
  onBindingChangeRef.current = onBindingChange;
  onBindingMutationRef.current = onBindingMutation;

  const refreshBinding = useCallback(async () => {
    const generation = ++requestGenerationRef.current;
    try {
      const binding = await ipc.getWindowBinding(stickyId);
      if (
        !mountedRef.current ||
        generation !== requestGenerationRef.current
      ) {
        return;
      }
      bindingRef.current = binding;
      setLoadState({ kind: "ready", binding });
      onBindingChangeRef.current?.(binding);
      if (binding === null) {
        setMenuOpen(false);
      }
    } catch {
      if (
        !mountedRef.current ||
        generation !== requestGenerationRef.current
      ) {
        return;
      }
      bindingRef.current = undefined;
      setLoadState({ kind: "error" });
      onBindingChangeRef.current?.(undefined);
    }
  }, [stickyId]);

  useEffect(() => {
    mountedRef.current = true;
    bindingRef.current = undefined;
    setLoadState({ kind: "loading" });
    setEventSyncError(false);
    onBindingChangeRef.current?.(undefined);
    void refreshBinding();

    const unlistenPromise = listen<string>(
      "binding-changed",
      (event) => {
        if (event.payload === stickyId) {
          void refreshBinding();
        }
      },
    ).catch((error) => {
      console.error("[floaty] binding event subscription failed:", error);
      if (mountedRef.current) {
        setEventSyncError(true);
      }
      return null;
    });

    return () => {
      mountedRef.current = false;
      requestGenerationRef.current += 1;
      void unlistenPromise.then((unlisten) => unlisten?.());
    };
  }, [refreshBinding, stickyId]);

  useEffect(() => {
    if (!menuOpen) return;

    firstActionRef.current?.focus();

    const handleMouseDown = (event: MouseEvent) => {
      const target = event.target;
      if (!(target instanceof Node)) return;
      if (
        !menuRef.current?.contains(target) &&
        !triggerRef.current?.contains(target)
      ) {
        setMenuOpen(false);
        setActionError(null);
      }
    };
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      event.preventDefault();
      setMenuOpen(false);
      setActionError(null);
      triggerRef.current?.focus();
    };

    document.addEventListener("mousedown", handleMouseDown);
    document.addEventListener("keydown", handleKeyDown);
    return () => {
      document.removeEventListener("mousedown", handleMouseDown);
      document.removeEventListener("keydown", handleKeyDown);
    };
  }, [menuOpen]);

  const openPicker = async () => {
    if (actionInFlightRef.current) return;
    actionInFlightRef.current = true;
    setPendingAction("rebind");
    setActionError(null);
    try {
      await ipc.openWindowPicker(stickyId);
      if (mountedRef.current) {
        setMenuOpen(false);
        triggerRef.current?.focus();
      }
    } catch {
      if (mountedRef.current) {
        setActionError("无法打开窗口选择器，请重试。");
      }
    } finally {
      actionInFlightRef.current = false;
      if (mountedRef.current) {
        setPendingAction(null);
      }
    }
  };

  const unbind = async () => {
    if (actionInFlightRef.current) return;
    actionInFlightRef.current = true;
    setPendingAction("unbind");
    setActionError(null);
    try {
      await ipc.unbindNoteFromWindow(stickyId);
      await refreshBinding();
      await onBindingMutationRef.current?.();
      if (mountedRef.current) {
        triggerRef.current?.focus();
      }
    } catch {
      if (mountedRef.current) {
        if (bindingRef.current) {
          setActionError("取消绑定失败，请重试。");
        } else {
          setMenuOpen(false);
        }
      }
    } finally {
      actionInFlightRef.current = false;
      if (mountedRef.current) {
        setPendingAction(null);
      }
    }
  };

  const stopHeaderEvent = (event: React.SyntheticEvent) => {
    event.stopPropagation();
  };

  if (loadState.kind === "loading") {
    return (
      <button
        ref={triggerRef}
        type="button"
        className="w-6 h-6 flex items-center justify-center rounded-md opacity-35"
        aria-label="正在检查窗口绑定"
        title="正在检查窗口绑定"
        disabled
        onMouseDown={stopHeaderEvent}
        onDoubleClick={stopHeaderEvent}
      >
        <Link2 size={13} strokeWidth={2.2} aria-hidden="true" />
      </button>
    );
  }

  if (loadState.kind === "error") {
    return (
      <button
        ref={triggerRef}
        type="button"
        className="relative w-6 h-6 flex items-center justify-center rounded-md opacity-55 hover:opacity-95 hover:bg-black/5 transition-all duration-150 active:scale-[0.92]"
        aria-label="无法加载窗口绑定，点击重试"
        title="无法加载窗口绑定，点击重试"
        onClick={(event) => {
          event.stopPropagation();
          void refreshBinding();
        }}
        onMouseDown={stopHeaderEvent}
        onDoubleClick={stopHeaderEvent}
      >
        <Link2 size={13} strokeWidth={2.2} aria-hidden="true" />
        <span
          className="absolute top-[3px] right-[3px] w-1.5 h-1.5 rounded-full bg-amber-500 ring-1 ring-white/70"
          aria-hidden="true"
        />
      </button>
    );
  }

  const binding = loadState.binding;
  if (binding === null) {
    const unboundStatus = actionError
      ? "无法打开窗口选择器，请重试"
      : pendingAction === "rebind"
        ? "正在打开窗口选择器"
        : eventSyncError
          ? "绑定到窗口，状态自动更新暂不可用"
          : "绑定到窗口";
    return (
      <button
        ref={triggerRef}
        type="button"
        className="relative w-6 h-6 flex items-center justify-center rounded-md opacity-45 hover:opacity-90 hover:bg-black/5 transition-all duration-150 active:scale-[0.92]"
        aria-label={unboundStatus}
        title={unboundStatus}
        disabled={pendingAction === "rebind"}
        onClick={(event) => {
          event.stopPropagation();
          void openPicker();
        }}
        onMouseDown={stopHeaderEvent}
        onDoubleClick={stopHeaderEvent}
      >
        <AppWindow size={13} strokeWidth={2.1} aria-hidden="true" />
        <Link2
          className="absolute bottom-[2px] right-[2px]"
          size={7}
          strokeWidth={2.8}
          aria-hidden="true"
        />
        {(actionError || eventSyncError) && (
          <span
            className="absolute top-[2px] right-[2px] w-1.5 h-1.5 rounded-full bg-amber-500 ring-1 ring-white/70"
            aria-hidden="true"
          />
        )}
      </button>
    );
  }

  const warningLabel = WARNING_LABELS[binding.state];
  const description = targetDescription(binding);
  const statusLabel = warningLabel ?? (
    eventSyncError ? "状态更新暂时不可用" : undefined
  );
  const accessibleLabel = statusLabel
    ? `已绑定到 ${description}，${statusLabel}`
    : `已绑定到 ${description}`;

  return (
    <div
      className="relative"
      onClick={stopHeaderEvent}
      onMouseDown={stopHeaderEvent}
      onDoubleClick={stopHeaderEvent}
    >
      <button
        ref={triggerRef}
        type="button"
        className={`relative w-6 h-6 flex items-center justify-center rounded-md transition-all duration-150 active:scale-[0.92] ${
          menuOpen
            ? "bg-black/10 opacity-100"
            : "opacity-65 hover:opacity-100 hover:bg-black/5"
        }`}
        aria-label={accessibleLabel}
        aria-haspopup="menu"
        aria-expanded={menuOpen}
        title={accessibleLabel}
        onClick={() => {
          setActionError(null);
          setMenuOpen((open) => !open);
        }}
      >
        {binding.appIconDataUrl ? (
          <img
            className="w-[15px] h-[15px] rounded-[3px] object-contain"
            src={binding.appIconDataUrl}
            alt=""
            draggable={false}
          />
        ) : (
          <AppWindow size={14} strokeWidth={2.1} aria-hidden="true" />
        )}
        {statusLabel && (
          <span
            data-testid="binding-warning"
            className="absolute top-[2px] right-[2px] w-1.5 h-1.5 rounded-full bg-amber-500 ring-1 ring-white/70"
            aria-hidden="true"
          />
        )}
      </button>

      {menuOpen && (
        <div
          ref={menuRef}
          role="menu"
          aria-label="窗口绑定管理"
          className="absolute top-7 right-0 z-30 w-56 rounded-xl border border-black/10 bg-white/95 p-2 text-[#1d1d1f] shadow-xl backdrop-blur-md"
        >
          <div
            role="presentation"
            className="px-2 pb-2 mb-1 border-b border-black/[0.07]"
          >
            <div className="text-[11px] font-semibold truncate">
              {binding.appName}
            </div>
            <div
              className="mt-0.5 text-[10px] opacity-60 truncate"
              title={binding.windowTitle || "未命名窗口"}
            >
              {binding.windowTitle || "未命名窗口"}
            </div>
            {warningLabel && (
              <div className="mt-1 text-[10px] font-medium text-amber-700">
                {warningLabel}
              </div>
            )}
          </div>
          <button
            ref={firstActionRef}
            type="button"
            role="menuitem"
            className="w-full h-7 px-2 rounded-md flex items-center gap-2 text-[11px] font-medium hover:bg-black/5 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-black/30 disabled:opacity-45"
            disabled={pendingAction !== null}
            onClick={() => void openPicker()}
          >
            <RefreshCw size={12} aria-hidden="true" />
            {pendingAction === "rebind" ? "正在打开…" : "重新绑定"}
          </button>
          <button
            type="button"
            role="menuitem"
            className="w-full h-7 px-2 rounded-md flex items-center gap-2 text-[11px] font-medium text-red-600 hover:bg-red-500/10 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-red-500/40 disabled:opacity-45"
            disabled={pendingAction !== null}
            onClick={() => void unbind()}
          >
            {pendingAction === "unbind" ? (
              <RefreshCw
                className="animate-spin"
                size={12}
                aria-hidden="true"
              />
            ) : (
              <Unlink size={12} aria-hidden="true" />
            )}
            {pendingAction === "unbind" ? "正在取消…" : "取消绑定"}
          </button>
          {actionError && (
            <div
              className="mt-1 px-2 py-1.5 rounded-md bg-red-500/10 text-[10px] text-red-700 flex items-start gap-1.5"
              role="alert"
            >
              <AlertCircle
                className="mt-px flex-shrink-0"
                size={11}
                aria-hidden="true"
              />
              <span>{actionError}</span>
            </div>
          )}
        </div>
      )}
    </div>
  );
}
