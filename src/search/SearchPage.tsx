import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { ipc } from "../ipc/client";
import { Search, CornerDownLeft } from "lucide-react";

interface Hit {
  sticky_id: string;
  item_id: string;
  snippet: string;
  bg_color: string;
}

export function SearchPage() {
  const [query, setQuery] = useState("");
  const [hits, setHits] = useState<Hit[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [cursor, setCursor] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);
  const listRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    inputRef.current?.focus();
  }, []);

  useEffect(() => {
    const q = query.trim();
    if (!q) {
      setHits([]);
      setError(null);
      return;
    }
    let cancelled = false;
    setLoading(true);
    const timer = setTimeout(async () => {
      try {
        const rows = await invoke<Hit[]>("search_stickies", { query: q });
        if (!cancelled) {
          setHits(rows);
          setCursor(0);
          setError(null);
        }
      } catch (err) {
        if (!cancelled) {
          setError(err instanceof Error ? err.message : String(err));
          setHits([]);
        }
      } finally {
        if (!cancelled) setLoading(false);
      }
    }, 120);
    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  }, [query]);

  const openHit = async (hit: Hit) => {
    try {
      await ipc.showSticky(hit.sticky_id);
    } catch (err) {
      console.error("[floaty] open from search failed:", err);
    }
  };

  const onKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === "ArrowDown") {
      e.preventDefault();
      setCursor((c) => Math.min(c + 1, Math.max(hits.length - 1, 0)));
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      setCursor((c) => Math.max(c - 1, 0));
    } else if (e.key === "Enter") {
      e.preventDefault();
      if (hits[cursor]) openHit(hits[cursor]);
    } else if (e.key === "Escape") {
      e.preventDefault();
      setQuery("");
    }
  };

  useEffect(() => {
    if (listRef.current) {
      const el = listRef.current.querySelector<HTMLElement>(`[data-cursor="${cursor}"]`);
      el?.scrollIntoView({ block: "nearest" });
    }
  }, [cursor]);

  return (
    <div className="h-screen flex flex-col bg-white/95 dark:bg-zinc-900/95 text-sm" style={{ color: "#1d1d1f" }}>
      <div className="px-4 py-3 border-b border-black/5 flex items-center gap-2.5 bg-black/[0.01]">
        <Search size={14} className="opacity-40" />
        <input
          ref={inputRef}
          type="text"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          onKeyDown={onKeyDown}
          placeholder="搜索便签内容..."
          className="flex-1 outline-none bg-transparent text-sm placeholder-black/35 font-medium"
        />
        {loading && (
          <div className="flex items-center gap-1.5 flex-shrink-0">
            <span className="w-1.5 h-1.5 rounded-full bg-black/40 animate-pulse" />
            <span className="text-[10px] opacity-40 font-medium">搜索中...</span>
          </div>
        )}
      </div>

      <div className="flex-1 overflow-auto" ref={listRef}>
        {error && (
          <div className="p-4 text-xs text-red-600 font-medium">搜索失败：{error}</div>
        )}
        {!error && hits.length === 0 && query.trim() && !loading && (
          <div className="p-8 text-center text-xs opacity-40 select-none">没有找到匹配的内容</div>
        )}
        {!error && hits.length === 0 && !query.trim() && (
          <div className="p-8 text-center text-xs opacity-50 select-none">
            <div className="font-semibold text-black/70 mb-2">键盘快捷键</div>
            <div className="text-[10.5px] text-black/55 flex justify-center items-center gap-3">
              <span className="flex items-center gap-1">
                <kbd className="font-mono px-1.5 py-0.5 rounded-md border border-black/15 bg-black/[0.04] shadow-sm">↑↓</kbd> 移动
              </span>
              <span className="opacity-30">·</span>
              <span className="flex items-center gap-1">
                <kbd className="font-mono px-1.5 py-0.5 rounded-md border border-black/15 bg-black/[0.04] shadow-sm">Enter</kbd> 打开
              </span>
              <span className="opacity-30">·</span>
              <span className="flex items-center gap-1">
                <kbd className="font-mono px-1.5 py-0.5 rounded-md border border-black/15 bg-black/[0.04] shadow-sm">Esc</kbd> 清除
              </span>
            </div>
          </div>
        )}
        {hits.map((h, i) => {
          const selected = i === cursor;
          return (
            <button
              key={h.item_id}
              data-cursor={i}
              onClick={() => openHit(h)}
              onMouseEnter={() => setCursor(i)}
              className={`w-full text-left px-4 py-3 border-b border-black/[0.03] transition-all duration-150 flex items-center justify-between group ${
                selected ? "bg-black/[0.04]" : "hover:bg-black/[0.01]"
              }`}
            >
              <div className="flex-1 min-w-0 pr-4">
                <div className="flex items-center gap-2 mb-1.5">
                  <span
                    className="inline-block w-2.5 h-2.5 rounded-full border border-black/10 shadow-sm"
                    style={{ backgroundColor: h.bg_color }}
                  />
                  <span className="text-[9px] font-semibold opacity-40 font-mono tracking-wider">
                    NOTE-{h.sticky_id.slice(-6).toUpperCase()}
                  </span>
                </div>
                <div className="text-xs leading-relaxed text-black/80 font-medium">{renderSnippet(h.snippet)}</div>
              </div>
              {selected && (
                <span className="opacity-40 flex items-center gap-0.5 text-[9px] font-semibold tracking-wider uppercase px-1.5 py-0.5 rounded bg-black/5 flex-shrink-0 animate-fade-in">
                  打开 <CornerDownLeft size={8} strokeWidth={2.5} />
                </span>
              )}
            </button>
          );
        })}
      </div>
    </div>
  );
}

/// 把 FTS snippet 里的 【...】 标记解析成 <mark> React 节点。
/// 纯 JSX 构造，无 innerHTML，避免 XSS。
function renderSnippet(raw: string): React.ReactNode[] {
  const parts: React.ReactNode[] = [];
  const matches = [...raw.matchAll(/【(.+?)】/g)];
  let cursor = 0;
  let key = 0;
  for (const m of matches) {
    const start = m.index ?? 0;
    if (start > cursor) {
      parts.push(<span key={key++}>{raw.slice(cursor, start)}</span>);
    }
    parts.push(
      <mark key={key++} className="bg-yellow-400/30 text-yellow-900 font-semibold px-1 rounded-md">
        {m[1]}
      </mark>,
    );
    cursor = start + m[0].length;
  }
  if (cursor < raw.length) {
    parts.push(<span key={key++}>{raw.slice(cursor)}</span>);
  }
  return parts;
}
