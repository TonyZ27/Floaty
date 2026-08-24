import { useState } from "react";
import { ArrowLeft, ChevronRight } from "lucide-react";
import { PRIORITY_CONFIG, PriorityLevel } from "./Priority";

export interface AnchorRect {
  left: number;
  top: number;
  bottom: number;
}

export interface PickerPosition {
  x: number;
  top?: number;
  bottom?: number;
  maxHeight: number;
}

export function computePickerPosition(
  anchor: AnchorRect,
  vw: number = typeof window !== "undefined" ? window.innerWidth : 800,
  vh: number = typeof window !== "undefined" ? window.innerHeight : 600,
): PickerPosition {
  const PICKER_WIDTH = 220;
  const MARGIN = 8;
  const GAP = 4;
  const FULL_HEIGHT = 380;

  // 1. 水平对齐：左对齐于光标，且不超出视口右侧和左侧
  let x = anchor.left;
  if (x + PICKER_WIDTH > vw - MARGIN) {
    x = Math.max(MARGIN, vw - PICKER_WIDTH - MARGIN);
  }
  if (x < MARGIN) {
    x = MARGIN;
  }

  // 2. 垂直对齐：优先向下展开
  const spaceBelow = vh - anchor.bottom - GAP - MARGIN;
  const spaceAbove = anchor.top - GAP - MARGIN;

  if (spaceBelow >= 200 || spaceBelow >= spaceAbove) {
    return {
      x,
      top: anchor.bottom + GAP,
      maxHeight: Math.min(FULL_HEIGHT, Math.max(120, spaceBelow)),
    };
  } else {
    return {
      x,
      bottom: vh - anchor.top + GAP,
      maxHeight: Math.min(FULL_HEIGHT, Math.max(120, spaceAbove)),
    };
  }
}

export type InsertChoice =
  | { kind: "priority"; level: PriorityLevel }
  | { kind: "dueTime"; iso: string };

export interface InsertPickerProps {
  anchor?: AnchorRect;
  x?: number; // legacy fallback
  y?: number;
  onPick: (choice: InsertChoice) => void;
}

function toIso(d: Date): string {
  // Output as local ISO with timezone offset, e.g. 2026-04-20T22:00:00+08:00
  const pad = (n: number) => (n < 10 ? `0${n}` : `${n}`);
  const offMin = -d.getTimezoneOffset();
  const offSign = offMin >= 0 ? "+" : "-";
  const offH = pad(Math.floor(Math.abs(offMin) / 60));
  const offM = pad(Math.abs(offMin) % 60);
  return (
    `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}` +
    `T${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}` +
    `${offSign}${offH}:${offM}`
  );
}

function makeQuickOptions(): { label: string; date: Date }[] {
  const now = new Date();
  const set = (base: Date, h: number, m = 0) => {
    const d = new Date(base);
    d.setHours(h, m, 0, 0);
    return d;
  };
  const today = now;
  const tmr = new Date(now);
  tmr.setDate(tmr.getDate() + 1);
  const daysUntilSat = (6 - now.getDay() + 7) % 7 || 7;
  const sat = new Date(now);
  sat.setDate(sat.getDate() + daysUntilSat);
  const nextMon = new Date(now);
  nextMon.setDate(nextMon.getDate() + ((1 - now.getDay() + 7) % 7 || 7));

  return [
    { label: "今天 18:00", date: set(today, 18) },
    { label: "今晚 22:00", date: set(today, 22) },
    { label: "明天 09:00", date: set(tmr, 9) },
    { label: "本周六 10:00", date: set(sat, 10) },
    { label: "下周一 09:00", date: set(nextMon, 9) },
  ];
}

const PRIORITY_OPTIONS: PriorityLevel[] = ["P0", "P1", "P2", "P3"];

export function InsertPicker({ anchor, x = 20, y = 20, onPick }: InsertPickerProps) {
  const [view, setView] = useState<"main" | "custom-datetime">("main");
  const [custom, setCustom] = useState<string>(() => {
    const d = new Date();
    d.setMinutes(0, 0, 0);
    d.setHours(d.getHours() + 1);
    return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}T${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`;
  });

  const options = makeQuickOptions();
  const formatPreview = (d: Date) => {
    const pad = (n: number) => (n < 10 ? `0${n}` : `${n}`);
    return `${d.getMonth() + 1}/${d.getDate()} ${pad(d.getHours())}:${pad(d.getMinutes())}`;
  };

  const handleCustomSubmit = () => {
    if (!custom) return;
    const d = new Date(custom);
    if (!isNaN(d.getTime())) {
      onPick({ kind: "dueTime", iso: toIso(d) });
    }
  };

  const pos = anchor
    ? computePickerPosition(anchor)
    : { x, top: y, maxHeight: 380 };

  return (
    <div
      className="fixed z-50 rounded-xl shadow-2xl border border-black/10 bg-white/95 dark:bg-zinc-800/95 backdrop-blur-xl flex flex-col overflow-hidden animate-in fade-in zoom-in-95 duration-100 select-none"
      style={{
        left: `${pos.x}px`,
        ...(pos.top !== undefined ? { top: pos.top } : { bottom: pos.bottom }),
        maxHeight: `${pos.maxHeight}px`,
        width: "220px",
        maxWidth: "calc(100vw - 16px)",
        color: "#1d1d1f",
      }}
      onMouseDown={(e) => e.stopPropagation()}
      onClick={(e) => e.stopPropagation()}
    >
      {view === "main" ? (
        <div className="py-1 text-xs overflow-y-auto flex-1 min-h-0 divide-y divide-black/5">
          {/* 优先级 */}
          <div className="py-1">
            <div className="px-3 py-1 text-[10px] font-semibold uppercase tracking-wider opacity-40">
              优先级
            </div>
            {PRIORITY_OPTIONS.map((level) => {
              const conf = PRIORITY_CONFIG[level];
              return (
                <button
                  key={level}
                  className="w-full text-left px-3 py-1.5 hover:bg-black/5 flex items-center justify-between transition-colors duration-100"
                  onClick={() => onPick({ kind: "priority", level })}
                >
                  <div className="flex items-center gap-2">
                    <span
                      className="px-1.5 py-0.5 rounded text-[10px] font-semibold font-mono"
                      style={{
                        backgroundColor: conf.color.bg,
                        color: conf.color.fg,
                        border: `1px solid ${conf.color.border || "transparent"}`,
                      }}
                    >
                      {conf.badge}
                    </span>
                    <span className="font-medium text-xs">{conf.label}</span>
                  </div>
                </button>
              );
            })}
          </div>

          {/* 截止时间 */}
          <div className="py-1">
            <div className="px-3 py-1 text-[10px] font-semibold uppercase tracking-wider opacity-40">
              截止时间
            </div>
            {options.map((opt) => (
              <button
                key={opt.label}
                className="w-full text-left px-3 py-1.5 hover:bg-black/5 flex justify-between items-center transition-colors duration-100"
                onClick={() => onPick({ kind: "dueTime", iso: toIso(opt.date) })}
              >
                <span className="font-medium">{opt.label}</span>
                <span className="text-[9px] opacity-40 font-mono tracking-tighter">
                  {formatPreview(opt.date)}
                </span>
              </button>
            ))}

            <button
              className="w-full text-left px-3 py-1.5 hover:bg-black/5 flex justify-between items-center transition-colors duration-100 text-black/80 font-medium"
              onClick={() => setView("custom-datetime")}
            >
              <span>指定日期时间...</span>
              <ChevronRight size={13} className="opacity-40" />
            </button>
          </div>
        </div>
      ) : (
        <>
          <div className="flex items-center px-2 py-2 border-b border-black/5 text-[10px] font-semibold uppercase tracking-wider opacity-60 flex-shrink-0">
            <button
              className="w-5 h-5 flex items-center justify-center rounded-full hover:bg-black/5 mr-1 text-black/70 hover:text-black transition active:scale-90"
              onClick={() => setView("main")}
              title="返回"
            >
              <ArrowLeft size={12} strokeWidth={2.5} />
            </button>
            <span>指定日期时间</span>
          </div>

          <div className="p-3 text-xs flex flex-col gap-2.5 overflow-y-auto flex-1 min-h-0">
            <input
              type="datetime-local"
              className="w-full text-xs px-2.5 py-1.5 rounded-lg border border-black/10 bg-black/[0.02] focus:outline-none focus:border-black/30 font-mono"
              value={custom}
              onChange={(e) => setCustom(e.target.value)}
            />
            <button
              className="w-full h-7 rounded-lg bg-black text-white text-xs font-medium hover:bg-black/85 transition active:scale-[0.97]"
              onClick={handleCustomSubmit}
            >
              确定
            </button>
          </div>
        </>
      )}
    </div>
  );
}

// Re-export for backward compatibility
export { InsertPicker as DueTimePicker };
