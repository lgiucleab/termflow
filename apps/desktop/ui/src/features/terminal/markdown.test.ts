import { expect, test } from "vitest";
import type { Block } from "../../bridge";
import { blockMarkdown, formatDuration } from "./markdown";

const block: Block = {
  id: "b",
  terminal_id: "t",
  seq: 0,
  command: "cargo test",
  cwd: "/w",
  git_branch: "main",
  shell: "zsh",
  exit_code: 101,
  started_at: "2026-11-22T10:00:00Z",
  finished_at: "2026-11-22T10:00:04.200Z",
  chat_session_id: null,
};

test("formats durations", () => {
  expect(formatDuration(120)).toBe("120 ms");
  expect(formatDuration(4200)).toBe("4.2 s");
  expect(formatDuration(65_000)).toBe("1m 5s");
});

test("renders block as markdown with a safe fence", () => {
  const md = blockMarkdown(block, "x ``` y");
  expect(md).toContain("**`$ cargo test`** — exit 101 · 4.2 s · `/w` · (main)");
  expect(md).toContain("````text\nx ``` y\n````");
});
