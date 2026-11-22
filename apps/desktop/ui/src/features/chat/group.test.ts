import { expect, test } from "vitest";
import type { Part } from "../../bridge";
import { groupParts } from "./group";

const call = (name: string, id = name): Part => ({
  type: "tool_call",
  call_id: id,
  name,
  args: {},
});
const text = (t: string): Part => ({ type: "text", text: t });

test("back-to-back lookups collapse into one explore row", () => {
  const rows = groupParts([
    text("a"),
    call("read", "1"),
    call("grep", "2"),
    call("glob", "3"),
    text("b"),
  ]);
  expect(rows.map((r) => r.kind)).toEqual(["part", "explore", "part"]);
  const ex = rows[1];
  expect(ex?.kind === "explore" && ex.calls.map((c) => c.call_id)).toEqual(["1", "2", "3"]);
});

test("a single lookup, or one broken by an edit, stays a normal row", () => {
  expect(groupParts([call("read")]).map((r) => r.kind)).toEqual(["part"]);
  const rows = groupParts([call("read", "1"), call("edit", "2"), call("read", "3")]);
  expect(rows.map((r) => r.kind)).toEqual(["part", "part", "part"]);
});

test("a trailing run is flushed", () => {
  expect(groupParts([text("x"), call("ls", "1"), call("read", "2")]).map((r) => r.kind)).toEqual([
    "part",
    "explore",
  ]);
});
