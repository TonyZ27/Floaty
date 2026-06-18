import { useState } from "react";
import type { Sticky, StickyPatch } from "../ipc/types";
import { autoFg } from "../theme/contrast";
import { X, RotateCcw, Trash2 } from "lucide-react";

interface SettingsPopoverProps {
  sticky: Sticky;
  onPatch: (patch: StickyPatch) => void;
  onClose: () => void;
  onDelete: () => void;
}

const PRESET_COLORS = [
  { name: "黄", value: "#FFDC96" },
  { name: "薄荷", value: "#B4E6C8" },
  { name: "薰衣", value: "#C8C8FF" },
  { name: "粉红", value: "#FFB4B4" },
  { name: "杏", value: "#FFE6B4" },
  { name: "天蓝", value: "#B4D8F0" },
  { name: "紫", value: "#E6B4FF" },
  { name: "灰", value: "#D0D0D0" },
];

/// 一键切整套外观（bg + 字色 + 透明度 + 字号）
interface Theme {
  name: string;
  bg: string;
  fg: string;
  opacity: number;
  font_size: number;
}
const THEMES: Theme[] = [
  { name: "默认",   bg: "#FFDC96", fg: "#3A2E10", opacity: 0.85, font_size: 14 },
  { name: "纸质",   bg: "#F5ECD9", fg: "#3A2E10", opacity: 0.95, font_size: 14 },
  { name: "薄荷",   bg: "#B4E6C8", fg: "#1F3A2A", opacity: 0.88, font_size: 14 },
  { name: "黄昏",   bg: "#FFB4A0", fg: "#3A1E14", opacity: 0.90, font_size: 14 },
  { name: "石板",   bg: "#3A3A42", fg: "#F0F0F0", opacity: 0.95, font_size: 14 },
  { name: "柠檬",   bg: "#FFF6A8", fg: "#403800", opacity: 0.88, font_size: 15 },
];

export function SettingsPopover({ sticky, onPatch, onClose, onDelete }: SettingsPopoverProps) {
  // "auto" = 字体颜色是不是等于 autoFg(bg)；此状态是 UI 级（不入 DB）
  const [fontColorAuto, setFontColorAuto] = useState(
    sticky.font_color == null ||
      sticky.font_color.toLowerCase() === autoFg(sticky.bg_color).toLowerCase(),
  );

  const applyBg = (bg: string) => {
    const patch: StickyPatch = { bg_color: bg };
    if (fontColorAuto) patch.font_color = autoFg(bg);
    onPatch(patch);
  };

  const toggleAuto = (auto: boolean) => {
    setFontColorAuto(auto);
    if (auto) {
      onPatch({ font_color: autoFg(sticky.bg_color) });
    }
    // auto=false：保留当前颜色，用户可通过 HEX 自己改
  };

  const reset = () => {
    setFontColorAuto(true);
    onPatch({
      bg_color: "#FFDC96",
      opacity: 0.85,
      font_size: 14,
      font_color: "#3a2e10", // autoFg("#FFDC96") = "#3a2e10"
    });
  };

  return (
    <div
      className="absolute top-[38px] left-2 right-2 z-10 glass-panel rounded-xl overflow-hidden flex flex-col transition-all duration-200"
      style={{
        color: "#1d1d1f",
        maxHeight: "calc(100vh - 46px)",
        maxWidth: "280px",
        marginLeft: "auto",
      }}
      onClick={(e) => e.stopPropagation()}
      onMouseDown={(e) => e.stopPropagation()}
    >
      <div className="flex items-center justify-between px-3 py-2.5 border-b border-black/5 flex-shrink-0">
        <strong className="text-xs font-semibold">外观设置</strong>
        <button
          className="w-5 h-5 flex items-center justify-center rounded-full hover:bg-black/5 opacity-60 hover:opacity-100 transition-all active:scale-90"
          onClick={onClose}
        >
          <X size={12} strokeWidth={2.5} />
        </button>
      </div>

      <div className="p-3 space-y-3.5 text-xs overflow-y-auto">
        {/* 便签标题 */}
        <div>
          <div className="text-[10px] font-semibold uppercase tracking-wider opacity-50 mb-1.5">便签标题</div>
          <input
            type="text"
            placeholder="未命名 (自动从第一行提取)"
            value={sticky.title}
            onChange={(e) => onPatch({ title: e.target.value })}
            className="w-full h-8 px-2.5 outline-none text-xs glass-input"
          />
        </div>

        {/* 主题 — 一键切整套配色 */}
        <div>
          <div className="text-[10px] font-semibold uppercase tracking-wider opacity-50 mb-1.5">快捷主题</div>
          <div className="grid grid-cols-3 gap-1.5">
            {THEMES.map((t) => {
              const selected =
                sticky.bg_color.toLowerCase() === t.bg.toLowerCase() &&
                (sticky.font_color ?? autoFg(sticky.bg_color)).toLowerCase() === t.fg.toLowerCase();
              return (
                <button
                  key={t.name}
                  className={`h-11 rounded-lg border-2 text-[10px] font-semibold flex items-center justify-center transition active:scale-[0.95] ${
                    selected ? "border-black/45 shadow-sm" : "border-transparent hover:border-black/10"
                  }`}
                  style={{ backgroundColor: t.bg, color: t.fg }}
                  onClick={() => {
                    setFontColorAuto(false); // 主题明确设了 fg，锁成自定义
                    onPatch({
                      bg_color: t.bg,
                      font_color: t.fg,
                      opacity: t.opacity,
                      font_size: t.font_size,
                    });
                  }}
                  title={`${t.name} · ${Math.round(t.opacity * 100)}% · ${t.font_size}px`}
                >
                  {t.name}
                </button>
              );
            })}
          </div>
        </div>

        {/* 背景色 */}
        <div>
          <div className="text-[10px] font-semibold uppercase tracking-wider opacity-50 mb-1.5">背景色</div>
          <div className="grid grid-cols-8 gap-1.5">
            {PRESET_COLORS.map((c) => (
              <button
                key={c.value}
                className={`h-5 w-5 rounded-full border-2 transition active:scale-[0.9] ${
                  sticky.bg_color.toLowerCase() === c.value.toLowerCase()
                    ? "border-black/50 scale-110 shadow-sm"
                    : "border-transparent hover:scale-105"
                }`}
                style={{ backgroundColor: c.value }}
                onClick={() => applyBg(c.value)}
                title={c.name}
              />
            ))}
          </div>
          <div className="mt-2.5 flex items-center gap-2">
            <div className="relative w-8 h-6 rounded-md overflow-hidden border border-black/10 active:scale-[0.95] transition">
              <input
                type="color"
                value={sticky.bg_color}
                onChange={(e) => applyBg(e.target.value.toUpperCase())}
                className="absolute inset-0 w-full h-full p-0 border-0 cursor-pointer scale-150 bg-transparent"
              />
            </div>
            <span className="font-mono text-[10px] font-medium opacity-65 tracking-wider">{sticky.bg_color}</span>
          </div>
        </div>

        {/* 透明度 */}
        <div>
          <div className="flex items-center justify-between mb-1">
            <span className="text-[10px] font-semibold uppercase tracking-wider opacity-50">透明度</span>
            <span className="font-mono text-[10px] font-medium opacity-65">
              {Math.round(sticky.opacity * 100)}%
            </span>
          </div>
          <input
            type="range"
            min={30}
            max={100}
            step={5}
            value={Math.round(sticky.opacity * 100)}
            onChange={(e) => onPatch({ opacity: Number(e.target.value) / 100 })}
            className="w-full accent-black"
          />
        </div>

        {/* 字号 */}
        <div>
          <div className="text-[10px] font-semibold uppercase tracking-wider opacity-50 mb-1.5">字体大小</div>
          <div className="flex gap-1 bg-black/[0.04] p-0.5 rounded-lg border border-black/[0.02]">
            {[
              { label: "小", val: 12 },
              { label: "中", val: 14 },
              { label: "大", val: 16 },
            ].map((o) => (
              <button
                key={o.val}
                className={`flex-1 h-7 rounded-md text-xs font-semibold transition active:scale-[0.97] ${
                  sticky.font_size === o.val
                    ? "bg-white shadow-sm border border-black/[0.02] text-black"
                    : "text-black/60 hover:text-black"
                }`}
                onClick={() => onPatch({ font_size: o.val })}
              >
                <span>{o.label}</span>
              </button>
            ))}
          </div>
        </div>

        {/* 字体颜色 */}
        <div>
          <div className="text-[10px] font-semibold uppercase tracking-wider opacity-50 mb-1.5">字体颜色</div>
          <div className="flex gap-1 bg-black/[0.04] p-0.5 rounded-lg border border-black/[0.02] mb-2.5">
            <button
              className={`flex-1 h-6 rounded-md text-[11px] font-semibold transition active:scale-[0.97] ${
                fontColorAuto
                  ? "bg-white shadow-sm border border-black/[0.02] text-black"
                  : "text-black/50 hover:text-black"
              }`}
              onClick={() => toggleAuto(true)}
            >
              自动
            </button>
            <button
              className={`flex-1 h-6 rounded-md text-[11px] font-semibold transition active:scale-[0.97] ${
                !fontColorAuto
                  ? "bg-white shadow-sm border border-black/[0.02] text-black"
                  : "text-black/50 hover:text-black"
              }`}
              onClick={() => toggleAuto(false)}
            >
              自定义
            </button>
          </div>
          <div className="flex items-center gap-2">
            <div className={`relative w-8 h-6 rounded-md overflow-hidden border border-black/10 active:scale-[0.95] transition ${fontColorAuto ? "opacity-40 cursor-not-allowed" : ""}`}>
              <input
                type="color"
                value={sticky.font_color ?? autoFg(sticky.bg_color)}
                disabled={fontColorAuto}
                onChange={(e) => onPatch({ font_color: e.target.value.toUpperCase() })}
                className="absolute inset-0 w-full h-full p-0 border-0 cursor-pointer scale-150 bg-transparent"
              />
            </div>
            <span className="font-mono text-[10px] font-medium opacity-65 tracking-wider">
              {(sticky.font_color ?? autoFg(sticky.bg_color)).toUpperCase()}
              {fontColorAuto && <span className="ml-1 opacity-60">（自动）</span>}
            </span>
          </div>
        </div>

        {/* 重置 + 删除 */}
        <div className="pt-3 border-t border-black/5 space-y-2">
          <button
            className="w-full h-8 rounded-lg border border-black/10 hover:bg-black/5 text-xs font-medium flex items-center justify-center gap-1 transition active:scale-[0.98]"
            onClick={reset}
          >
            <RotateCcw size={12} />
            重置默认
          </button>
          <button
            className="w-full h-8 rounded-lg bg-red-500/10 text-red-600 hover:bg-red-500 hover:text-white text-xs font-medium flex items-center justify-center gap-1 transition active:scale-[0.98]"
            onClick={onDelete}
          >
            <Trash2 size={12} />
            删除便签...
          </button>
        </div>
      </div>
    </div>
  );
}
