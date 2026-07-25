import { useCallback, useEffect, useRef, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import {
  AppWindow,
  LockKeyhole,
  RefreshCw,
  Settings,
} from "lucide-react";
import { ipc } from "../ipc/client";
import type { WindowDescriptor } from "../ipc/types";

type PickerState = "loading" | "denied" | "empty" | "ready" | "error";

const GRID_COLUMNS = 3;
const STALE_TARGET_ERROR = "selected target window is unavailable";

function targetKey(target: WindowDescriptor): string {
  return `${target.pid}:${target.windowId}`;
}

function isStaleTargetError(error: unknown): boolean {
  return String(error).includes(STALE_TARGET_ERROR);
}

async function closePicker(): Promise<boolean> {
  try {
    await getCurrentWindow().close();
    return true;
  } catch (error) {
    console.error("[floaty] close window picker failed:", error);
    return false;
  }
}

export function WindowPickerPage({ stickyId }: { stickyId: string }) {
  const [state, setState] = useState<PickerState>("loading");
  const [windows, setWindows] = useState<WindowDescriptor[]>([]);
  const [notice, setNotice] = useState<string | null>(null);
  const [selectingKey, setSelectingKey] = useState<string | null>(null);
  const requestGeneration = useRef(0);
  const selectionPending = useRef(false);
  const mounted = useRef(false);
  const focusIndex = useRef(0);
  const cardRefs = useRef<Array<HTMLButtonElement | null>>([]);

  const loadWindows = useCallback(async () => {
    const generation = ++requestGeneration.current;
    setState("loading");
    setNotice(null);
    try {
      const permission = await ipc.getAccessibilityStatus();
      if (generation !== requestGeneration.current) return;
      if (permission !== "granted") {
        setWindows([]);
        setState("denied");
        return;
      }

      const eligible = await ipc.listEligibleWindows();
      if (generation !== requestGeneration.current) return;
      focusIndex.current = 0;
      setWindows(eligible);
      setState(eligible.length > 0 ? "ready" : "empty");
    } catch {
      if (generation !== requestGeneration.current) return;
      setWindows([]);
      setState("error");
    }
  }, []);

  useEffect(() => {
    mounted.current = true;
    void loadWindows();
    return () => {
      mounted.current = false;
      requestGeneration.current += 1;
    };
  }, [loadWindows]);

  useEffect(() => {
    const handleEscape = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      event.preventDefault();
      if (selectionPending.current) return;
      void closePicker();
    };
    document.addEventListener("keydown", handleEscape);
    return () => document.removeEventListener("keydown", handleEscape);
  }, []);

  useEffect(() => {
    if (state === "ready" && selectingKey === null) {
      const index = Math.min(focusIndex.current, windows.length - 1);
      cardRefs.current[Math.max(index, 0)]?.focus();
    }
  }, [state, windows, selectingKey]);

  const openSettings = async () => {
    setNotice(null);
    try {
      await ipc.openAccessibilitySettings();
    } catch {
      if (mounted.current) {
        setNotice("无法打开系统设置，请稍后重试。");
      }
    }
  };

  const selectWindow = async (target: WindowDescriptor, selectedIndex: number) => {
    if (selectionPending.current) return;
    selectionPending.current = true;
    focusIndex.current = selectedIndex;
    let bindingSucceeded = false;
    const selectedKey = targetKey(target);
    setSelectingKey(selectedKey);
    setNotice(null);
    try {
      await ipc.bindNoteToWindow(stickyId, target);
      bindingSucceeded = true;
      if (!mounted.current) return;
      if (!(await closePicker())) {
        if (mounted.current) {
          setNotice("绑定成功，请手动关闭此窗口。");
        }
      }
    } catch (error) {
      if (!mounted.current) return;
      if (isStaleTargetError(error)) {
        const remaining = windows.filter(
          (candidate) => targetKey(candidate) !== selectedKey,
        );
        focusIndex.current = Math.min(selectedIndex, remaining.length - 1);
        setWindows(remaining);
        setState(remaining.length > 0 ? "ready" : "empty");
        setNotice("该窗口已关闭，请重新选择");
      } else {
        setNotice("绑定失败，请重试。");
      }
    } finally {
      if (!bindingSucceeded && mounted.current) {
        selectionPending.current = false;
        setSelectingKey(null);
      }
    }
  };

  const handleCardKeyDown = (
    event: React.KeyboardEvent<HTMLButtonElement>,
    index: number,
    target: WindowDescriptor,
  ) => {
    let nextIndex: number | null = null;
    const column = index % GRID_COLUMNS;
    if (
      event.key === "ArrowRight" &&
      column < GRID_COLUMNS - 1 &&
      index + 1 < windows.length
    ) {
      nextIndex = index + 1;
    } else if (event.key === "ArrowLeft" && column > 0) {
      nextIndex = index - 1;
    } else if (event.key === "ArrowDown" && index + GRID_COLUMNS < windows.length) {
      nextIndex = index + GRID_COLUMNS;
    } else if (event.key === "ArrowUp" && index - GRID_COLUMNS >= 0) {
      nextIndex = index - GRID_COLUMNS;
    } else if (event.key === "Enter" || event.key === " ") {
      event.preventDefault();
      void selectWindow(target, index);
      return;
    }

    if (nextIndex !== null) {
      event.preventDefault();
      focusIndex.current = nextIndex;
      cardRefs.current[nextIndex]?.focus();
    } else if (
      event.key === "ArrowRight" ||
      event.key === "ArrowLeft" ||
      event.key === "ArrowDown" ||
      event.key === "ArrowUp"
    ) {
      event.preventDefault();
    }
  };

  return (
    <main className="window-picker">
      <header className="window-picker-header">
        <div>
          <h1>绑定到窗口</h1>
          <p>选择一个当前窗口，便签会跟随它所在的桌面空间。</p>
        </div>
        <div className="window-picker-privacy">
          <LockKeyhole size={14} aria-hidden="true" />
          仅显示窗口信息，不读取屏幕内容
        </div>
      </header>

      <section className="window-picker-content" aria-live="polite">
        {notice && (
          <div className="window-picker-notice" role="status">
            {notice}
          </div>
        )}

        {state === "loading" && (
          <div className="window-picker-state" role="status">
            <RefreshCw className="window-picker-spinner" size={20} aria-hidden="true" />
            <strong>正在查找可绑定窗口</strong>
            <span>只会读取应用名称和窗口标题。</span>
          </div>
        )}

        {state === "denied" && (
          <div className="window-picker-state">
            <LockKeyhole size={24} aria-hidden="true" />
            <strong>需要辅助功能权限</strong>
            <span>
              Floaty 使用此权限识别你选择的窗口，不会请求屏幕录制权限。
            </span>
            <div className="window-picker-actions">
              <button type="button" className="window-picker-primary" onClick={openSettings}>
                <Settings size={14} aria-hidden="true" />
                前往系统设置
              </button>
              <button type="button" onClick={() => void loadWindows()}>
                重新检查
              </button>
            </div>
          </div>
        )}

        {state === "empty" && (
          <div className="window-picker-state">
            <AppWindow size={24} aria-hidden="true" />
            <strong>没有可绑定的窗口</strong>
            <span>请打开或恢复一个普通应用窗口，然后刷新列表。</span>
            <div className="window-picker-actions">
              <button
                type="button"
                className="window-picker-primary"
                onClick={() => void loadWindows()}
              >
                <RefreshCw size={14} aria-hidden="true" />
                刷新
              </button>
              <button type="button" onClick={() => void closePicker()}>
                取消
              </button>
            </div>
          </div>
        )}

        {state === "error" && (
          <div className="window-picker-state">
            <AppWindow size={24} aria-hidden="true" />
            <strong>暂时无法获取窗口</strong>
            <span>请检查辅助功能权限，或稍后重试。</span>
            <div className="window-picker-actions">
              <button
                type="button"
                className="window-picker-primary"
                onClick={() => void loadWindows()}
              >
                <RefreshCw size={14} aria-hidden="true" />
                重试
              </button>
              <button type="button" onClick={() => void closePicker()}>
                取消
              </button>
            </div>
          </div>
        )}

        {state === "ready" && (
          <div
            className="window-picker-grid"
            role="group"
            aria-label="可绑定窗口"
            aria-busy={selectionPending.current}
          >
            {windows.map((target, index) => {
              const key = targetKey(target);
              const accessibleTitle = target.windowTitle || "未命名窗口";
              const isSelecting = selectingKey === key;
              return (
                <button
                  key={key}
                  ref={(element) => {
                    cardRefs.current[index] = element;
                  }}
                  type="button"
                  className="window-picker-card"
                  aria-label={`${target.appName}，${accessibleTitle}`}
                  disabled={selectingKey !== null}
                  onClick={() => void selectWindow(target, index)}
                  onKeyDown={(event) => handleCardKeyDown(event, index, target)}
                >
                  {target.appIconDataUrl ? (
                    <img
                      className="window-picker-app-icon"
                      src={target.appIconDataUrl}
                      alt=""
                      draggable={false}
                    />
                  ) : (
                    <span className="window-picker-icon-fallback" aria-hidden="true">
                      {target.appName.trim().charAt(0).toLocaleUpperCase() || "?"}
                    </span>
                  )}
                  <span className="window-picker-card-copy">
                    <strong>{target.appName}</strong>
                    <span title={accessibleTitle}>{accessibleTitle}</span>
                  </span>
                  {isSelecting && (
                    <span className="window-picker-selecting" aria-hidden="true">
                      绑定中…
                    </span>
                  )}
                </button>
              );
            })}
          </div>
        )}
      </section>

      <footer className="window-picker-footer">
        <span>方向键选择</span>
        <span>Enter 确认</span>
        <span>Esc 取消</span>
      </footer>
    </main>
  );
}
