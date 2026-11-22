import { expect, test } from "vitest";
import { fuzzyScore, rank } from "./fuzzy";

test("matches subsequences and rejects others", () => {
  expect(fuzzyScore("nt", "New terminal tab")).not.toBeNull();
  expect(fuzzyScore("xyz", "New terminal tab")).toBeNull();
  expect(fuzzyScore("", "anything")).toBe(0);
});

test("ranks word starts and consecutive runs higher", () => {
  const items = ["Split down", "Theme: Nord", "New terminal tab", "Split right"];
  expect(rank(items, "split r", (s) => s)[0]).toBe("Split right");
  expect(rank(items, "nord", (s) => s)).toEqual(["Theme: Nord"]);
  expect(rank(items, "ntt", (s) => s)[0]).toBe("New terminal tab");
});
