import { describe, it, expect, vi } from "vitest";
import { render } from "@testing-library/react";
import { Editor } from "./Editor";

// Mock Tauri invoke & listen
vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn().mockResolvedValue(null),
}));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn().mockResolvedValue(() => {}),
}));

describe("Editor @ DueTimePicker integration", () => {
  it("renders editor properly", async () => {
    const onChange = vi.fn();
    const { container } = render(<Editor initialMarkdown="hello" onChange={onChange} />);
    const proseMirror = container.querySelector(".ProseMirror") as HTMLElement;
    expect(proseMirror).toBeInTheDocument();
  });
});
