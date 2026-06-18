import { describe, it, expect } from "vitest";
import { docToMarkdown, markdownToDoc, extractDues } from "./markdown";

describe("markdown round-trip", () => {
  it("serializes a single unchecked todo", () => {
    const doc = {
      type: "doc",
      content: [{
        type: "taskList",
        content: [{
          type: "taskItem",
          attrs: { checked: false },
          content: [{ type: "paragraph", content: [{ type: "text", text: "buy milk" }] }],
        }],
      }],
    };
    expect(docToMarkdown(doc)).toBe("- [ ] buy milk");
  });

  it("serializes a checked todo", () => {
    const doc = {
      type: "doc",
      content: [{
        type: "taskList",
        content: [{
          type: "taskItem",
          attrs: { checked: true },
          content: [{ type: "paragraph", content: [{ type: "text", text: "done" }] }],
        }],
      }],
    };
    expect(docToMarkdown(doc)).toBe("- [x] done");
  });

  it("parses markdown back into doc", () => {
    const md = "- [ ] buy milk\n- [x] done";
    const doc = markdownToDoc(md);
    expect(doc.content?.[0].type).toBe("taskList");
    expect(doc.content?.[0].content).toHaveLength(2);
    expect(doc.content?.[0].content?.[0].attrs?.checked).toBe(false);
    expect(doc.content?.[0].content?.[1].attrs?.checked).toBe(true);
  });

  it("round-trips a paragraph + bold + link", () => {
    const md = "Hello **world** and [tau](https://tauri.app)";
    const doc = markdownToDoc(md);
    expect(docToMarkdown(doc)).toBe(md);
  });

  it("round-trips a heading", () => {
    const md = "# Title";
    expect(docToMarkdown(markdownToDoc(md))).toBe(md);
  });

  it("round-trips a todo with @due token", () => {
    const md = "- [ ] 写周报 @due:2026-04-20T22:00:00Z";
    const doc = markdownToDoc(md);
    const para = doc.content?.[0].content?.[0].content?.[0];
    expect(para?.content).toHaveLength(2);
    expect(para?.content?.[0].type).toBe("text");
    expect(para?.content?.[1].type).toBe("dueTime");
    expect(para?.content?.[1].attrs?.datetime).toBe("2026-04-20T22:00:00Z");
    expect(docToMarkdown(doc)).toBe(md);
  });

  it("parses multiple @due tokens in one line", () => {
    const md = "mixed @due:2026-04-20T10:00:00Z and @due:2026-04-21T15:30:00Z";
    const doc = markdownToDoc(md);
    const para = doc.content?.[0];
    const dueNodes = para?.content?.filter((n) => n.type === "dueTime") ?? [];
    expect(dueNodes).toHaveLength(2);
  });
});

describe("markdown extended blocks", () => {
  it("round-trips bullet list", () => {
    const md = "- apple\n- banana";
    const doc = markdownToDoc(md);
    expect(doc.content?.[0].type).toBe("bulletList");
    expect(doc.content?.[0].content).toHaveLength(2);
    expect(docToMarkdown(doc)).toBe(md);
  });

  it("round-trips ordered list", () => {
    const md = "1. one\n2. two\n3. three";
    const doc = markdownToDoc(md);
    expect(doc.content?.[0].type).toBe("orderedList");
    expect(doc.content?.[0].content).toHaveLength(3);
    expect(docToMarkdown(doc)).toBe(md);
  });

  it("round-trips blockquote", () => {
    const md = "> wisdom";
    const doc = markdownToDoc(md);
    expect(doc.content?.[0].type).toBe("blockquote");
    expect(docToMarkdown(doc)).toBe(md);
  });

  it("round-trips code block", () => {
    const md = "```ts\nconst x = 1;\n```";
    const doc = markdownToDoc(md);
    expect(doc.content?.[0].type).toBe("codeBlock");
    expect(doc.content?.[0].attrs?.language).toBe("ts");
    expect(docToMarkdown(doc)).toBe(md);
  });

  it("round-trips horizontal rule", () => {
    expect(docToMarkdown(markdownToDoc("---"))).toBe("---");
  });

  it("round-trips strikethrough", () => {
    const md = "~~gone~~";
    expect(docToMarkdown(markdownToDoc(md))).toBe(md);
  });

  it("round-trips inline code", () => {
    const md = "use `foo` here";
    expect(docToMarkdown(markdownToDoc(md))).toBe(md);
  });

  it("task list still wins over bullet for '- [ ]' lines", () => {
    const doc = markdownToDoc("- [ ] task\n- item");
    expect(doc.content?.[0].type).toBe("taskList");
    expect(doc.content?.[1].type).toBe("bulletList");
  });

  it("round-trips nested bullet lists", () => {
    const md = "- parent\n  - child\n    - grandchild";
    const doc = markdownToDoc(md);
    expect(doc.content?.[0].type).toBe("bulletList");
    expect(docToMarkdown(doc)).toBe(md);
  });

  it("round-trips nested task lists", () => {
    const md = "- [ ] parent\n  - [ ] child\n    - [x] grandchild";
    const doc = markdownToDoc(md);
    expect(doc.content?.[0].type).toBe("taskList");
    expect(docToMarkdown(doc)).toBe(md);
  });
});

describe("extractDues", () => {
  it("returns empty when no @due", () => {
    expect(extractDues("- [ ] plain todo")).toEqual([]);
  });

  it("extracts single @due with preview", () => {
    const md = "- [ ] 写周报 @due:2026-04-20T22:00:00Z";
    expect(extractDues(md)).toEqual([
      { itemIndex: 0, iso: "2026-04-20T22:00:00Z", preview: "写周报" },
    ]);
  });

  it("assigns itemIndex per non-empty line", () => {
    const md = [
      "- [ ] a @due:2026-04-20T10:00:00Z",
      "",
      "- [ ] b @due:2026-04-21T10:00:00Z",
    ].join("\n");
    const dues = extractDues(md);
    expect(dues).toHaveLength(2);
    expect(dues[0].itemIndex).toBe(0);
    expect(dues[1].itemIndex).toBe(1);
  });

  it("strips markdown decoration from preview", () => {
    const md = "- [ ] **urgent** [link](http://x) @due:2026-04-20T10:00:00Z";
    expect(extractDues(md)[0].preview).toBe("urgent link");
  });

  it("skips completed todos (no reminder for '- [x]')", () => {
    const md = [
      "- [x] done @due:2026-04-20T10:00:00Z",
      "- [ ] pending @due:2026-04-21T10:00:00Z",
    ].join("\n");
    const dues = extractDues(md);
    expect(dues).toHaveLength(1);
    expect(dues[0].iso).toBe("2026-04-21T10:00:00Z");
  });
});
