import { expect, test } from "vitest";
import { matchSlash, mentionQuery, slashQuery } from "./slash";

test("slash query only for a bare /word", () => {
  expect(slashQuery("/pl")).toBe("pl");
  expect(slashQuery("/")).toBe("");
  expect(slashQuery("/plan now")).toBeNull();
  expect(slashQuery("hello")).toBeNull();
  expect(matchSlash("pl").map((c) => c.id)).toEqual(["plan"]);
  expect(matchSlash("").length).toBe(12);
});

test("mention query before the cursor", () => {
  expect(mentionQuery("look at @src/ma", 15)).toEqual({ start: 8, q: "src/ma" });
  expect(mentionQuery("@", 1)).toEqual({ start: 0, q: "" });
  expect(mentionQuery("mail@example.com", 16)).toBeNull();
  expect(mentionQuery("@a.rs done", 10)).toBeNull();
});
