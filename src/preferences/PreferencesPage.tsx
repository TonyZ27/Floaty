import { useEffect, useState, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { enable, disable, isEnabled } from "@tauri-apps/plugin-autostart";
import { check, Update } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import { save } from "@tauri-apps/plugin-dialog";
import { getVersion } from "@tauri-apps/api/app";
import { ipc } from "../ipc/client";
import { IS_MAC } from "../platform";
import { ShortcutRecorder } from "../shortcuts/ShortcutRecorder";
import { KeyCombo, DEFAULT_SHORTCUTS } from "../shortcuts/types";
import { serializeKeyCombo, parseKeyCombo } from "../shortcuts/utils";

type UpdateStatus =
  | { kind: "idle" }
  | { kind: "checking" }
  | { kind: "up-to-date" }
  | { kind: "available"; update: Update }
  | { kind: "downloading"; received: number; total: number | null }
  | { kind: "installed" }
  | { kind: "error"; message: string };

interface Stats {
  stickies: number;
  items: number;
  pending_reminders: number;
}

export function PreferencesPage() {
  const [stats, setStats] = useState<Stats | null>(null);
  const [dataDir, setDataDir] = useState<string>("");
  const [autostart, setAutostart] = useState<boolean>(false);
  const [globalEnabled, setGlobalEnabled] = useState<boolean>(true);
  const [bubbleEnabled, setBubbleEnabled] = useState<boolean>(true);
  const [floatingEnabled, setFloatingEnabled] = useState<boolean>(true);
  const [currentVersion, setCurrentVersion] = useState<string>("");
  const [updateStatus, setUpdateStatus] = useState<UpdateStatus>({ kind: "idle" });
  const [backupStatus, setBackupStatus] = useState<
    | { kind: "idle" }
    | { kind: "running" }
    | { kind: "done"; path: string }
    | { kind: "error"; message: string }
  >({ kind: "idle" });

  const [shortcuts, setShortcuts] = useState<Record<string, KeyCombo | null>>({});
  const [toast, setToast] = useState<string | null>(null);
  const toastTimerRef = useRef<number | null>(null);
  const [showResetConfirm, setShowResetConfirm] = useState<boolean>(false);

  const showToast = (msg: string) => {
    setToast(msg);
    if (toastTimerRef.current) window.clearTimeout(toastTimerRef.current);
    toastTimerRef.current = window.setTimeout(() => setToast(null), 2200);
  };

  useEffect(() => {
    (async () => {
      try {
        const [s, d, a, g, bm, fm, v] = await Promise.all([
          invoke<Stats>("get_stats"),
          invoke<string>("get_data_dir"),
          isEnabled(),
          invoke<string | null>("get_setting", { key: "global_shortcut_enabled" }),
          invoke<string | null>("get_setting", { key: "bubble_menu_enabled" }),
          invoke<string | null>("get_setting", { key: "floating_menu_enabled" }),
          getVersion(),
        ]);
        setStats(s);
        setDataDir(d);
        setAutostart(a);
        setGlobalEnabled(g !== "false");
        setBubbleEnabled(bm !== "false");
        setFloatingEnabled(fm !== "false");
        setCurrentVersion(v);

        // 加载自定义快捷键
        const loadedShortcuts: Record<string, KeyCombo | null> = {};
        for (const def of DEFAULT_SHORTCUTS) {
          const val = await invoke<string | null>("get_setting", { key: def.id });
          loadedShortcuts[def.id] = val !== null ? parseKeyCombo(val) : def.defaultCombo;
        }
        setShortcuts(loadedShortcuts);
      } catch (err) {
        console.error("[floaty] preferences load failed:", err);
      }
    })();
  }, []);

  const handleShortcutChange = async (id: string, newCombo: KeyCombo | null) => {
    setShortcuts((prev) => ({ ...prev, [id]: newCombo }));
    try {
      const serialized = serializeKeyCombo(newCombo);
      await invoke("set_setting", { key: id, value: serialized });
      if (newCombo) {
        showToast("快捷键已更新");
      } else {
        showToast("已删除快捷键");
      }
    } catch (err) {
      console.error("[floaty] save shortcut failed:", err);
      showToast("保存快捷键失败");
    }
  };

  const handleConfirmReset = async () => {
    try {
      const resetMap: Record<string, KeyCombo | null> = {};
      for (const def of DEFAULT_SHORTCUTS) {
        resetMap[def.id] = def.defaultCombo;
        const serialized = serializeKeyCombo(def.defaultCombo);
        await invoke("set_setting", { key: def.id, value: serialized });
      }
      setShortcuts(resetMap);
      setShowResetConfirm(false);
      showToast("快捷键已恢复默认设置");
    } catch (err) {
      console.error("[floaty] reset shortcuts failed:", err);
      showToast("重置快捷键失败");
    }
  };

  const checkForUpdate = async () => {
    setUpdateStatus({ kind: "checking" });
    try {
      const update = await check();
      if (!update) {
        setUpdateStatus({ kind: "up-to-date" });
        return;
      }
      setUpdateStatus({ kind: "available", update });
    } catch (err) {
      setUpdateStatus({
        kind: "error",
        message: err instanceof Error ? err.message : String(err),
      });
    }
  };

  const downloadAndInstall = async (update: Update) => {
    setUpdateStatus({ kind: "downloading", received: 0, total: null });
    let received = 0;
    let total: number | null = null;
    try {
      await update.downloadAndInstall((evt) => {
        if (evt.event === "Started") {
          total = evt.data.contentLength ?? null;
          setUpdateStatus({ kind: "downloading", received: 0, total });
        } else if (evt.event === "Progress") {
          received += evt.data.chunkLength;
          setUpdateStatus({ kind: "downloading", received, total });
        } else if (evt.event === "Finished") {
          setUpdateStatus({ kind: "installed" });
        }
      });
      // macOS 下 downloadAndInstall 不会自动重启，手动 relaunch
      await relaunch();
    } catch (err) {
      setUpdateStatus({
        kind: "error",
        message: err instanceof Error ? err.message : String(err),
      });
    }
  };

  const toggleGlobal = async (next: boolean) => {
    try {
      await invoke("set_setting", {
        key: "global_shortcut_enabled",
        value: next ? "true" : "false",
      });
      setGlobalEnabled(next);
    } catch (err) {
      console.error("[floaty] toggle global shortcut failed:", err);
    }
  };

  const toggleBubble = async (next: boolean) => {
    try {
      await invoke("set_setting", { key: "bubble_menu_enabled", value: next ? "true" : "false" });
      setBubbleEnabled(next);
    } catch (err) {
      console.error("[floaty] toggle bubble failed:", err);
    }
  };

  const toggleFloating = async (next: boolean) => {
    try {
      await invoke("set_setting", { key: "floating_menu_enabled", value: next ? "true" : "false" });
      setFloatingEnabled(next);
    } catch (err) {
      console.error("[floaty] toggle floating failed:", err);
    }
  };

  const toggleAutostart = async (next: boolean) => {
    try {
      if (next) await enable();
      else await disable();
      setAutostart(next);
    } catch (err) {
      console.error("[floaty] toggle autostart failed:", err);
    }
  };

  const openDataDir = async () => {
    try {
      await invoke("plugin:opener|open_path", { path: dataDir });
    } catch {
      // 兜底：打开 file:// URL
      try {
        await invoke("plugin:opener|open_url", { url: `file://${dataDir}` });
      } catch (err) {
        console.error("[floaty] open data dir failed:", err);
      }
    }
  };

  const revealBackup = async (path: string) => {
    // 没有稳定跨平台的 reveal-in-dir，就打开父目录，用户自己能看到文件。
    const sep = IS_MAC ? "/" : path.includes("\\") ? "\\" : "/";
    const parent = path.substring(0, path.lastIndexOf(sep));
    try {
      await invoke("plugin:opener|open_path", { path: parent });
    } catch (err) {
      console.error("[floaty] reveal backup failed:", err);
    }
  };

  const runBackup = async () => {
    const now = new Date();
    const pad = (n: number) => String(n).padStart(2, "0");
    const stamp =
      `${now.getFullYear()}${pad(now.getMonth() + 1)}${pad(now.getDate())}-` +
      `${pad(now.getHours())}${pad(now.getMinutes())}${pad(now.getSeconds())}`;
    const defaultName = `floaty-backup-${stamp}.db`;
    try {
      const selected = await save({
        title: "保存 Floaty 备份",
        defaultPath: defaultName,
        filters: [{ name: "SQLite Database", extensions: ["db"] }],
      });
      if (!selected) return; // 用户取消
      setBackupStatus({ kind: "running" });
      const path = await invoke<string>("backup_database", { targetPath: selected });
      setBackupStatus({ kind: "done", path });
    } catch (err) {
      setBackupStatus({
        kind: "error",
        message: err instanceof Error ? err.message : String(err),
      });
    }
  };

  const newSticky = async () => {
    try {
      await ipc.newStickyWindow();
    } catch (err) {
      console.error("[floaty] new sticky failed:", err);
    }
  };

  const openWelcome = async () => {
    try {
      await invoke("open_welcome");
    } catch (err) {
      console.error("[floaty] open welcome failed:", err);
    }
  };

  return (
    <div className="h-screen flex flex-col bg-white/95 dark:bg-zinc-900/95 text-sm relative select-none" style={{ color: "#1d1d1f" }}>
      {/* 操作反馈 Toast */}
      {toast && (
        <div className="fixed top-3 left-1/2 -translate-x-1/2 z-50 pointer-events-none animate-in fade-in slide-in-from-top-2 duration-200">
          <div className="px-3.5 py-1.5 rounded-full bg-black/85 dark:bg-white/90 text-white dark:text-black shadow-lg text-xs font-medium backdrop-blur-md flex items-center gap-1.5">
            <span>{toast}</span>
          </div>
        </div>
      )}

      {/* 恢复默认快捷键确认弹窗 */}
      {showResetConfirm && (
        <div className="fixed inset-0 z-50 bg-black/30 backdrop-blur-sm flex items-center justify-center p-4 animate-in fade-in duration-150">
          <div className="bg-white dark:bg-zinc-800 rounded-2xl p-5 max-w-xs w-full shadow-2xl border border-black/10 dark:border-white/10 flex flex-col gap-3">
            <div className="font-semibold text-sm text-black dark:text-white">重置快捷键</div>
            <div className="text-xs text-black/60 dark:text-white/60 leading-relaxed">
              确定要将所有快捷键恢复为默认设置吗？
            </div>
            <div className="flex gap-2 justify-end pt-2">
              <button
                className="px-3 h-7 rounded-lg border border-black/10 dark:border-white/10 hover:bg-black/5 dark:hover:bg-white/5 text-xs font-medium transition active:scale-95 text-black/80 dark:text-white/80"
                onClick={() => setShowResetConfirm(false)}
              >
                取消
              </button>
              <button
                className="px-3 h-7 rounded-lg bg-black hover:bg-black/85 text-white text-xs font-medium shadow-sm transition active:scale-95"
                onClick={handleConfirmReset}
              >
                恢复默认
              </button>
            </div>
          </div>
        </div>
      )}

      <div className="px-4 py-3 border-b border-black/5 bg-black/[0.01]">
        <div className="font-semibold text-[15px] tracking-tight">Floaty 设置</div>
        <div className="text-[11px] opacity-50 mt-0.5">极简的 macOS 桌面便签式 Todo</div>
      </div>

      <div className="flex-1 overflow-auto p-4 space-y-5 bg-black/[0.01]">
        {/* 统计 */}
        <section className="space-y-2">
          <div className="text-[10px] font-semibold uppercase tracking-wider opacity-40 px-1">运行统计</div>
          <div className="grid grid-cols-3 gap-3">
            <Stat label="便签" value={stats?.stickies ?? "-"} />
            <Stat label="Todo 条数" value={stats?.items ?? "-"} />
            <Stat label="待触发提醒" value={stats?.pending_reminders ?? "-"} />
          </div>
        </section>

        {/* 启动与快捷键 */}
        <section className="space-y-2">
          <div className="text-[10px] font-semibold uppercase tracking-wider opacity-40 px-1">通用设置</div>
          <div className="rounded-xl border border-black/5 bg-white shadow-sm p-3 divide-y divide-black/5">
            <div className="flex items-center justify-between py-2">
              <div className="flex flex-col gap-0.5">
                <span className="text-xs font-semibold text-black/85">开机自动启动</span>
                <span className="text-[10px] opacity-50 font-medium">macOS LaunchAgent 自动加载</span>
              </div>
              <label className="ios-switch">
                <input
                  type="checkbox"
                  checked={autostart}
                  onChange={(e) => toggleAutostart(e.target.checked)}
                />
                <span className="ios-slider" />
              </label>
            </div>
            
            <div className="flex items-center justify-between py-2">
              <div className="flex flex-col gap-0.5">
                <span className="text-xs font-semibold text-black/85">全局新建便签快捷键</span>
                <span className="text-[10px] opacity-50 font-medium">
                  {IS_MAC ? "⌘⇧N 全局呼出新建" : "Ctrl+Shift+N 全局呼出新建"}
                </span>
              </div>
              <label className="ios-switch">
                <input
                  type="checkbox"
                  checked={globalEnabled}
                  onChange={(e) => toggleGlobal(e.target.checked)}
                />
                <span className="ios-slider" />
              </label>
            </div>

            <div className="flex items-center justify-between py-2">
              <div className="flex flex-col gap-0.5">
                <span className="text-xs font-semibold text-black/85">文本选区快捷气泡菜单</span>
                <span className="text-[10px] opacity-50 font-medium">选中文字时上方显示格式栏</span>
              </div>
              <label className="ios-switch">
                <input
                  type="checkbox"
                  checked={bubbleEnabled}
                  onChange={(e) => toggleBubble(e.target.checked)}
                />
                <span className="ios-slider" />
              </label>
            </div>

            <div className="flex items-center justify-between py-2">
              <div className="flex flex-col gap-0.5">
                <span className="text-xs font-semibold text-black/85">便签底部悬浮快捷栏</span>
                <span className="text-[10px] opacity-50 font-medium">在便签底部显示常用格式与时间按钮</span>
              </div>
              <label className="ios-switch">
                <input
                  type="checkbox"
                  checked={floatingEnabled}
                  onChange={(e) => toggleFloating(e.target.checked)}
                />
                <span className="ios-switch-track">
                  <span className="ios-switch-thumb" />
                </span>
              </label>
            </div>
          </div>
        </section>

        {/* 数据备份 */}
        <section className="space-y-2">
          <div className="text-[10px] font-semibold uppercase tracking-wider opacity-40 px-1">数据存储与备份</div>
          <div className="rounded-xl border border-black/5 bg-white shadow-sm p-3 space-y-3">
            <div>
              <div className="text-[10px] opacity-50 mb-1">当前数据目录</div>
              <div className="text-[10.5px] font-mono break-all p-2 bg-black/[0.03] rounded-lg border border-black/[0.02]">{dataDir}</div>
            </div>
            <div className="flex gap-2 pt-1">
              <button
                className="text-xs px-3 h-8 rounded-lg border border-black/10 hover:bg-black/5 transition active:scale-[0.97]"
                onClick={openDataDir}
              >
                {IS_MAC ? "在 Finder 中显示" : "在资源管理器中显示"}
              </button>
              <button
                className="text-xs px-3 h-8 rounded-lg bg-black text-white hover:bg-black/85 disabled:opacity-50 transition active:scale-[0.97]"
                onClick={runBackup}
                disabled={backupStatus.kind === "running"}
              >
                {backupStatus.kind === "running" ? "正在备份..." : "立即备份..."}
              </button>
            </div>
            {backupStatus.kind === "done" && (
              <div className="mt-2 p-2.5 rounded-lg bg-emerald-50 text-[10.5px] text-emerald-800 border border-emerald-100 flex flex-col gap-1.5">
                <span className="font-semibold">备份成功：</span>
                <span className="font-mono break-all opacity-90">{backupStatus.path}</span>
                <button
                  className="self-start text-[10px] px-2 h-6 rounded border border-emerald-200 bg-white hover:bg-emerald-50 text-emerald-800 transition active:scale-[0.97]"
                  onClick={() => revealBackup(backupStatus.path)}
                >
                  {IS_MAC ? "在 Finder 打开" : "打开所在文件夹"}
                </button>
              </div>
            )}
            {backupStatus.kind === "error" && (
              <div className="mt-2 p-2 rounded-lg bg-red-50 text-[10.5px] text-red-800 border border-red-100 font-medium">
                备份失败：{backupStatus.message}
              </div>
            )}
          </div>
        </section>

        {/* 关于和更新 */}
        <section className="space-y-2">
          <div className="text-[10px] font-semibold uppercase tracking-wider opacity-40 px-1">关于 Floaty</div>
          <div className="rounded-xl border border-black/5 bg-white shadow-sm p-3 flex flex-col gap-2.5">
            <div className="flex justify-between items-center text-xs">
              <span className="font-semibold text-black/85">软件版本</span>
              <span className="font-mono bg-black/[0.04] px-2 py-0.5 rounded-md border border-black/[0.02]">{currentVersion || "—"}</span>
            </div>
            <div className="pt-2 border-t border-black/5">
              <UpdatePanel
                status={updateStatus}
                onCheck={checkForUpdate}
                onInstall={downloadAndInstall}
              />
            </div>
          </div>
        </section>

        {/* 快捷键 */}
        <section className="space-y-2">
          <div className="text-[10px] font-semibold uppercase tracking-wider opacity-40 px-1">快捷键</div>
          <div className="rounded-xl border border-black/5 bg-white shadow-sm p-3 divide-y divide-black/5 text-xs">
            {DEFAULT_SHORTCUTS.map((def) => {
              const current = shortcuts[def.id] !== undefined ? shortcuts[def.id] : def.defaultCombo;
              return (
                <div key={def.id} className="py-2.5 flex items-center justify-between gap-3">
                  <div className="flex flex-col gap-0.5">
                    <span className="font-semibold text-black/85 text-xs">{def.label}</span>
                    <span className="text-[10px] opacity-45">{def.description}</span>
                  </div>
                  <ShortcutRecorder
                    value={current}
                    onChange={(combo) => handleShortcutChange(def.id, combo)}
                    onInvalidCombo={(msg) => showToast(msg)}
                  />
                </div>
              );
            })}
          </div>

          {/* 恢复默认快捷键 */}
          <div className="rounded-xl border border-black/5 bg-white shadow-sm px-3 py-2.5 flex items-center justify-between">
            <span className="text-xs font-medium text-black/75">恢复默认快捷键</span>
            <button
              className="text-xs px-2.5 h-7 rounded-lg border border-black/10 hover:bg-black/5 text-black/75 hover:text-black transition active:scale-95"
              onClick={() => setShowResetConfirm(true)}
            >
              恢复默认
            </button>
          </div>
        </section>

        {/* 操作与帮助 */}
        <section className="space-y-2.5">
          <div className="text-[10px] font-semibold uppercase tracking-wider opacity-40 px-1">日常操作</div>
          <div className="flex gap-2 flex-wrap">
            <button
              className="text-xs px-3 h-8 rounded-lg bg-black text-white hover:bg-black/85 transition active:scale-[0.97]"
              onClick={newSticky}
            >
              ＋ 新建便签
            </button>
            <button
              className="text-xs px-3 h-8 rounded-lg border border-black/10 hover:bg-black/5 transition active:scale-[0.97]"
              onClick={openWelcome}
            >
              📖 打开上手指南
            </button>
          </div>
          <div className="pt-3 border-t border-black/5 text-[10px] opacity-45 leading-relaxed">
            关闭所有便签后 App 仍在 macOS 菜单栏常驻。点击菜单栏图标可随时恢复或新建便签。
          </div>
        </section>
      </div>
    </div>
  );
}

function Stat({ label, value }: { label: string; value: number | string }) {
  return (
    <div className="rounded-xl border border-black/5 bg-white shadow-sm p-3 text-center transition hover:shadow-md hover:scale-[1.01]">
      <div className="text-[10px] font-semibold uppercase tracking-wider opacity-45 mb-1">{label}</div>
      <div className="text-xl font-bold tracking-tight text-black">{value}</div>
    </div>
  );
}


function UpdatePanel({
  status,
  onCheck,
  onInstall,
}: {
  status: UpdateStatus;
  onCheck: () => void;
  onInstall: (u: Update) => void;
}) {
  const btn =
    "text-xs px-3 h-8 rounded-lg border border-black/10 hover:bg-black/5 disabled:opacity-50 transition active:scale-[0.97]";

  if (status.kind === "idle") {
    return (
      <button className={btn} onClick={onCheck}>
        检查更新
      </button>
    );
  }
  if (status.kind === "checking") {
    return (
      <button className={btn} disabled>
        正在检查更新...
      </button>
    );
  }
  if (status.kind === "up-to-date") {
    return (
      <div className="flex items-center gap-2">
        <span className="text-xs opacity-55 font-medium">已是最新版本</span>
        <button className={btn} onClick={onCheck}>
          重新检查
        </button>
      </div>
    );
  }
  if (status.kind === "available") {
    const u = status.update;
    return (
      <div className="space-y-2">
        <div className="text-xs font-semibold">
          发现新版本 <span className="font-mono bg-black/[0.04] px-1.5 py-0.5 rounded-md border border-black/[0.02]">{u.version}</span>
          {u.date && (
            <span className="opacity-55 ml-2 text-[10px]">{u.date.split(" ")[0]}</span>
          )}
        </div>
        {u.body && (
          <div className="text-[11px] opacity-75 whitespace-pre-wrap p-2.5 bg-black/[0.03] border border-black/[0.02] rounded-lg max-h-32 overflow-auto leading-relaxed">
            {u.body}
          </div>
        )}
        <button className="text-xs px-3 h-8 rounded-lg bg-black text-white hover:bg-black/85 transition active:scale-[0.97]" onClick={() => onInstall(u)}>
          下载并安装更新
        </button>
      </div>
    );
  }
  if (status.kind === "downloading") {
    const pct =
      status.total && status.total > 0
        ? Math.round((status.received / status.total) * 100)
        : null;
    return (
      <div className="space-y-1.5">
        <div className="text-xs font-semibold opacity-70">
          下载中... {pct !== null ? `${pct}%` : `${(status.received / 1024 / 1024).toFixed(1)} MB`}
        </div>
        <div className="h-1.5 bg-black/5 rounded-full overflow-hidden">
          <div
            className="h-full bg-black transition-all duration-300"
            style={{ width: pct !== null ? `${pct}%` : "33%" }}
          />
        </div>
      </div>
    );
  }
  if (status.kind === "installed") {
    return <div className="text-xs font-medium opacity-65">安装完成，正在重新启动...</div>;
  }
  return (
    <div className="space-y-2">
      <div className="text-xs text-red-600 font-semibold">更新失败：{status.message}</div>
      <button className={btn} onClick={onCheck}>
        重试
      </button>
    </div>
  );
}
