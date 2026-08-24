import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { ShortcutRecorder } from "./ShortcutRecorder";

describe("ShortcutRecorder", () => {
  it("renders formatted key combo with '+' delimiter", () => {
    render(
      <ShortcutRecorder
        value={{ meta: true, shift: true, key: "N" }}
        onChange={vi.fn()}
      />,
    );
    expect(screen.getByText("⇧ + ⌘ + N")).toBeInTheDocument();
  });

  it("renders '未设置' when value is null", () => {
    render(<ShortcutRecorder value={null} onChange={vi.fn()} />);
    expect(screen.getByText("未设置")).toBeInTheDocument();
  });

  it("triggers onChange(null) when clear button is clicked", () => {
    const onChange = vi.fn();
    render(
      <ShortcutRecorder
        value={{ meta: true, key: "W" }}
        onChange={onChange}
      />,
    );

    const clearBtn = screen.getByTitle("清除快捷键");
    fireEvent.click(clearBtn);
    expect(onChange).toHaveBeenCalledWith(null);
  });

  it("records valid key combo and calls onChange", () => {
    const onChange = vi.fn();
    render(
      <ShortcutRecorder
        value={{ meta: true, key: "W" }}
        onChange={onChange}
      />,
    );

    // 点击进入录入模式
    const recorder = screen.getByText("⌘ + W");
    fireEvent.click(recorder);

    // 录入 ⌥ + ⇧ + T
    fireEvent.keyDown(window, {
      key: "T",
      altKey: true,
      shiftKey: true,
    });

    expect(onChange).toHaveBeenCalledWith({
      ctrl: false,
      alt: true,
      shift: true,
      meta: false,
      key: "T",
    });
  });

  it("calls onInvalidCombo when no Control/Option/Command modifier is pressed", () => {
    const onChange = vi.fn();
    const onInvalidCombo = vi.fn();
    render(
      <ShortcutRecorder
        value={null}
        onChange={onChange}
        onInvalidCombo={onInvalidCombo}
      />,
    );

    fireEvent.click(screen.getByText("未设置"));

    // 按下单个字母 T（无修饰键）
    fireEvent.keyDown(window, { key: "T" });

    expect(onInvalidCombo).toHaveBeenCalledWith(
      "快捷键必须包含 Control、Option 或 Command",
    );
    expect(onChange).not.toHaveBeenCalled();
  });

  it("exits recording mode on Escape without triggering onChange", () => {
    const onChange = vi.fn();
    render(
      <ShortcutRecorder
        value={{ meta: true, key: "W" }}
        onChange={onChange}
      />,
    );

    fireEvent.click(screen.getByText("⌘ + W"));
    expect(screen.getByText("请按下快捷键...")).toBeInTheDocument();

    fireEvent.keyDown(window, { key: "Escape" });
    expect(onChange).not.toHaveBeenCalled();
    expect(screen.getByText("⌘ + W")).toBeInTheDocument();
  });
});
