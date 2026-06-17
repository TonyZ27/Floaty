import { useEffect, useState } from "react";
import { Editor } from "../editor/Editor";
import { useStickyData } from "./useStickyData";
import { SettingsPopover } from "./SettingsPopover";
import { ipc } from "../ipc/client";
import { autoFg } from "../theme/contrast";
import type { StickyPatch } from "../ipc/types";

interface StickyPageProps {
  stickyId: string;
}

export function StickyPage({ stickyId }: StickyPageProps) {
  const { sticky, setSticky, markdown, loaded, save } = useStickyData(stickyId);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const [isEditing, setIsEditing] = useState(false);
  const [tempTitle, setTempTitle] = useState("");

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
        (async () => {
          try {
            await ipc.togglePin(stickyId);
            const fresh = await ipc.getSticky(stickyId);
            setSticky(fresh);
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
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [stickyId, setSticky]);

  if (!loaded || !sticky) {
    return <div className="h-screen bg-yellow-100 p-3 text-xs opacity-60">Loading...</div>;
  }

  const pinned = sticky.pinned === 1;
  const fg = sticky.font_color ?? autoFg(sticky.bg_color);

  const onTogglePin = async (e: React.MouseEvent) => {
    e.preventDefault();
    e.stopPropagation();
    try {
      await ipc.togglePin(stickyId);
      const fresh = await ipc.getSticky(stickyId);
      setSticky(fresh);
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
      className="h-screen flex flex-col backdrop-blur-md relative"
      style={{
        backgroundColor: hexWithAlpha(sticky.bg_color, sticky.opacity),
      }}
      onClick={() => settingsOpen && setSettingsOpen(false)}
    >
      <div
        data-tauri-drag-region
        className="flex items-center justify-between px-3 py-1.5 text-xs border-b border-black/5 select-none cursor-grab active:cursor-grabbing"
        style={{ color: autoFg(sticky.bg_color) }}
      >
        <div data-tauri-drag-region className="flex items-center gap-1.5 opacity-70 flex-1 min-w-0 mr-2">
          <span className="pointer-events-none select-none">📋</span>
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
              className="bg-black/10 border border-black/10 rounded px-1 py-0.5 text-xs outline-none w-full max-w-[130px] font-semibold"
              style={{ color: "inherit" }}
              autoFocus
              onMouseDown={(e) => e.stopPropagation()}
              onClick={(e) => e.stopPropagation()}
            />
          ) : (
            <span
              className="cursor-pointer hover:bg-black/5 px-1 py-0.5 rounded font-semibold truncate max-w-[130px] select-none"
              onDoubleClick={startEditing}
              title="双击重命名"
              style={{ pointerEvents: "auto" } as React.CSSProperties}
            >
              {sticky.title || "Floaty"}
            </span>
          )}
          {pinned && (
            <span className="pointer-events-none text-[9px] font-semibold uppercase tracking-wider px-1.5 py-[1px] rounded bg-red-500 text-white max-[240px]:hidden flex-shrink-0">
              已置顶
            </span>
          )}
        </div>
        <div
          className="flex items-center gap-1"
          style={{ WebkitAppRegion: "no-drag" } as React.CSSProperties}
          onMouseDown={(e) => e.stopPropagation()}
        >
          <button
            type="button"
            className={
              pinned
                ? "text-[12px] w-6 h-5 rounded bg-red-500 text-white shadow-sm"
                : "text-[12px] w-6 h-5 rounded opacity-40 hover:opacity-80 hover:bg-black/5"
            }
            onClick={onTogglePin}
            title={pinned ? "已置顶（点击取消）" : "置顶"}
            style={{ WebkitAppRegion: "no-drag" } as React.CSSProperties}
          >
            📌
          </button>
          <button
            type="button"
            className={
              settingsOpen
                ? "text-[11px] w-6 h-5 rounded bg-black/10"
                : "text-[11px] w-6 h-5 rounded opacity-40 hover:opacity-80 hover:bg-black/5"
            }
            onClick={toggleSettings}
            title="外观设置"
            style={{ WebkitAppRegion: "no-drag" } as React.CSSProperties}
          >
            ⚙
          </button>
          <button
            type="button"
            className="text-[11px] w-6 h-5 rounded opacity-40 hover:opacity-80 hover:bg-black/5"
            onClick={onClose}
            title="关闭（不删除，可从菜单栏恢复）"
            style={{ WebkitAppRegion: "no-drag" } as React.CSSProperties}
          >
            ✕
          </button>
        </div>
      </div>
      <div
        className="flex-1 overflow-auto p-3"
        style={{ color: fg, fontSize: `${sticky.font_size}px` }}
      >
        <Editor initialMarkdown={markdown} onChange={save} />
      </div>
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
          className="absolute inset-0 z-20 flex items-center justify-center"
          style={{ backgroundColor: "rgba(0,0,0,0.35)" }}
          onClick={() => setConfirmDelete(false)}
          onMouseDown={(e) => e.stopPropagation()}
        >
          <div
            className="bg-white rounded-lg shadow-xl p-3 w-[90%] max-w-[260px] text-xs"
            style={{ color: "#333" }}
            onClick={(e) => e.stopPropagation()}
          >
            <div className="font-semibold mb-1">删除这张便签？</div>
            <div className="opacity-70 mb-3 leading-relaxed">
              所有内容和提醒会被永久删除，无法恢复。
            </div>
            <div className="flex gap-2 justify-end">
              <button
                className="px-3 h-7 rounded border border-black/10 hover:bg-black/5"
                onClick={() => setConfirmDelete(false)}
              >
                取消
              </button>
              <button
                className="px-3 h-7 rounded bg-red-500 text-white hover:bg-red-600"
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
