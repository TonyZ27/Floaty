import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { InsertPicker, computePickerPosition } from "./InsertPicker";

describe("computePickerPosition", () => {
  it("places picker below anchor when there is ample vertical space", () => {
    const pos = computePickerPosition({ left: 100, top: 100, bottom: 120 }, 800, 600);
    expect(pos.top).toBe(124);
    expect(pos.bottom).toBeUndefined();
    expect(pos.x).toBe(100);
  });

  it("places picker above anchor when near bottom edge", () => {
    const pos = computePickerPosition({ left: 100, top: 520, bottom: 540 }, 800, 600);
    expect(pos.bottom).toBe(84); // 600 - 520 + 4
    expect(pos.top).toBeUndefined();
  });

  it("clamps horizontal position to stay inside viewport", () => {
    const posRight = computePickerPosition({ left: 750, top: 100, bottom: 120 }, 800, 600);
    expect(posRight.x).toBe(800 - 220 - 8);

    const posLeft = computePickerPosition({ left: -20, top: 100, bottom: 120 }, 800, 600);
    expect(posLeft.x).toBe(8);
  });
});

describe("InsertPicker component", () => {
  it("renders priority options and due time options without countdown", () => {
    const onPick = vi.fn();
    render(
      <InsertPicker
        anchor={{ left: 20, top: 40, bottom: 60 }}
        onPick={onPick}
      />,
    );

    expect(screen.getByText("优先级")).toBeInTheDocument();
    expect(screen.getByText("P0")).toBeInTheDocument();
    expect(screen.getByText("P1")).toBeInTheDocument();
    expect(screen.getByText("P2")).toBeInTheDocument();
    expect(screen.getByText("P3")).toBeInTheDocument();

    expect(screen.getByText("截止时间")).toBeInTheDocument();
    expect(screen.getByText("今天 18:00")).toBeInTheDocument();
    expect(screen.getByText("指定日期时间...")).toBeInTheDocument();

    // 验证倒计时已被移除
    expect(screen.queryByText("倒计时")).not.toBeInTheDocument();
  });

  it("fires onPick with priority when P0 is clicked", () => {
    const onPick = vi.fn();
    render(
      <InsertPicker
        anchor={{ left: 20, top: 40, bottom: 60 }}
        onPick={onPick}
      />,
    );

    fireEvent.click(screen.getByText("P0"));
    expect(onPick).toHaveBeenCalledTimes(1);
    expect(onPick).toHaveBeenCalledWith({ kind: "priority", level: "P0" });
  });

  it("fires onPick with dueTime when a quick option is clicked", () => {
    const onPick = vi.fn();
    render(
      <InsertPicker
        anchor={{ left: 20, top: 40, bottom: 60 }}
        onPick={onPick}
      />,
    );

    fireEvent.click(screen.getByText("今天 18:00"));
    expect(onPick).toHaveBeenCalledTimes(1);
    expect(onPick).toHaveBeenCalledWith({
      kind: "dueTime",
      iso: expect.stringMatching(/T18:00:00/),
    });
  });

  it("switches to custom datetime subview with back button and allows return", () => {
    const onPick = vi.fn();
    render(
      <InsertPicker
        anchor={{ left: 20, top: 40, bottom: 60 }}
        onPick={onPick}
      />,
    );

    // 点击进入指定日期时间子页面
    fireEvent.click(screen.getByText("指定日期时间..."));
    expect(screen.getByTitle("返回")).toBeInTheDocument();
    expect(screen.getByText("确定")).toBeInTheDocument();
    expect(screen.queryByText("优先级")).not.toBeInTheDocument();

    // 点击返回回到主页面
    fireEvent.click(screen.getByTitle("返回"));
    expect(screen.getByText("优先级")).toBeInTheDocument();
    expect(screen.getByText("截止时间")).toBeInTheDocument();
  });

  it("submits custom datetime from subview", () => {
    const onPick = vi.fn();
    render(
      <InsertPicker
        anchor={{ left: 20, top: 40, bottom: 60 }}
        onPick={onPick}
      />,
    );

    fireEvent.click(screen.getByText("指定日期时间..."));
    fireEvent.click(screen.getByText("确定"));
    expect(onPick).toHaveBeenCalledTimes(1);
    expect(onPick).toHaveBeenCalledWith({
      kind: "dueTime",
      iso: expect.stringMatching(/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}/),
    });
  });
});
