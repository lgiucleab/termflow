import { expect, test } from "vitest";
import { THEMES, themeById } from "./themes";

function lum(hex: string): number {
  const c = [1, 3, 5].map((i) => Number.parseInt(hex.slice(i, i + 2), 16) / 255);
  const [r, g, b] = c.map((v) => (v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4)) as [
    number,
    number,
    number,
  ];
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}

const contrast = (a: string, b: string) => {
  const [x, y] = [lum(a), lum(b)].sort((p, q) => q - p) as [number, number];
  return (x + 0.05) / (y + 0.05);
};

test.each(["mono-dark", "mono-light"])("%s meets WCAG AA", (id) => {
  const t = themeById(id);
  expect(t.id).toBe(id);
  for (const bg of [t.ui.bg, t.ui.surface, t.ui.surface2]) {
    expect(contrast(t.ui.text, bg)).toBeGreaterThanOrEqual(4.5);
    expect(contrast(t.ui.muted, bg)).toBeGreaterThanOrEqual(4.5);
    expect(contrast(t.ui.accent, bg)).toBeGreaterThanOrEqual(3); // focus rings, UI components
    expect(contrast(t.ui.success, bg)).toBeGreaterThanOrEqual(3);
    expect(contrast(t.ui.danger, bg)).toBeGreaterThanOrEqual(3);
  }
});

test("Mono Dark is the default", () => {
  expect(themeById(undefined).id).toBe("mono-dark");
  expect(themeById("nope").id).toBe("mono-dark");
});

test("every theme defines every semantic token", () => {
  const keys = Object.keys(themeById("mono-dark").ui).sort();
  for (const t of THEMES) {
    expect(Object.keys(t.ui).sort(), t.id).toEqual(keys);
    for (const v of Object.values(t.ui)) expect(v, t.id).toMatch(/^#[0-9a-f]{6}([0-9a-f]{2})?$/i);
    // The live xterm sits on the panel, so its background must match it.
    expect(t.terminal.background, t.id).toBe(t.ui.bg);
  }
});
