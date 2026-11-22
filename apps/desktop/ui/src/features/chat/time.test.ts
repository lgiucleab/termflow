import { expect, test } from "vitest";
import { ago } from "./time";

test("relative times for session rows", () => {
  const now = Date.parse("2026-11-22T12:00:00Z");
  expect(ago("2026-11-22T11:59:30Z", now)).toBe("now");
  expect(ago("2026-11-22T11:55:00Z", now)).toBe("5m");
  expect(ago("2026-11-22T09:00:00Z", now)).toBe("3h");
  expect(ago("2026-11-20T12:00:00Z", now)).toBe("2d");
  expect(ago("2026-12-01T12:00:00Z", now)).toBe("now"); // clock skew never goes negative
  expect(ago("garbage", now)).toBe("");
});
