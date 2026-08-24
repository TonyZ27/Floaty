import { useState } from "react";
import { X } from "lucide-react";

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
  const FULL_HEIGHT = 400;

  const x = Math.max(MARGIN, Math.min(anchor.left, vw - PICKER_WIDTH - MARGIN));

  const spaceBelow = vh - anchor.bottom - GAP - MARGIN;
  const spaceAbove = anchor.top - GAP - MARGIN;

  // 1. 如果下方空间足够完整显示面板，直接在下方展开
  if (spaceBelow >= FULL_HEIGHT) {
    const top = anchor.bottom + GAP;
    return {
      x,
      top,
      maxHeight: vh - top - MARGIN,
    };
  }

  // 2. 如果上方空间足够完整显示面板，在上方展开
  if (spaceAbove >= FULL_HEIGHT) {
    return {
      x,
      bottom: vh - anchor.top + GAP,
      maxHeight: spaceAbove,
    };
  }

  // 3. 超小视口保护 (高度 <= 220px)
  if (vh <= 220) {
    return {
      x,
      top: MARGIN,
      maxHeight: Math.max(60, vh - MARGIN * 2),
    };
  }

  // 4. 哪边空间大选哪边，并根据可用空间限制 maxHeight 与位置
  if (spaceBelow >= spaceAbove) {
    const top = anchor.bottom + GAP;
    const maxHeight = Math.max(100, spaceBelow);
    const clampedTop = Math.min(top, vh - maxHeight - MARGIN);
    const finalTop = Math.max(MARGIN, clampedTop);
    return {
      x,
      top: finalTop,
      maxHeight: Math.min(maxHeight, vh - finalTop - MARGIN),
    };
  } else {
    const maxHeight = Math.max(100, spaceAbove);
    const bottom = vh - anchor.top + GAP;
    const clampedBottom = Math.min(bottom, vh - maxHeight - MARGIN);
    const finalBottom = Math.max(MARGIN, clampedBottom);
    return {
      x,
      bottom: finalBottom,
      maxHeight: Math.min(maxHeight, vh - finalBottom - MARGIN),
    };
  }
}

interface DueTimePickerProps {
  anchor?: AnchorRect;
  x?: number; // legacy fallback
  y?: number;
  onPick: (iso: string) => void;
  onCancel: () => void;
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

const COUNTDOWN_CHIPS = [
  { label: "15分钟", minutes: 15 },
  { label: "30分钟", minutes: 30 },
  { label: "1小时", minutes: 60 },
  { label: "2小时", minutes: 120 },
];

export function DueTimePicker({ anchor, x, y, onPick, onCancel }: DueTimePickerProps) {
  const [custom, setCustom] = useState<string>(() => {
    const d = new Date();
    d.setMinutes(0, 0, 0);
    d.setHours(d.getHours() + 1);
    return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}T${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`;
  });
  const [countdownVal, setCountdownVal] = useState<string>("30");
  const [countdownUnit, setCountdownUnit] = useState<"min" | "hr">("min");
  const options = makeQuickOptions();

  const pos = anchor
    ? computePickerPosition(anchor)
    : computePickerPosition({ left: x ?? 20, top: (y ?? 24) - 4, bottom: y ?? 24 });

  const pickCountdown = (minutes: number) => {
    const d = new Date(Date.now() + minutes * 60_000);
    onPick(toIso(d));
  };

  const pickCustomCountdown = () => {
    const n = Number(countdownVal);
    if (!n || n <= 0) return;
    const minutes = countdownUnit === "hr" ? n * 60 : n;
    pickCountdown(minutes);
  };

  return (
    <div
      className="fixed z-50 glass-panel rounded-xl flex flex-col shadow-2xl overflow-hidden transition-all duration-200"
      style={{
        left: pos.x,
        ...(pos.top !== undefined ? { top: pos.top } : { bottom: pos.bottom }),
        maxHeight: `${pos.maxHeight}px`,
        width: "220px",
        maxWidth: "calc(100vw - 16px)",
        color: "#1d1d1f",
      }}
      onMouseDown={(e) => e.stopPropagation()}
      onClick={(e) => e.stopPropagation()}
    >
      <div className="flex items-center justify-between px-3 py-2 border-b border-black/5 text-[10px] font-semibold uppercase tracking-wider opacity-50 flex-shrink-0">
        <span>选择截止时间</span>
        <button
          className="w-5 h-5 flex items-center justify-center rounded-full hover:bg-black/5 opacity-60 hover:opacity-100 transition active:scale-90"
          onClick={onCancel}
          title="取消 (Esc)"
        >
          <X size={11} strokeWidth={2.5} />
        </button>
      </div>
      <div className="py-1.5 text-xs overflow-y-auto flex-1 min-h-0">
        {options.map((opt) => (
          <button
            key={opt.label}
            className="w-full text-left px-3 py-2 hover:bg-black/5 flex justify-between items-center transition-colors duration-100"
            onClick={() => onPick(toIso(opt.date))}
          >
            <span className="font-medium">{opt.label}</span>
            <span className="text-[9px] opacity-40 font-mono tracking-tighter">{formatPreview(opt.date)}</span>
          </button>
        ))}

        <div className="px-3 py-2.5 mt-1.5 border-t border-black/5">
          <div className="text-[10px] font-semibold uppercase tracking-wider opacity-50 mb-1.5">倒计时</div>
          <div className="grid grid-cols-2 gap-1.5 mb-2">
            {COUNTDOWN_CHIPS.map((c) => (
              <button
                key={c.label}
                className="px-2 h-7 rounded-lg border border-black/10 hover:bg-black/5 hover:border-black/20 text-[11px] font-medium transition active:scale-[0.96]"
                onClick={() => pickCountdown(c.minutes)}
              >
                {c.label}
              </button>
            ))}
          </div>
          <div className="flex gap-1 items-center">
            <input
              type="number"
              min={1}
              value={countdownVal}
              onChange={(e) => setCountdownVal(e.target.value)}
              className="w-14 h-7 text-xs border border-black/10 rounded-lg px-2 outline-none focus:border-black/30 bg-white/50"
            />
            <select
              value={countdownUnit}
              onChange={(e) => setCountdownUnit(e.target.value as "min" | "hr")}
              className="text-xs border border-black/10 rounded-lg px-1 py-0.5 bg-white/50 h-7 outline-none"
            >
              <option value="min">分钟</option>
              <option value="hr">小时</option>
            </select>
            <button
              className="flex-1 text-[11px] h-7 rounded-lg bg-black text-white hover:bg-black/85 font-medium transition active:scale-[0.96]"
              onClick={pickCustomCountdown}
            >
              确定
            </button>
          </div>
        </div>
      </div>
      <div className="px-3 py-2.5 border-t border-black/5 flex-shrink-0">
        <div className="text-[10px] font-semibold uppercase tracking-wider opacity-50 mb-1.5">指定日期时间</div>
        <input
          type="datetime-local"
          value={custom}
          onChange={(e) => setCustom(e.target.value)}
          className="w-full text-xs border border-black/10 rounded-lg px-2 py-1 bg-white/50 h-8 outline-none focus:border-black/30"
        />
        <button
          className="mt-2 w-full text-xs h-8 rounded-lg bg-black text-white hover:bg-black/85 font-semibold transition active:scale-[0.96]"
          onClick={() => {
            const d = new Date(custom);
            if (!isNaN(d.getTime())) onPick(toIso(d));
          }}
        >
          确定
        </button>
      </div>
    </div>
  );
}

function formatPreview(d: Date): string {
  const pad = (n: number) => (n < 10 ? `0${n}` : `${n}`);
  return `${pad(d.getMonth() + 1)}/${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}`;
}
