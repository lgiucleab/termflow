import { expect, test } from "vitest";
import { themeById } from "./themes";

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
