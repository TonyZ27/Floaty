import { useState, useRef, useEffect } from "react";
import { X } from "lucide-react";
import { KeyCombo } from "./types";
import { formatKeyCombo, isValidCombo, eventToKeyCombo } from "./utils";

export interface ShortcutRecorderProps {
  value: KeyCombo | null;
  onChange: (value: KeyCombo | null) => void;
  onInvalidCombo?: (message: string) => void;
}

export function ShortcutRecorder({
  value,
  onChange,
  onInvalidCombo,
}: ShortcutRecorderProps) {
  const [recording, setRecording] = useState(false);
  const [liveModifiers, setLiveModifiers] = useState<string[]>([]);
  const containerRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!recording) return;

    const handleKeyDown = (e: KeyboardEvent) => {
      e.preventDefault();
      e.stopPropagation();

      // Escape 退出录入
      if (e.key === "Escape") {
        setRecording(false);
        setLiveModifiers([]);
        return;
      }

      const res = eventToKeyCombo(e);
      if (!res) return;

      if (res.isModifierOnly) {
        const mods: string[] = [];
        if (res.combo.ctrl) mods.push("⌃");
        if (res.combo.alt) mods.push("⌥");
        if (res.combo.shift) mods.push("⇧");
        if (res.combo.meta) mods.push("⌘");
        setLiveModifiers(mods);
        return;
      }

      // 非仅修饰键时，校验是否包含 Ctrl / Alt / Cmd
      if (!isValidCombo(res.combo)) {
        onInvalidCombo?.("快捷键必须包含 Control、Option 或 Command");
        return;
      }

      // 录入成功
      onChange(res.combo);
      setRecording(false);
      setLiveModifiers([]);
    };

    const handleKeyUp = (e: KeyboardEvent) => {
      const isModifierKey = ["Control", "Alt", "Shift", "Meta", "Command", "Option", "Super"].includes(e.key);
      if (isModifierKey) {
        const mods: string[] = [];
        if (e.ctrlKey) mods.push("⌃");
        if (e.altKey) mods.push("⌥");
        if (e.shiftKey) mods.push("⇧");
        if (e.metaKey) mods.push("⌘");
        setLiveModifiers(mods);
      }
    };

    const handleWindowClick = (e: MouseEvent) => {
      if (containerRef.current && !containerRef.current.contains(e.target as Node)) {
        setRecording(false);
        setLiveModifiers([]);
      }
    };

    window.addEventListener("keydown", handleKeyDown, true);
    window.addEventListener("keyup", handleKeyUp, true);
    window.addEventListener("mousedown", handleWindowClick, true);

    return () => {
      window.removeEventListener("keydown", handleKeyDown, true);
      window.removeEventListener("keyup", handleKeyUp, true);
      window.removeEventListener("mousedown", handleWindowClick, true);
    };
  }, [recording, onChange, onInvalidCombo]);

  const handleClear = (e: React.MouseEvent) => {
    e.stopPropagation();
    onChange(null);
    setRecording(false);
    setLiveModifiers([]);
  };

  const displayText = recording
    ? liveModifiers.length > 0
      ? `${liveModifiers.join(" + ")} + ...`
      : "请按下快捷键..."
    : formatKeyCombo(value);

  const hasValue = Boolean(value && value.key);

  return (
    <div
      ref={containerRef}
      tabIndex={0}
      className={`group relative h-7 min-w-[110px] px-2.5 rounded-lg text-xs font-mono select-none cursor-pointer flex items-center justify-between transition-all duration-150 border ${
        recording
          ? "bg-white dark:bg-zinc-800 border-black/30 dark:border-white/40 ring-2 ring-black/10 dark:ring-white/20 text-black dark:text-white"
          : "bg-black/[0.04] dark:bg-white/[0.06] hover:bg-black/[0.08] dark:hover:bg-white/[0.1] border-black/[0.04] dark:border-white/[0.06] text-black/85 dark:text-white/85"
      }`}
      onClick={() => setRecording(true)}
    >
      <span className={`tracking-tight ${!hasValue && !recording ? "opacity-40 italic font-sans" : "font-medium"}`}>
        {displayText}
      </span>

      {/* Hover 或聚焦时显示清除按钮 */}
      {hasValue && (
        <button
          type="button"
          className="ml-2 w-4 h-4 rounded-full bg-black/10 dark:bg-white/15 hover:bg-black/20 dark:hover:bg-white/25 text-black/50 dark:text-white/50 hover:text-black dark:hover:text-white flex items-center justify-center opacity-0 group-hover:opacity-100 group-focus-within:opacity-100 transition-opacity active:scale-90"
          onClick={handleClear}
          title="清除快捷键"
        >
          <X size={10} strokeWidth={2.5} />
        </button>
      )}
    </div>
  );
}
