import { expect, test } from "vitest";
import { actionFor } from "./keybindings";

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

test("mac uses Cmd and leaves Ctrl to the terminal", () => {
  expect(actionFor(k("k", { meta: true }), true)).toBe("palette");
  expect(actionFor(k("D", { meta: true, shift: true }), true)).toBe("splitDown");
  expect(actionFor(k("d", { ctrl: true }), true)).toBeNull(); // Ctrl+D = EOF
  expect(actionFor(k("c", { meta: true }), true)).toBeNull(); // native copy
});

test("windows uses Ctrl+Shift so Ctrl+W/D/K reach the shell", () => {
  expect(actionFor(k("w", { ctrl: true }), false)).toBeNull();
  expect(actionFor(k("d", { ctrl: true }), false)).toBeNull();
  expect(actionFor(k("T", { ctrl: true, shift: true }), false)).toBe("newTab");
  expect(actionFor(k("E", { ctrl: true, shift: true }), false)).toBe("splitDown");
});
