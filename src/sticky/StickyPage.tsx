import { useCallback, useEffect, useState } from "react";
import { Editor } from "../editor/Editor";
import { useStickyData } from "./useStickyData";
import { SettingsPopover } from "./SettingsPopover";
import { BindingButton } from "./BindingButton";
import { ipc } from "../ipc/client";
import { autoFg } from "../theme/contrast";
import type { StickyPatch, WindowBindingView } from "../ipc/types";
import { FileText, Pin, Settings, X, ChevronDown, ChevronUp } from "lucide-react";

interface StickyPageProps {
  stickyId: string;
}

export function StickyPage({ stickyId }: StickyPageProps) {
  const { sticky, setSticky, markdown, loaded, save } = useStickyData(stickyId);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const [isEditing, setIsEditing] = useState(false);
  const [tempTitle, setTempTitle] = useState("");
  const [bindingStatus, setBindingStatus] = useState<boolean | null>(null);

  const refreshSticky = useCallback(async () => {
    try {
      const fresh = await ipc.getSticky(stickyId);
      setSticky(fresh);
    } catch (err) {
      console.error("[floaty] sticky refresh failed:", err);
    }
  }, [setSticky, stickyId]);

  const handleBindingChange = useCallback(
    (binding: WindowBindingView | null | undefined) => {
      setBindingStatus(binding === undefined ? null : binding !== null);
      if (binding === null) {
        void refreshSticky();
      }
    },
    [refreshSticky],
  );

  // 便签内快捷键：⌘W / ⌘⇧P / ⌘, / ⌘⌫
  // 必须在任何 early-return 之前挂，否则 Rules of Hooks 炸。
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      const mod = e.metaKey || e.ctrlKey;
      if (!mod) return;
      const key = e.key.toLowerCase();
      if (key === "w" && !e.shiftKey) {
        e.preventDefault();
        ipc.hideSticky(stickyId).catch((err) => console.error("[floaty] ⌘W:", err));
        return;
      }
      // ⌘N：在当前便签聚焦时新建一张
      if (key === "n" && !e.shiftKey) {
        e.preventDefault();
        ipc.newStickyWindow().catch((err) => console.error("[floaty] ⌘N:", err));
        return;
      }
      if (key === "p" && e.shiftKey) {
        e.preventDefault();
        if (bindingStatus !== false) return;
        (async () => {
          try {
            await ipc.togglePin(stickyId);
            await refreshSticky();
          } catch (err) {
            console.error("[floaty] ⌘⇧P:", err);
          }
        })();
        return;
      }
      if (key === ",") {
        e.preventDefault();
        setSettingsOpen((v) => !v);
        return;
      }
      if (key === "backspace") {
        e.preventDefault();
        setSettingsOpen(false);
        setConfirmDelete(true);
        return;
      }
      if (e.key === "\\") {
        e.preventDefault();
        (async () => {
          try {
            const current = await ipc.getSticky(stickyId);
            const nextCollapsed = current.collapsed === 1 ? 0 : 1;
            const fresh = await ipc.updateSticky(stickyId, { collapsed: nextCollapsed });
            setSticky(fresh);
          } catch (err) {
            console.error("[floaty] ⌘\\:", err);
          }
        })();
        return;
      }
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [bindingStatus, refreshSticky, stickyId, setSticky]);

  if (!loaded || !sticky) {
    return <div className="h-screen bg-yellow-100 p-3 text-xs opacity-60">Loading...</div>;
  }

  const pinned = sticky.pinned === 1;
  const fg = sticky.font_color ?? autoFg(sticky.bg_color);

  const onTogglePin = async (e: React.MouseEvent) => {
    e.preventDefault();
    e.stopPropagation();
    if (bindingStatus !== false) return;
    try {
      await ipc.togglePin(stickyId);
      await refreshSticky();
    } catch (err) {
      console.error("[floaty] togglePin failed:", err);
    }
  };

  const onClose = async (e: React.MouseEvent) => {
    e.preventDefault();
    e.stopPropagation();
    try {
      await ipc.hideSticky(stickyId);
    } catch (err) {
      console.error("[floaty] hideSticky failed:", err);
    }
  };

  const toggleSettings = (e: React.MouseEvent) => {
    e.preventDefault();
    e.stopPropagation();
    setSettingsOpen((v) => !v);
  };

  const onPatch = async (patch: StickyPatch) => {
    try {
      const fresh = await ipc.updateSticky(stickyId, patch);
      setSticky(fresh);
    } catch (err) {
      console.error("[floaty] updateSticky failed:", err);
    }
  };

  const onToggleCollapse = async (e?: React.MouseEvent) => {
    if (e) {
      e.preventDefault();
      e.stopPropagation();
    }
    if (!sticky) return;
    if (sticky.collapsed === 0) {
      setSettingsOpen(false);
    }
    try {
      await onPatch({ collapsed: sticky.collapsed === 1 ? 0 : 1 });
    } catch (err) {
      console.error("[floaty] toggleCollapse failed:", err);
    }
  };

  const onHeaderDoubleClick = (e: React.MouseEvent) => {
    e.preventDefault();
    onToggleCollapse();
  };

  const startEditing = (e: React.MouseEvent) => {
    e.preventDefault();
    e.stopPropagation();
    if (sticky) {
      setTempTitle(sticky.title);
      setIsEditing(true);
    }
  };

  const onSaveTitle = async () => {
    setIsEditing(false);
    if (!sticky) return;
    const trimmed = tempTitle.trim();
    if (trimmed !== sticky.title) {
      await onPatch({ title: trimmed });
    }
  };

  const onCancelTitle = () => {
    setIsEditing(false);
  };

  const requestDelete = () => {
    setSettingsOpen(false);
    setConfirmDelete(true);
  };

  const performDelete = async () => {
    try {
      await ipc.deleteSticky(stickyId);
    } catch (err) {
      console.error("[floaty] deleteSticky failed:", err);
      setConfirmDelete(false);
    }
  };

  return (
    <div
      className="h-screen flex flex-col backdrop-blur-md relative rounded-[16px] overflow-hidden border border-black/[0.08] dark:border-white/10 shadow-lg"
      style={{
        backgroundColor: hexWithAlpha(sticky.bg_color, sticky.opacity),
      }}
      onClick={() => settingsOpen && setSettingsOpen(false)}
    >
      <div
        data-tauri-drag-region
        className={`flex items-center justify-between px-3 py-2 text-xs select-none cursor-grab active:cursor-grabbing ${
          sticky.collapsed === 1 ? "" : "border-b border-black/5"
        }`}
        style={{ color: autoFg(sticky.bg_color) }}
        onDoubleClick={onHeaderDoubleClick}
      >
        <div data-tauri-drag-region className="flex items-center gap-1.5 opacity-80 flex-1 min-w-0 mr-2">
          <FileText size={13} className="opacity-70 pointer-events-none select-none" />
          {isEditing ? (
            <input
              type="text"
              value={tempTitle}
              onChange={(e) => setTempTitle(e.target.value)}
              onBlur={onSaveTitle}
              onKeyDown={(e) => {
                if (e.key === "Enter") onSaveTitle();
                if (e.key === "Escape") onCancelTitle();
              }}
              className="bg-black/10 border border-black/10 rounded px-1.5 py-0.5 text-xs outline-none w-full max-w-[130px] font-semibold"
              style={{ color: "inherit" }}
              autoFocus
              onMouseDown={(e) => e.stopPropagation()}
              onClick={(e) => e.stopPropagation()}
            />
          ) : (
            <span
              className="cursor-pointer hover:bg-black/5 px-1.5 py-0.5 rounded font-semibold truncate max-w-[130px] select-none transition-colors duration-150"
              onDoubleClick={(e) => {
                e.stopPropagation();
                startEditing(e);
              }}
              title="双击重命名"
              style={{ pointerEvents: "auto" } as React.CSSProperties}
            >
              {sticky.title || "Floaty"}
            </span>
          )}
        </div>
        <div
          className="flex items-center gap-1"
          style={{ WebkitAppRegion: "no-drag" } as React.CSSProperties}
          onMouseDown={(e) => e.stopPropagation()}
          onDoubleClick={(e) => e.stopPropagation()}
        >
          <button
            type="button"
            className="w-6 h-6 flex items-center justify-center rounded-md opacity-45 hover:opacity-90 hover:bg-black/5 transition-all duration-150 active:scale-[0.92]"
            onClick={onToggleCollapse}
            title={sticky.collapsed === 1 ? "展开便签" : "折叠便签"}
            style={{ WebkitAppRegion: "no-drag" } as React.CSSProperties}
          >
            {sticky.collapsed === 1 ? (
              <ChevronDown size={13} strokeWidth={2.2} />
            ) : (
              <ChevronUp size={13} strokeWidth={2.2} />
            )}
          </button>
          <BindingButton
            stickyId={stickyId}
            onBindingChange={handleBindingChange}
            onBindingMutation={refreshSticky}
          />
          <button
            type="button"
            className={`w-6 h-6 flex items-center justify-center rounded-md transition-all duration-150 active:scale-[0.92] ${
              pinned
                ? "bg-red-500/20 text-red-600 hover:bg-red-500/30"
                : "opacity-45 hover:opacity-90 hover:bg-black/5"
            }`}
            onClick={onTogglePin}
            disabled={bindingStatus !== false}
            aria-label={
              bindingStatus === true
                ? "已绑定到窗口时，置顶由绑定关系管理"
                : bindingStatus === null
                  ? "无法确认窗口绑定，置顶暂不可用"
                  : pinned
                    ? "已置顶（点击取消）"
                    : "置顶"
            }
            title={
              bindingStatus === true
                ? "已绑定到窗口时，置顶由绑定关系管理"
                : bindingStatus === null
                  ? "无法确认窗口绑定，置顶暂不可用"
                  : pinned
                    ? "已置顶（点击取消）"
                    : "置顶"
            }
            style={{ WebkitAppRegion: "no-drag" } as React.CSSProperties}
          >
            <Pin size={13} strokeWidth={2.2} className={pinned ? "rotate-45" : ""} />
          </button>
          {sticky.collapsed === 0 && (
            <button
              type="button"
              className={`w-6 h-6 flex items-center justify-center rounded-md transition-all duration-150 active:scale-[0.92] ${
                settingsOpen
                  ? "bg-black/10 opacity-95"
                  : "opacity-45 hover:opacity-90 hover:bg-black/5"
              }`}
              onClick={toggleSettings}
              title="外观设置"
              style={{ WebkitAppRegion: "no-drag" } as React.CSSProperties}
            >
              <Settings size={13} strokeWidth={2.2} />
            </button>
          )}
          <button
            type="button"
            className="w-6 h-6 flex items-center justify-center rounded-md opacity-45 hover:opacity-90 hover:bg-black/5 hover:text-red-500 transition-all duration-150 active:scale-[0.92]"
            onClick={onClose}
            title="关闭（不删除，可从菜单栏恢复）"
            style={{ WebkitAppRegion: "no-drag" } as React.CSSProperties}
          >
            <X size={13} strokeWidth={2.2} />
          </button>
        </div>
      </div>
      {sticky.collapsed === 0 && (
        <div
          className="flex-1 overflow-auto p-3"
          style={{ color: fg, fontSize: `${sticky.font_size}px` }}
        >
          <Editor initialMarkdown={markdown} onChange={save} />
        </div>
      )}
      {settingsOpen && (
        <SettingsPopover
          sticky={sticky}
          onPatch={onPatch}
          onClose={() => setSettingsOpen(false)}
          onDelete={requestDelete}
        />
      )}
      {confirmDelete && (
        <div
          className="absolute inset-0 z-20 flex items-center justify-center backdrop-blur-md bg-black/15"
          onClick={() => setConfirmDelete(false)}
          onMouseDown={(e) => e.stopPropagation()}
        >
          <div
            className="bg-white/95 dark:bg-zinc-900/95 rounded-xl border border-white/20 shadow-2xl p-4 w-[90%] max-w-[260px] text-xs flex flex-col gap-1 transition-all"
            style={{ color: "#1d1d1f" }}
            onClick={(e) => e.stopPropagation()}
          >
            <div className="font-semibold text-[13px] tracking-tight">删除这张便签？</div>
            <div className="opacity-70 mb-3 leading-relaxed text-[11px]">
              所有内容和提醒会被永久删除，无法恢复。
            </div>
            <div className="flex gap-2 justify-end">
              <button
                className="px-3 h-7 rounded-md border border-black/10 hover:bg-black/5 text-[11px] font-medium transition active:scale-[0.98]"
                onClick={() => setConfirmDelete(false)}
              >
                取消
              </button>
              <button
                className="px-3 h-7 rounded-md bg-red-500 text-white hover:bg-red-600 font-medium text-[11px] transition active:scale-[0.98] shadow-sm shadow-red-500/10"
                onClick={performDelete}
              >
                删除
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}

function hexWithAlpha(hex: string, alpha: number): string {
  const r = parseInt(hex.slice(1, 3), 16);
  const g = parseInt(hex.slice(3, 5), 16);
  const b = parseInt(hex.slice(5, 7), 16);
  return `rgba(${r}, ${g}, ${b}, ${alpha})`;
}
