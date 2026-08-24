import { KeyCombo } from "./types";

/**
 * Format key combination into human-readable string with " + " delimiter between keys.
 * Example on macOS: "⌘ + ⇧ + N", "⌥ + Space", "⌘ + W", "⌘ + ⌫"
 */
export function formatKeyCombo(combo: KeyCombo | null | undefined, isMac = true): string {
  if (!combo || !combo.key) return "未设置";

  const parts: string[] = [];

  // Modifiers order: Control -> Option/Alt -> Shift -> Command/Meta
  if (combo.ctrl) {
    parts.push(isMac ? "⌃" : "Ctrl");
  }
  if (combo.alt) {
    parts.push(isMac ? "⌥" : "Alt");
  }
  if (combo.shift) {
    parts.push(isMac ? "⇧" : "Shift");
  }
  if (combo.meta) {
    parts.push(isMac ? "⌘" : "Win");
  }

  // Key representation
  const keyUpper = combo.key.toUpperCase();
  if (combo.key === "Backspace") {
    parts.push(isMac ? "⌫" : "Backspace");
  } else if (combo.key === "Enter") {
    parts.push(isMac ? "↵" : "Enter");
  } else if (combo.key === "Tab") {
    parts.push(isMac ? "⇥" : "Tab");
  } else if (combo.key === " " || keyUpper === "SPACE") {
    parts.push("Space");
  } else if (combo.key === "Escape") {
    parts.push("Esc");
  } else {
    parts.push(combo.key.length === 1 ? keyUpper : combo.key);
  }

  return parts.join(" + ");
}

/**
 * Validates that the combo contains at least one of Control, Option/Alt, or Command/Meta,
 * and has a non-modifier key.
 */
export function isValidCombo(combo: KeyCombo | null | undefined): boolean {
  if (!combo) return false;
  const hasModifier = Boolean(combo.ctrl || combo.alt || combo.meta);
  const hasKey = Boolean(combo.key && combo.key.trim().length > 0);
  return hasModifier && hasKey;
}

/**
 * Serialize KeyCombo to string for SQLite settings persistence.
 * E.g. "Meta+Shift+N", "Meta+W", "none"
 */
export function serializeKeyCombo(combo: KeyCombo | null | undefined): string {
  if (!combo || !combo.key) return "none";
  const parts: string[] = [];
  if (combo.ctrl) parts.push("Ctrl");
  if (combo.alt) parts.push("Alt");
  if (combo.shift) parts.push("Shift");
  if (combo.meta) parts.push("Meta");
  parts.push(combo.key);
  return parts.join("+");
}

/**
 * Parse string from SQLite settings back to KeyCombo.
 */
export function parseKeyCombo(str: string | null | undefined): KeyCombo | null {
  if (!str || str === "none" || str.trim() === "") return null;
  const tokens = str.split("+");
  const key = tokens.pop() || "";
  const combo: KeyCombo = {
    ctrl: tokens.includes("Ctrl"),
    alt: tokens.includes("Alt"),
    shift: tokens.includes("Shift"),
    meta: tokens.includes("Meta") || tokens.includes("Cmd") || tokens.includes("Super"),
    key: key === "Comma" ? "," : key,
  };
  return combo;
}

/**
 * Check whether a KeyboardEvent matches a KeyCombo.
 */
export function matchesShortcut(e: KeyboardEvent, combo: KeyCombo | null | undefined): boolean {
  if (!combo || !combo.key) return false;

  const ctrlMatches = Boolean(combo.ctrl) === Boolean(e.ctrlKey);
  const altMatches = Boolean(combo.alt) === Boolean(e.altKey);
  const shiftMatches = Boolean(combo.shift) === Boolean(e.shiftKey);
  const metaMatches = Boolean(combo.meta) === Boolean(e.metaKey);

  if (!ctrlMatches || !altMatches || !shiftMatches || !metaMatches) {
    return false;
  }

  // Key matching
  const targetKey = combo.key.toLowerCase();
  const eventKey = e.key.toLowerCase();

  if (targetKey === "space" && (eventKey === " " || eventKey === "space")) {
    return true;
  }
  if (targetKey === "," && (eventKey === "," || eventKey === "comma")) {
    return true;
  }
  if (targetKey === "backspace" && eventKey === "backspace") {
    return true;
  }

  return eventKey === targetKey;
}

/**
 * Normalize a KeyboardEvent to a KeyCombo candidate.
 */
export function eventToKeyCombo(e: KeyboardEvent): { combo: KeyCombo; isModifierOnly: boolean } | null {
  const isModifierKey = ["Control", "Alt", "Shift", "Meta", "Command", "Option", "Super"].includes(e.key);

  const combo: KeyCombo = {
    ctrl: e.ctrlKey,
    alt: e.altKey,
    shift: e.shiftKey,
    meta: e.metaKey,
  };

  if (isModifierKey) {
    return { combo, isModifierOnly: true };
  }

  let key = e.key;
  if (key === " ") key = "Space";
  else if (key.length === 1) key = key.toUpperCase();

  combo.key = key;
  return { combo, isModifierOnly: false };
}
