import { useEditor, EditorContent } from "@tiptap/react";
import { BubbleMenu, FloatingMenu } from "@tiptap/react/menus";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { Editor as TipTapEditorT } from "@tiptap/core";
import type { EditorState } from "@tiptap/pm/state";
import { Extension } from "@tiptap/core";
import { Plugin } from "@tiptap/pm/state";
import StarterKit from "@tiptap/starter-kit";
import TaskList from "@tiptap/extension-task-list";
import TaskItem from "@tiptap/extension-task-item";
import Link from "@tiptap/extension-link";
import { useEffect, useRef, useState } from "react";
import { docToMarkdown, markdownToDoc } from "./markdown";
import { DueTime, refreshAllPills } from "./DueTime";
import { Priority, refreshAllPriorities, PriorityLevel } from "./Priority";
import { InsertPicker, type AnchorRect, type InsertChoice } from "./InsertPicker";
import { parseTimeHint, type TimeHint } from "./timeHints";
import { tierLabel, tierOf } from "../theme/urgency";
import {
  Bold,
  Italic,
  Strikethrough,
  Code,
  Heading1,
  Heading2,
  Link as LinkIcon,
  Quote,
  ListTodo,
  List,
  ListOrdered,
} from "lucide-react";

/**
 * 键入 `)` 时，往前扫描看是否刚好构成 `[text](url)`；是则替换为 link mark。
 * 用 ProseMirror 的 handleTextInput 直接处理，不走 TipTap 的 InputRule 封装
 * （避免 capture-group 约定和 Link 扩展自身规则的冲突）。
 */
const MarkdownLinkShortcut = Extension.create({
  name: "markdownLinkShortcut",
  addProseMirrorPlugins() {
    return [
      new Plugin({
        props: {
          handleTextInput(view, from, to, text) {
            if (text !== ")") return false;
            const $from = view.state.doc.resolve(from);
            const blockFrom = $from.start();
            const before = view.state.doc.textBetween(blockFrom, from, "\n", "\n");
            const candidate = before + text;
            const m = candidate.match(/\[([^\]\n]+)\]\(([^)\s]+)\)$/);
            if (!m) return false;
            const matchStart = from - (m[0].length - 1);
            const linkMarkType = view.state.schema.marks.link;
            if (!linkMarkType) return false;
            const tr = view.state.tr;
            tr.replaceRangeWith(
              matchStart,
              to,
              view.state.schema.text(m[1], [linkMarkType.create({ href: m[2] })]),
            );
            tr.removeStoredMark(linkMarkType);
            view.dispatch(tr);
            console.log("[floaty] link shortcut fired:", { text: m[1], url: m[2] });
            return true;
          },
        },
      }),
    ];
  },
});

interface AtMentionOptions {
  onTrigger: (anchor: AnchorRect) => void;
  onDismiss: () => void;
}

const AtMentionTrigger = Extension.create<AtMentionOptions>({
  name: "atMentionTrigger",
  addOptions() {
    return {
      onTrigger: () => {},
      onDismiss: () => {},
    };
  },
  addProseMirrorPlugins() {
    return [
      new Plugin({
        view: () => {
          return {
            update: (view) => {
              const { state } = view;
              const { selection } = state;
              const { empty, from } = selection;

              if (!empty || from === 0) {
                this.options.onDismiss();
                return;
              }

              const $from = selection.$from;
              if (
                $from.parent.type.name === "codeBlock" ||
                (view.state.schema.marks.code &&
                  view.state.doc.rangeHasMark(from - 1, from, view.state.schema.marks.code))
              ) {
                this.options.onDismiss();
                return;
              }

              const charBefore = state.doc.textBetween(from - 1, from, "\n", " ");
              if (charBefore === "@") {
                try {
                  const coords = view.coordsAtPos(from);
                  this.options.onTrigger({
                    left: coords.left,
                    top: coords.top,
                    bottom: coords.bottom,
                  });
                } catch (e) {
                  console.warn("[floaty] failed to get coords at pos:", e);
                }
              } else {
                this.options.onDismiss();
              }
            },
          };
        },
      }),
    ];
  },
});

interface EditorProps {
  initialMarkdown: string;
  onChange: (markdown: string) => void;
}

type PickerState =
  | null
  | { anchor: AnchorRect; mode: "insert" }
  | { anchor: AnchorRect; mode: "edit-due"; pos: number }
  | { anchor: AnchorRect; mode: "edit-priority"; pos: number };

interface GhostState {
  from: number;
  to: number;
  hint: TimeHint;
  x: number;
  y: number;
}

export function Editor({ initialMarkdown, onChange }: EditorProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const [picker, setPicker] = useState<PickerState>(null);
  const pickerRef = useRef<PickerState>(null);
  pickerRef.current = picker;
  const [ghost, setGhost] = useState<GhostState | null>(null);
  const ghostRef = useRef<GhostState | null>(null);
  ghostRef.current = ghost;
  const [bubbleEnabled, setBubbleEnabled] = useState(true);
  const [floatingEnabled, setFloatingEnabled] = useState(true);

  useEffect(() => {
    const load = async () => {
      const [b, f] = await Promise.all([
        invoke<string | null>("get_setting", { key: "bubble_menu_enabled" }),
        invoke<string | null>("get_setting", { key: "floating_menu_enabled" }),
      ]);
      setBubbleEnabled(b !== "false");
      setFloatingEnabled(f !== "false");
    };
    load();
    const unlistenPromise = listen<string>("settings-changed", (e) => {
      if (e.payload === "bubble_menu_enabled" || e.payload === "floating_menu_enabled") load();
    });
    return () => {
      unlistenPromise.then((fn) => fn());
    };
  }, []);

  const editor = useEditor({
    extensions: [
      StarterKit.configure({ heading: { levels: [1, 2, 3] } }),
      TaskList,
      TaskItem.configure({ nested: true }),
      Link.configure({ openOnClick: false }),
      MarkdownLinkShortcut,
      AtMentionTrigger.configure({
        onTrigger: (anchor) => {
          setPicker({ anchor, mode: "insert" });
          setGhost(null);
        },
        onDismiss: () => {
          if (pickerRef.current?.mode === "insert") {
            setPicker(null);
          }
        },
      }),
      Priority.configure({
        onEdit: (pos: number, level: PriorityLevel) => {
          const el = containerRef.current?.querySelector<HTMLElement>(
            `span[data-priority="${level}"]`,
          );
          const rect = el?.getBoundingClientRect();
          const anchor: AnchorRect = rect
            ? { left: rect.left, top: rect.top, bottom: rect.bottom }
            : { left: 20, top: 20, bottom: 40 };
          setPicker({ anchor, mode: "edit-priority", pos });
        },
      }),
      DueTime.configure({
        onEdit: (pos: number, iso: string) => {
          const el = containerRef.current?.querySelector<HTMLElement>(
            `span[data-due-time][data-datetime="${iso}"]`,
          );
          const rect = el?.getBoundingClientRect();
          const anchor: AnchorRect = rect
            ? { left: rect.left, top: rect.top, bottom: rect.bottom }
            : { left: 20, top: 20, bottom: 40 };
          setPicker({ anchor, mode: "edit-due", pos });
        },
      }),
    ],
    content: markdownToDoc(initialMarkdown),
    onUpdate: ({ editor }) => {
      const md = docToMarkdown(editor.getJSON() as any);
      onChange(md);
    },
    editorProps: {
      handleKeyDown(view, event) {
        // Tab 接受 ghost 时间预览
        if (event.key === "Tab" && ghostRef.current && !pickerRef.current) {
          const g = ghostRef.current;
          acceptGhost(g);
          event.preventDefault();
          return true;
        }
        // Tab / Shift-Tab 列表与文本缩进
        if (event.key === "Tab" && !ghostRef.current && !pickerRef.current) {
          event.preventDefault();
          const { state } = view;
          if (editor.isActive("bulletList") || editor.isActive("orderedList")) {
            if (event.shiftKey) {
              editor.commands.liftListItem("listItem");
            } else {
              const success = editor.commands.sinkListItem("listItem");
              if (!success) {
                wrapCurrentList(editor, "listItem");
              }
            }
          } else if (editor.isActive("taskList")) {
            if (event.shiftKey) {
              editor.commands.liftListItem("taskItem");
            } else {
              const success = editor.commands.sinkListItem("taskItem");
              if (!success) {
                wrapCurrentList(editor, "taskItem");
              }
            }
          } else {
            if (event.shiftKey) {
              // 减少缩进: 从当前行首删除最多 2 个空格或 1 个 tab
              const { $from } = state.selection;
              const text = $from.parent.textContent;
              let indentLength = 0;
              if (text.startsWith("\t")) {
                indentLength = 1;
              } else if (text.startsWith("  ")) {
                indentLength = 2;
              } else if (text.startsWith(" ")) {
                indentLength = 1;
              }
              if (indentLength > 0) {
                const start = $from.start();
                editor.chain().focus().deleteRange({ from: start, to: start + indentLength }).run();
              }
            } else {
              // 增加缩进: 插入 2 个空格
              editor.chain().focus().insertContent("  ").run();
            }
          }
          return true;
        }
        // Enter 键空列表项提升与中断
        if (event.key === "Enter" && !event.shiftKey) {
          const { state } = view;
          const { selection } = state;
          const { $from, empty } = selection;
          if (empty) {
            const depth = $from.depth;
            if (depth >= 2) {
              const paragraphNode = $from.node(depth);
              const parentNode = $from.node(depth - 1);
              if (
                paragraphNode.type.name === "paragraph" &&
                (parentNode.type.name === "listItem" || parentNode.type.name === "taskItem")
              ) {
                if (paragraphNode.textContent.trim() === "") {
                  event.preventDefault();
                  if (parentNode.type.name === "listItem") {
                    editor.commands.liftListItem("listItem");
                  } else {
                    editor.commands.liftListItem("taskItem");
                  }
                  return true;
                }
              }
            }
          }
        }
        if (event.key === "Escape") {
          if (pickerRef.current) {
            setPicker(null);
            event.preventDefault();
            return true;
          }
          if (ghostRef.current) {
            setGhost(null);
            event.preventDefault();
            return true;
          }
        }
        return false;
      },
    },
  });

  // 每 60s 刷新所有 pill 与 priority
  useEffect(() => {
    const tick = () => {
      if (containerRef.current) {
        refreshAllPills(containerRef.current);
        refreshAllPriorities(containerRef.current);
      }
    };
    tick();
    const id = window.setInterval(tick, 60000);
    return () => window.clearInterval(id);
  }, [editor]);

  useEffect(() => {
    if (!editor) return;
    const handler = () => {
      if (containerRef.current) {
        refreshAllPills(containerRef.current);
        refreshAllPriorities(containerRef.current);
      }
    };
    editor.on("update", handler);
    return () => {
      editor.off("update", handler);
    };
  }, [editor]);

  useEffect(() => () => editor?.destroy(), [editor]);

  // 每次 selection / doc update 重算 ghost 预览
  useEffect(() => {
    if (!editor) return;
    const update = () => {
      if (pickerRef.current) {
        setGhost(null);
        return;
      }

      const { state } = editor;
      const sel = state.selection;
      const { $from, empty } = sel;

      if (!empty) {
        setGhost(null);
        return;
      }
      // 取当前 paragraph/taskItem 起点到光标的纯文本。
      // 用 $from.parent 拿到最内层块节点（paragraph），再 textBetween 0 → offset；
      // 比 $from.start() + doc.textBetween 更鲁棒（对 taskItem > paragraph 结构友好）。
      const text = $from.parent.textBetween(0, $from.parentOffset, "\n", " ");
      const hint = parseTimeHint(text);
      if (!hint) {
        setGhost(null);
        return;
      }
      // 匹配文本可以不在 cursor 尾部（"今天 24 点吃饭"）：from/to 由 hint.start/end 换算 doc 坐标
      const paraStart = $from.start();
      const fromPos = paraStart + hint.start;
      const toPos = paraStart + hint.end;
      try {
        const coords = editor.view.coordsAtPos(toPos);
        setGhost({ from: fromPos, to: toPos, hint, x: coords.left, y: coords.bottom });
      } catch {
        setGhost(null);
      }
    };
    editor.on("update", update);
    editor.on("selectionUpdate", update);
    update();
    return () => {
      editor.off("update", update);
      editor.off("selectionUpdate", update);
    };
  }, [editor]);

  const acceptGhost = (g: GhostState) => {
    if (!editor) return;
    const iso = g.hint.date.toISOString();
    editor
      .chain()
      .focus()
      .deleteRange({ from: g.from, to: g.to })
      .insertContent([
        { type: "dueTime", attrs: { datetime: iso } },
        { type: "text", text: " " },
      ])
      .run();
    setGhost(null);
  };

  const handlePick = (choice: InsertChoice) => {
    if (!editor) return;
    const current = pickerRef.current;
    if (!current) return;

    if (current.mode === "edit-due") {
      if (choice.kind === "dueTime") {
        editor
          .chain()
          .focus()
          .setNodeSelection(current.pos)
          .updateAttributes("dueTime", { datetime: choice.iso })
          .run();
      }
      setPicker(null);
      return;
    }

    if (current.mode === "edit-priority") {
      if (choice.kind === "priority") {
        editor
          .chain()
          .focus()
          .setNodeSelection(current.pos)
          .updateAttributes("priority", { level: choice.level })
          .run();
      }
      setPicker(null);
      return;
    }

    // insert 模式：删除光标前可能残留的 '@' 并插入对应的 Chip
    const { from } = editor.state.selection;
    if (from > 0) {
      const char = editor.state.doc.textBetween(from - 1, from);
      if (char === "@") {
        editor.chain().focus().deleteRange({ from: from - 1, to: from }).run();
      }
    }

    if (choice.kind === "priority") {
      editor
        .chain()
        .focus()
        .insertContent([
          { type: "priority", attrs: { level: choice.level } },
          { type: "text", text: " " },
        ])
        .run();
    } else {
      editor
        .chain()
        .focus()
        .insertContent([
          { type: "dueTime", attrs: { datetime: choice.iso } },
          { type: "text", text: " " },
        ])
        .run();
    }
    setPicker(null);
  };

  return (
    <div ref={containerRef} className="relative">
      <EditorContent editor={editor} className="prose prose-sm max-w-none focus:outline-none" />
      {editor && floatingEnabled && (
        <FloatingMenu
          editor={editor}
          shouldShow={({ editor, state }: { editor: TipTapEditorT; state: EditorState }) => {
            const { $from, empty } = state.selection;
            if (!empty) return false;
            // 光标所在块必须是空段落（不是 task/list/heading/etc）
            const parent = $from.parent;
            if (parent.type.name !== "paragraph") return false;
            if (parent.textContent.length > 0) return false;
            return editor.isEditable;
          }}
        >
          <div className="flex items-center gap-0.5 rounded-md shadow-lg border border-black/10 bg-white/95 backdrop-blur-sm px-1 py-0.5 text-[11px]">
            <FmtBtn
              onClick={() => editor.chain().focus().toggleTaskList().run()}
              title="待办清单"
            >
              <ListTodo size={14} />
            </FmtBtn>
            <FmtBtn
              onClick={() => editor.chain().focus().toggleBulletList().run()}
              title="无序列表"
            >
              <List size={14} />
            </FmtBtn>
            <FmtBtn
              onClick={() => editor.chain().focus().toggleOrderedList().run()}
              title="有序列表"
            >
              <ListOrdered size={14} />
            </FmtBtn>
            <FmtBtn
              onClick={() => editor.chain().focus().toggleHeading({ level: 1 }).run()}
              title="一级标题"
            >
              <Heading1 size={14} />
            </FmtBtn>
            <FmtBtn
              onClick={() => editor.chain().focus().toggleBlockquote().run()}
              title="引用"
            >
              <Quote size={14} />
            </FmtBtn>
          </div>
        </FloatingMenu>
      )}
      {editor && bubbleEnabled && (
        <BubbleMenu
          editor={editor}
          shouldShow={({ editor, state }: { editor: TipTapEditorT; state: EditorState }) => {
            const { from, to, empty } = state.selection;
            if (empty) return false;
            if (to - from < 1) return false;
            return editor.isEditable;
          }}
        >
          <div className="flex items-center gap-0.5 rounded-md shadow-lg border border-black/10 bg-white/95 backdrop-blur-sm px-1 py-0.5">
            <FmtBtn
              active={editor.isActive("bold")}
              onClick={() => editor.chain().focus().toggleBold().run()}
              title="粗体 ⌘B"
            >
              <Bold size={14} />
            </FmtBtn>
            <FmtBtn
              active={editor.isActive("italic")}
              onClick={() => editor.chain().focus().toggleItalic().run()}
              title="斜体 ⌘I"
            >
              <Italic size={14} />
            </FmtBtn>
            <FmtBtn
              active={editor.isActive("strike")}
              onClick={() => editor.chain().focus().toggleStrike().run()}
              title="删除线 ⌘⇧X"
            >
              <Strikethrough size={14} />
            </FmtBtn>
            <FmtBtn
              active={editor.isActive("code")}
              onClick={() => editor.chain().focus().toggleCode().run()}
              title="行内代码"
            >
              <Code size={14} />
            </FmtBtn>
            <div className="w-px h-4 bg-black/10 mx-0.5" />
            <FmtBtn
              active={editor.isActive("heading", { level: 1 })}
              onClick={() => editor.chain().focus().toggleHeading({ level: 1 }).run()}
              title="一级标题"
            >
              <Heading1 size={14} />
            </FmtBtn>
            <FmtBtn
              active={editor.isActive("heading", { level: 2 })}
              onClick={() => editor.chain().focus().toggleHeading({ level: 2 }).run()}
              title="二级标题"
            >
              <Heading2 size={14} />
            </FmtBtn>
            <FmtBtn
              onClick={() => {
                const url = window.prompt("链接地址：");
                if (!url) return;
                editor.chain().focus().setLink({ href: url }).run();
              }}
              title="添加链接"
            >
              <LinkIcon size={14} />
            </FmtBtn>
            <div className="w-px h-4 bg-black/10 mx-0.5" />
            <FmtBtn
              active={editor.isActive("taskList")}
              onClick={() => editor.chain().focus().toggleTaskList().run()}
              title="待办清单"
            >
              <ListTodo size={14} />
            </FmtBtn>
            <FmtBtn
              active={editor.isActive("bulletList")}
              onClick={() => editor.chain().focus().toggleBulletList().run()}
              title="无序列表"
            >
              <List size={14} />
            </FmtBtn>
            <FmtBtn
              active={editor.isActive("orderedList")}
              onClick={() => editor.chain().focus().toggleOrderedList().run()}
              title="有序列表"
            >
              <ListOrdered size={14} />
            </FmtBtn>
          </div>
        </BubbleMenu>
      )}
      {picker && (
        <InsertPicker
          anchor={picker.anchor}
          onPick={handlePick}
        />
      )}
      {ghost && !picker && <GhostPreview ghost={ghost} onAccept={() => acceptGhost(ghost)} />}
    </div>
  );
}

function FmtBtn({
  active,
  onClick,
  title,
  children,
}: {
  active?: boolean;
  onClick: () => void;
  title: string;
  children: React.ReactNode;
}) {
  return (
    <button
      type="button"
      title={title}
      className={`flex items-center justify-center min-w-[24px] h-6 px-1 rounded text-[11px] font-medium transition-colors ${
        active ? "bg-black text-white" : "text-black/70 hover:bg-black/5 active:bg-black/10"
      }`}
      // 只 preventDefault，防止编辑器失焦丢失选区
      onMouseDown={(e) => e.preventDefault()}
      onClick={(e) => {
        e.preventDefault();
        e.stopPropagation();
        onClick();
      }}
    >
      {children}
    </button>
  );
}

function formatDebugTime(d: Date): string {
  const pad = (n: number) => (n < 10 ? `0${n}` : `${n}`);
  return (
    `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} ` +
    `${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}`
  );
}

function GhostPreview({ ghost, onAccept }: { ghost: GhostState; onAccept: () => void }) {
  const iso = ghost.hint.date.toISOString();
  const tier = tierOf(iso);
  const label = tierLabel(tier, iso);
  const debugTime = formatDebugTime(ghost.hint.date);
  return (
    <div
      className="fixed z-40 pointer-events-auto select-none text-[10px]"
      style={{ left: ghost.x + 4, top: ghost.y - 2 }}
      onMouseDown={(e) => {
        e.preventDefault();
        onAccept();
      }}
      title="按 Tab 接受"
    >
      <span className="inline-flex items-center gap-1.5 px-1.5 py-[1px] rounded border border-dashed border-black/30 bg-white/80 text-black/60">
        <span>{label}</span>
        <span className="font-mono text-[9px] opacity-60">{debugTime}</span>
        <kbd className="px-1 rounded bg-black/10 text-[9px]">Tab</kbd>
      </span>
    </div>
  );
}

function wrapCurrentList(editor: any, itemTypeName: string) {
  const { state, view } = editor;
  const { tr, selection } = state;
  const { $from } = selection;

  let listDepth = -1;
  for (let d = $from.depth; d > 0; d--) {
    const node = $from.node(d);
    if (node.type.name === "bulletList" || node.type.name === "orderedList" || node.type.name === "taskList") {
      listDepth = d;
      break;
    }
  }

  if (listDepth > -1) {
    const listNode = $from.node(listDepth);
    const listPos = $from.before(listDepth);
    const listType = listNode.type;
    const itemType = state.schema.nodes[itemTypeName];

    if (itemType && listType) {
      const range = state.doc.resolve(listPos).blockRange(state.doc.resolve(listPos + listNode.nodeSize));
      if (range) {
        const wrapping = [
          { type: listType },
          { type: itemType }
        ];
        const wrapTr = tr.wrap(range, wrapping);
        if (wrapTr) {
          view.dispatch(wrapTr);
        }
      }
    }
  }
}
