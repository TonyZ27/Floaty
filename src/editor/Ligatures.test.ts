import { describe, it, expect } from "vitest";
import { Editor } from "@tiptap/core";
import StarterKit from "@tiptap/starter-kit";
import { Ligatures } from "./Ligatures";

describe("Ligatures extension", () => {
  const createTestEditor = (content = "") => {
    return new Editor({
      extensions: [StarterKit, Ligatures],
      content,
    });
  };

  function typeText(editor: Editor, text: string) {
    for (const char of text) {
      const { from, to } = editor.state.selection;
      let handled = false;
      editor.view.someProp("handleTextInput", (f: any) => {
        if (f(editor.view, from, to, char)) {
          handled = true;
          return true;
        }
        return false;
      });
      if (!handled) {
        editor.commands.insertContent(char);
      }
    }
  }

  it("converts -> to →", () => {
    const editor = createTestEditor("<p>test</p>");
    editor.commands.focus("end");
    typeText(editor, " ->");
    expect(editor.getText()).toBe("test →");
  });

  it("converts <- to ←", () => {
    const editor = createTestEditor("<p>test</p>");
    editor.commands.focus("end");
    typeText(editor, " <-");
    expect(editor.getText()).toBe("test ←");
  });

  it("converts => to ⇒", () => {
    const editor = createTestEditor("<p>test</p>");
    editor.commands.focus("end");
    typeText(editor, " =>");
    expect(editor.getText()).toBe("test ⇒");
  });

  it("converts <= to ⇐", () => {
    const editor = createTestEditor("<p>test</p>");
    editor.commands.focus("end");
    typeText(editor, " <=");
    expect(editor.getText()).toBe("test ⇐");
  });

  it("converts <-> to ↔", () => {
    const editor = createTestEditor("<p>test</p>");
    editor.commands.focus("end");
    typeText(editor, " <->");
    expect(editor.getText()).toBe("test ↔");
  });

  it("converts <=> to ⇔", () => {
    const editor = createTestEditor("<p>test</p>");
    editor.commands.focus("end");
    typeText(editor, " <=>");
    expect(editor.getText()).toBe("test ⇔");
  });

  it("converts -- to —", () => {
    const editor = createTestEditor("<p>test</p>");
    editor.commands.focus("end");
    typeText(editor, " --");
    expect(editor.getText()).toBe("test —");
  });
});
