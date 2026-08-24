import { describe, it, expect } from "vitest";
import {
  formatKeyCombo,
  isValidCombo,
  serializeKeyCombo,
  parseKeyCombo,
  matchesShortcut,
} from "./utils";

describe("formatKeyCombo", () => {
  it("formats key combos with '+' delimiter between keys", () => {
    expect(formatKeyCombo({ meta: true, shift: true, key: "N" })).toBe("⇧ + ⌘ + N");
    expect(formatKeyCombo({ alt: true, shift: true, key: "T" })).toBe("⌥ + ⇧ + T");
    expect(formatKeyCombo({ meta: true, key: "W" })).toBe("⌘ + W");
    expect(formatKeyCombo({ meta: true, key: "Backspace" })).toBe("⌘ + ⌫");
    expect(formatKeyCombo({ alt: true, key: "Space" })).toBe("⌥ + Space");
  });

  it("returns '未设置' when combo is empty or null", () => {
    expect(formatKeyCombo(null)).toBe("未设置");
    expect(formatKeyCombo({ meta: true })).toBe("未设置");
  });
});

describe("isValidCombo", () => {
  it("requires at least one modifier (Control, Option, or Command)", () => {
    expect(isValidCombo({ meta: true, key: "N" })).toBe(true);
    expect(isValidCombo({ alt: true, key: "T" })).toBe(true);
    expect(isValidCombo({ ctrl: true, key: "A" })).toBe(true);

    // No modifiers or only Shift without Ctrl/Alt/Cmd is invalid
    expect(isValidCombo({ shift: true, key: "N" })).toBe(false);
    expect(isValidCombo({ key: "N" })).toBe(false);
    expect(isValidCombo({ meta: true })).toBe(false);
  });
});

describe("serializeKeyCombo and parseKeyCombo", () => {
  it("round-trips KeyCombo to string and back", () => {
    const combo = { meta: true, shift: true, key: "N" };
    const serialized = serializeKeyCombo(combo);
    expect(serialized).toBe("Shift+Meta+N");
    const parsed = parseKeyCombo(serialized);
    expect(parsed?.meta).toBe(true);
    expect(parsed?.shift).toBe(true);
    expect(parsed?.key).toBe("N");
  });

  it("handles null/none gracefully", () => {
    expect(serializeKeyCombo(null)).toBe("none");
    expect(parseKeyCombo("none")).toBeNull();
    expect(parseKeyCombo("")).toBeNull();
  });
});

describe("matchesShortcut", () => {
  it("matches KeyboardEvent with combo", () => {
    const combo = { meta: true, shift: true, key: "N" };
    const matchingEvent = new KeyboardEvent("keydown", {
      key: "N",
      metaKey: true,
      shiftKey: true,
    });
    expect(matchesShortcut(matchingEvent, combo)).toBe(true);

    const nonMatchingEvent = new KeyboardEvent("keydown", {
      key: "N",
      metaKey: true,
      shiftKey: false,
    });
    expect(matchesShortcut(nonMatchingEvent, combo)).toBe(false);
  });
});
