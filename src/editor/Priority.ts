import { Node, mergeAttributes } from "@tiptap/core";

export type PriorityLevel = "P0" | "P1" | "P2" | "P3";

export interface PriorityMeta {
  level: PriorityLevel;
  label: string;
  badge: string;
  color: {
    bg: string;
    fg: string;
    border?: string;
  };
}

export const PRIORITY_CONFIG: Record<PriorityLevel, PriorityMeta> = {
  P0: {
    level: "P0",
    label: "紧急",
    badge: "P0",
    color: {
      bg: "rgba(239, 68, 68, 0.15)",
      fg: "#dc2626",
      border: "rgba(239, 68, 68, 0.3)",
    },
  },
  P1: {
    level: "P1",
    label: "高优",
    badge: "P1",
    color: {
      bg: "rgba(249, 115, 22, 0.15)",
      fg: "#ea580c",
      border: "rgba(249, 115, 22, 0.3)",
    },
  },
  P2: {
    level: "P2",
    label: "中优",
    badge: "P2",
    color: {
      bg: "rgba(234, 179, 8, 0.15)",
      fg: "#ca8a04",
      border: "rgba(234, 179, 8, 0.3)",
    },
  },
  P3: {
    level: "P3",
    label: "低优",
    badge: "P3",
    color: {
      bg: "rgba(100, 116, 139, 0.12)",
      fg: "#64748b",
      border: "rgba(100, 116, 139, 0.25)",
    },
  },
};

/**
 * TipTap inline atom node representing a priority chip.
 * Attr: level ("P0" | "P1" | "P2" | "P3")
 */
export const Priority = Node.create({
  name: "priority",
  group: "inline",
  inline: true,
  atom: true,
  selectable: true,

  addAttributes() {
    return {
      level: { default: "P1" as PriorityLevel },
    };
  },

  parseHTML() {
    return [{ tag: "span[data-priority]" }];
  },

  renderHTML({ HTMLAttributes, node }) {
    const level = (node.attrs.level as PriorityLevel) || "P1";
    return [
      "span",
      mergeAttributes(
        { "data-priority": level, contenteditable: "false" },
        HTMLAttributes,
      ),
      level,
    ];
  },

  addOptions() {
    return {
      onEdit: (_pos: number, _level: PriorityLevel) => {
        /* configured by Editor */
      },
    };
  },

  addNodeView() {
    return ({ node, getPos, editor }) => {
      const dom = document.createElement("span");
      const level = (node.attrs.level as PriorityLevel) || "P1";
      dom.setAttribute("data-priority", level);
      dom.contentEditable = "false";
      dom.title = "点击修改优先级";
      dom.addEventListener("click", (e) => {
        e.preventDefault();
        e.stopPropagation();
        const pos = typeof getPos === "function" ? getPos() : null;
        if (pos != null && level) {
          const opts = (editor.extensionManager.extensions.find((x) => x.name === "priority")
            ?.options ?? {}) as { onEdit?: (pos: number, level: PriorityLevel) => void };
          opts.onEdit?.(pos, level);
        }
      });
      refreshPriorityDom(dom);
      return { dom };
    };
  },
});

/**
 * Apply styling and label to priority DOM element.
 */
export function refreshPriorityDom(el: HTMLElement): void {
  const level = (el.getAttribute("data-priority") as PriorityLevel) || "P1";
  const config = PRIORITY_CONFIG[level] || PRIORITY_CONFIG.P1;

  el.textContent = config.badge;
  el.style.padding = "1px 6px";
  el.style.margin = "0 2px";
  el.style.borderRadius = "6px";
  el.style.fontSize = "0.78em";
  el.style.fontWeight = "600";
  el.style.fontFamily = "ui-monospace, monospace";
  el.style.display = "inline-block";
  el.style.verticalAlign = "middle";
  el.style.cursor = "default";
  el.style.userSelect = "none";
  el.style.border = `1px solid ${config.color.border || "transparent"}`;

  const completedLi = el.closest<HTMLElement>('li[data-checked="true"]');
  if (completedLi) {
    el.style.backgroundColor = "rgba(0,0,0,0.08)";
    el.style.color = "inherit";
    el.style.opacity = "0.55";
    el.style.textDecoration = "line-through";
    el.style.borderColor = "transparent";
    return;
  }

  el.style.backgroundColor = config.color.bg;
  el.style.color = config.color.fg;
  el.style.opacity = "1";
  el.style.textDecoration = "none";
}

/** Refresh all priority pills in a root element. */
export function refreshAllPriorities(root: ParentNode): void {
  root.querySelectorAll<HTMLElement>("span[data-priority]").forEach((el) => {
    refreshPriorityDom(el);
  });
}
