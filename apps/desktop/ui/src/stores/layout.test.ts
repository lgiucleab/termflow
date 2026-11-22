import { expect, test } from "vitest";
import {
  BOTTOM,
  bottomHeight,
  clamp,
  SIDEBAR,
  setBottomHeight,
  setSidebarWidth,
  sidebarWidth,
} from "./layout";

test("clamp keeps panel sizes in range", () => {
  expect(clamp(50, 136, 640)).toBe(136);
  expect(clamp(900, 136, 640)).toBe(640);
  expect(clamp(300.6, 136, 640)).toBe(301);
  // A window too small for the max still gets the min.
  expect(clamp(500, 120, 80)).toBe(120);
});

test("sidebar and bottom panel respect their limits", () => {
  setSidebarWidth(10);
  expect(sidebarWidth()).toBe(SIDEBAR.min);
  setSidebarWidth(10_000);
  expect(sidebarWidth()).toBe(SIDEBAR.max);
  setBottomHeight(900, 1000);
  expect(bottomHeight()).toBe(1000 * BOTTOM.maxRatio);
  setBottomHeight(5, 1000);
  expect(bottomHeight()).toBe(BOTTOM.min);
});
