import { expect, test } from "vitest";
import { actionFor, shortcutLabel } from "./keybindings";

const k = (
  key: string,
  mods: Partial<Record<"meta" | "ctrl" | "shift" | "alt", boolean>> = {},
) => ({
  key,
  metaKey: !!mods.meta,
  ctrlKey: !!mods.ctrl,
  shiftKey: !!mods.shift,
  altKey: !!mods.alt,
});

test("mac uses Cmd and leaves Ctrl to the shell", () => {
  expect(actionFor(k("k", { meta: true }), true)).toBe("palette");
  expect(actionFor(k("j", { meta: true }), true)).toBe("toggleTerminal");
  expect(actionFor(k("N", { meta: true, shift: true }), true)).toBe("newChat");
  expect(actionFor(k("d", { ctrl: true }), true)).toBeNull(); // Ctrl+D = EOF
  expect(actionFor(k("c", { meta: true }), true)).toBeNull(); // native copy
});

test("windows uses Ctrl+Shift so Ctrl+K/J/N reach the shell", () => {
  expect(actionFor(k("k", { ctrl: true }), false)).toBeNull();
  expect(actionFor(k("N", { ctrl: true, shift: true }), false)).toBe("newChat");
  expect(actionFor(k("PageDown", { ctrl: true, shift: true }), false)).toBe("nextChat");
  expect(shortcutLabel("toggleTerminal", false)).toBe("Ctrl+Shift+J");
});

test("M2d panel shortcuts", () => {
  expect(actionFor(k("B", { meta: true, shift: true }), true)).toBe("toggleSidebar");
  expect(actionFor(k("R", { meta: true, shift: true }), true)).toBe("toggleRight");
  expect(actionFor(k(",", { meta: true }), true)).toBe("settings");
  expect(actionFor(k("P", { meta: true, shift: true }), true)).toBe("palette");
  expect(actionFor(k("r", { meta: true }), true)).toBeNull(); // plain ⌘R stays free
  expect(actionFor(k("B", { ctrl: true, shift: true }), false)).toBe("toggleSidebar");
  expect(shortcutLabel("settings", true)).toBe("⌘,");
});
