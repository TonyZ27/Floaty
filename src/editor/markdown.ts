type ProseNode = {
  type: string;
  attrs?: Record<string, any>;
  content?: ProseNode[];
  text?: string;
  marks?: { type: string; attrs?: Record<string, any> }[];
};

type ProseDoc = { type: "doc"; content?: ProseNode[] };

// ============================================================
// Serialize: ProseMirror doc → markdown string
// ============================================================

export function docToMarkdown(doc: ProseNode | ProseDoc): string {
  if (!("content" in doc) || !doc.content) return "";
  return doc.content.map(serializeBlock).join("\n");
}

function serializeBlock(node: ProseNode): string {
  switch (node.type) {
    case "heading": {
      const level = node.attrs?.level ?? 1;
      return "#".repeat(level) + " " + serializeInline(node.content ?? []);
    }
    case "paragraph":
      return serializeInline(node.content ?? []);
    case "taskList":
      return (node.content ?? []).map(serializeTaskItem).join("\n");
    case "bulletList":
      return (node.content ?? [])
        .map((li) => "- " + serializeListItemInner(li))
        .join("\n");
    case "orderedList":
      return (node.content ?? [])
        .map((li, i) => `${i + 1}. ` + serializeListItemInner(li))
        .join("\n");
    case "blockquote":
      return (node.content ?? [])
        .map(serializeBlock)
        .join("\n")
        .split("\n")
        .map((l) => "> " + l)
        .join("\n");
    case "codeBlock": {
      const lang = node.attrs?.language ?? "";
      const body = (node.content ?? []).map((c) => c.text ?? "").join("");
      return "```" + lang + "\n" + body + "\n```";
    }
    case "horizontalRule":
      return "---";
    default:
      return serializeInline(node.content ?? []);
  }
}

function serializeTaskItem(node: ProseNode): string {
  const checked = node.attrs?.checked ? "x" : " ";
  const inner = (node.content ?? [])
    .map(child => {
      const blockStr = serializeBlock(child);
      if (child.type === "bulletList" || child.type === "orderedList" || child.type === "taskList" || child.type === "blockquote") {
        return blockStr.split("\n").map(line => "  " + line).join("\n");
      }
      return blockStr;
    })
    .join("\n");
  return `- [${checked}] ${inner}`;
}

function serializeListItemInner(li: ProseNode): string {
  // A listItem wraps one or more paragraph/block nodes; join their inline text.
  return (li.content ?? [])
    .map(child => {
      const blockStr = serializeBlock(child);
      if (child.type === "bulletList" || child.type === "orderedList" || child.type === "taskList" || child.type === "blockquote") {
        return blockStr.split("\n").map(line => "  " + line).join("\n");
      }
      return blockStr;
    })
    .join("\n");
}

function serializeInline(nodes: ProseNode[]): string {
  return nodes
    .map((n) => {
      if (n.type === "priority") {
        const level = n.attrs?.level ?? "P1";
        return `@priority:${level}`;
      }
      if (n.type === "dueTime") {
        const iso = n.attrs?.datetime ?? "";
        return iso ? `@due:${iso}` : "";
      }
      if (n.type === "hardBreak") return "\n";
      if (n.type !== "text") return "";
      let text = n.text ?? "";
      const marks = n.marks ?? [];
      // 顺序：先 code（锁定内容），再 strike，bold，italic，最后 link 包外
      for (const m of marks) {
        if (m.type === "code") text = "`" + text + "`";
      }
      for (const m of marks) {
        if (m.type === "strike") text = `~~${text}~~`;
      }
      for (const m of marks) {
        if (m.type === "bold") text = `**${text}**`;
      }
      for (const m of marks) {
        if (m.type === "italic") text = `*${text}*`;
      }
      for (const m of marks) {
        if (m.type === "link") text = `[${text}](${m.attrs?.href ?? ""})`;
      }
      return text;
    })
    .join("");
}

// ============================================================
// Parse: markdown string → ProseMirror doc
// ============================================================

const DUE_RE = /@due:(\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:Z|[+-]\d{2}:\d{2}))/;
const DUE_RE_GLOBAL = /@due:(\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:Z|[+-]\d{2}:\d{2}))/g;
const PRIORITY_RE = /@priority:(P[0-3])/;
const PRIORITY_RE_GLOBAL = /@priority:(P[0-3])/g;

export interface DueEntry {
  itemIndex: number;
  iso: string;
  preview: string;
}

export function extractDues(md: string): DueEntry[] {
  const entries: DueEntry[] = [];
  const lines = md.split("\n");
  let nonEmptyIndex = 0;
  for (const line of lines) {
    if (line.trim() === "") continue;
    // 已完成的 todo 不再产生提醒
    const isCompletedTodo = /^- \[x\] /.test(line);
    if (!isCompletedTodo) {
      const matches = [...line.matchAll(DUE_RE_GLOBAL)];
      if (matches.length > 0) {
        const preview = line
          .replace(DUE_RE_GLOBAL, "")
          .replace(PRIORITY_RE_GLOBAL, "")
          .replace(/^- \[[ x]\] /, "")
          .replace(/^#+\s+/, "")
          .replace(/\*\*(.+?)\*\*/g, "$1")
          .replace(/\*(.+?)\*/g, "$1")
          .replace(/\[([^\]]+)\]\([^)]+\)/g, "$1")
          .trim()
          .slice(0, 50);
        for (const m of matches) {
          entries.push({ itemIndex: nonEmptyIndex, iso: m[1], preview });
        }
      }
    }
    nonEmptyIndex++;
  }
  return entries;
}

function getListLineInfo(line: string) {
  const taskM = line.match(/^(\s*)- \[( |x)\] (.*)$/);
  if (taskM) {
    return {
      indent: taskM[1].length,
      listType: "taskList" as const,
      itemType: "taskItem",
      itemAttrs: { checked: taskM[2] === "x" },
      text: taskM[3],
    };
  }
  const bulletM = line.match(/^(\s*)[-*]\s(?!\[[ x]\]\s)(.*)$/);
  if (bulletM) {
    return {
      indent: bulletM[1].length,
      listType: "bulletList" as const,
      itemType: "listItem",
      text: bulletM[2],
    };
  }
  const orderedM = line.match(/^(\s*)\d+\.\s(.*)$/);
  if (orderedM) {
    return {
      indent: orderedM[1].length,
      listType: "orderedList" as const,
      itemType: "listItem",
      text: orderedM[2],
    };
  }
  return null;
}

interface ListLine {
  indent: number;
  listType: "taskList" | "bulletList" | "orderedList";
  itemType: string;
  itemAttrs?: any;
  text: string;
}

function buildListTree(items: ListLine[], startIdx: number, baseIndent: number): { nodes: ProseNode[]; nextIdx: number } {
  const nodes: ProseNode[] = [];
  let idx = startIdx;

  while (idx < items.length) {
    const item = items[idx];
    if (item.indent < baseIndent) {
      break;
    }

    if (item.indent > baseIndent) {
      if (nodes.length > 0) {
        const lastListNode = nodes[nodes.length - 1];
        const lastItemNode = lastListNode.content ? lastListNode.content[lastListNode.content.length - 1] : null;
        if (lastItemNode) {
          const { nodes: nestedLists, nextIdx } = buildListTree(items, idx, item.indent);
          lastItemNode.content = [...(lastItemNode.content ?? []), ...nestedLists];
          idx = nextIdx;
          continue;
        }
      }
      baseIndent = item.indent;
    }

    const currentListType = item.listType;
    let listNode = nodes.find(n => n.type === currentListType);
    if (!listNode) {
      listNode = { type: currentListType, content: [] };
      nodes.push(listNode);
    }

    const itemNode: ProseNode = {
      type: item.itemType,
      content: [{ type: "paragraph", content: parseInline(item.text) }],
    };
    if (item.itemAttrs) {
      itemNode.attrs = item.itemAttrs;
    }
    listNode.content!.push(itemNode);

    idx++;
  }

  return { nodes, nextIdx: idx };
}

export function markdownToDoc(md: string): ProseDoc {
  const lines = md.split("\n");
  const blocks: ProseNode[] = [];

  let i = 0;
  while (i < lines.length) {
    const line = lines[i];

    // --- Code block (```lang\n...\n```) ---
    const fenceMatch = line.match(/^```(\S*)\s*$/);
    if (fenceMatch) {
      const lang = fenceMatch[1] || null;
      const body: string[] = [];
      i++;
      while (i < lines.length && !/^```\s*$/.test(lines[i])) {
        body.push(lines[i]);
        i++;
      }
      if (i < lines.length) i++; // 吃掉闭合 ```
      blocks.push({
        type: "codeBlock",
        attrs: lang ? { language: lang } : {},
        content: body.length ? [{ type: "text", text: body.join("\n") }] : [],
      });
      continue;
    }

    // --- Horizontal rule ---
    if (/^-{3,}\s*$/.test(line) || /^\*{3,}\s*$/.test(line)) {
      blocks.push({ type: "horizontalRule" });
      i++;
      continue;
    }

    // --- Heading ---
    const headingMatch = line.match(/^(#{1,6}) (.*)$/);
    if (headingMatch) {
      blocks.push({
        type: "heading",
        attrs: { level: headingMatch[1].length },
        content: parseInline(headingMatch[2]),
      });
      i++;
      continue;
    }

    // --- Blockquote (consecutive > lines) ---
    if (/^>\s/.test(line)) {
      const quotedLines: string[] = [];
      while (i < lines.length && /^>\s?/.test(lines[i])) {
        quotedLines.push(lines[i].replace(/^>\s?/, ""));
        i++;
      }
      const innerDoc = markdownToDoc(quotedLines.join("\n"));
      blocks.push({ type: "blockquote", content: innerDoc.content ?? [] });
      continue;
    }

    // --- Indent-aware list parsing ---
    const listInfo = getListLineInfo(line);
    if (listInfo) {
      const listLines: ListLine[] = [];
      while (i < lines.length) {
        const info = getListLineInfo(lines[i]);
        if (!info) break;
        listLines.push(info);
        i++;
      }
      const { nodes } = buildListTree(listLines, 0, listLines[0].indent);
      blocks.push(...nodes);
      continue;
    }

    // --- Empty line ---
    if (line.trim() === "") {
      i++;
      continue;
    }

    // --- Paragraph (default) ---
    blocks.push({ type: "paragraph", content: parseInline(line) });
    i++;
  }

  if (blocks.length === 0) {
    blocks.push({ type: "paragraph", content: [] });
  }

  return { type: "doc", content: blocks };
}

function parseInline(text: string): ProseNode[] {
  const tokens: ProseNode[] = [];
  let i = 0;

  const pushText = (s: string, marks?: ProseNode["marks"]) => {
    if (!s) return;
    if (marks && marks.length) tokens.push({ type: "text", text: s, marks });
    else tokens.push({ type: "text", text: s });
  };

  while (i < text.length) {
    // `code`
    if (text[i] === "`") {
      const end = text.indexOf("`", i + 1);
      if (end > i + 1) {
        pushText(text.slice(i + 1, end), [{ type: "code" }]);
        i = end + 1;
        continue;
      }
    }
    // ~~strike~~
    if (text.startsWith("~~", i)) {
      const end = text.indexOf("~~", i + 2);
      if (end > i + 2) {
        pushText(text.slice(i + 2, end), [{ type: "strike" }]);
        i = end + 2;
        continue;
      }
    }
    // **bold**
    if (text.startsWith("**", i)) {
      const end = text.indexOf("**", i + 2);
      if (end > i + 2) {
        pushText(text.slice(i + 2, end), [{ type: "bold" }]);
        i = end + 2;
        continue;
      }
    }
    // *italic*
    if (text[i] === "*") {
      const end = text.indexOf("*", i + 1);
      if (end > i + 1) {
        pushText(text.slice(i + 1, end), [{ type: "italic" }]);
        i = end + 1;
        continue;
      }
    }
    // [text](url)
    if (text[i] === "[") {
      const close = text.indexOf("](", i);
      const end = close > 0 ? text.indexOf(")", close) : -1;
      if (close > 0 && end > 0) {
        const linkText = text.slice(i + 1, close);
        const href = text.slice(close + 2, end);
        pushText(linkText, [{ type: "link", attrs: { href } }]);
        i = end + 1;
        continue;
      }
    }
    // @due:ISO at cursor position
    if (text.startsWith("@due:", i)) {
      const rest = text.slice(i);
      const dueM = rest.match(DUE_RE);
      if (dueM && dueM.index === 0) {
        tokens.push({ type: "dueTime", attrs: { datetime: dueM[1] } });
        i += dueM[0].length;
        continue;
      }
    }
    // @priority:P0..P3 at cursor position
    if (text.startsWith("@priority:", i)) {
      const rest = text.slice(i);
      const priM = rest.match(PRIORITY_RE);
      if (priM && priM.index === 0) {
        tokens.push({ type: "priority", attrs: { level: priM[1] } });
        i += priM[0].length;
        continue;
      }
    }
    // Accumulate normal chars until next special
    let j = i;
    while (
      j < text.length &&
      !"*[`~".includes(text[j]) &&
      !text.startsWith("@due:", j) &&
      !text.startsWith("@priority:", j)
    )
      j++;
    if (j === i) j = i + 1;
    pushText(text.slice(i, j));
    i = j;
  }
  return tokens;
}
