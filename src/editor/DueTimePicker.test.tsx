import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { DueTimePicker, computePickerPosition } from "./DueTimePicker";

describe("computePickerPosition", () => {
  it("positions below anchor when enough space is available", () => {
    // 800x600 viewport, anchor at top-left
    const pos = computePickerPosition({ left: 20, top: 40, bottom: 60 }, 800, 600);
    expect(pos.x).toBe(20);
    expect(pos.top).toBe(64); // bottom + 4
    expect(pos.bottom).toBeUndefined();
    expect(pos.maxHeight).toBe(600 - 64 - 8);
  });

  it("positions above anchor when space below is tight but space above is large", () => {
    // 320x420 viewport, anchor near the bottom (top: 360, bottom: 380)
    const pos = computePickerPosition({ left: 20, top: 360, bottom: 380 }, 320, 420);
    expect(pos.x).toBe(20);
    expect(pos.top).toBeUndefined();
    expect(pos.bottom).toBe(420 - 360 + 4); // 64
    expect(pos.maxHeight).toBeLessThanOrEqual(420 - pos.bottom! - 8);
  });

  it("clamps horizontal coordinate when anchor is near right window border", () => {
    // 300px width window, anchor at left: 250 (picker width is 220)
    const pos = computePickerPosition({ left: 250, top: 40, bottom: 60 }, 300, 400);
    // max X should be 300 - 220 - 8 = 72
    expect(pos.x).toBe(72);
  });

  it("clamps to margins for very small viewports", () => {
    const pos = computePickerPosition({ left: 10, top: 50, bottom: 70 }, 200, 200);
    expect(pos.x).toBe(8);
    expect(pos.top).toBe(8);
    expect(pos.maxHeight).toBe(200 - 16);
  });
});

describe("DueTimePicker component", () => {
  it("renders quick options, countdown, and custom picker", () => {
    const onPick = vi.fn();
    const onCancel = vi.fn();
    render(
      <DueTimePicker
        anchor={{ left: 20, top: 40, bottom: 60 }}
        onPick={onPick}
        onCancel={onCancel}
      />,
    );

    expect(screen.getByText("选择截止时间")).toBeInTheDocument();
    expect(screen.getByText("今天 18:00")).toBeInTheDocument();
    expect(screen.getByText("倒计时")).toBeInTheDocument();
    expect(screen.getByText("指定日期时间")).toBeInTheDocument();
  });

  it("fires onPick when a quick option is clicked", () => {
    const onPick = vi.fn();
    const onCancel = vi.fn();
    render(
      <DueTimePicker
        anchor={{ left: 20, top: 40, bottom: 60 }}
        onPick={onPick}
        onCancel={onCancel}
      />,
    );

    fireEvent.click(screen.getByText("今天 18:00"));
    expect(onPick).toHaveBeenCalledTimes(1);
    expect(onPick).toHaveBeenCalledWith(expect.stringMatching(/T18:00:00/));
  });

  it("fires onCancel when cancel button is clicked", () => {
    const onPick = vi.fn();
    const onCancel = vi.fn();
    render(
      <DueTimePicker
        anchor={{ left: 20, top: 40, bottom: 60 }}
        onPick={onPick}
        onCancel={onCancel}
      />,
    );

    fireEvent.click(screen.getByTitle("取消 (Esc)"));
    expect(onCancel).toHaveBeenCalledTimes(1);
  });
});
