import { expect, test } from "vitest";
import { classify, stripOverride } from "./classify";

const EXE = new Set([
  "make",
  "cargo",
  "npm",
  "python3",
  "open",
  "find",
  "vim",
  "grep",
  "node",
  "pnpm",
]);

test.each([
  ["git status", "shell"],
  ["git commit -m 'fix the thing'", "shell"],
  ["make", "shell"],
  ["make the button blue", "agent"],
  ["cargo test", "shell"],
  ["cargo test -p termflow-agent", "shell"],
  ["ls -la", "shell"],
  ["cd src", "shell"],
  ["npm run build", "shell"],
  ["./run.sh", "shell"],
  ["~/bin/tool --flag", "shell"],
  ["find . -name '*.rs'", "shell"],
  ["find all usages of parse_config", "agent"],
  ["open the readme", "agent"],
  ["python3 -m http.server", "shell"],
  ["vim src/main.rs", "shell"],
  ["echo hello world", "shell"],
  ["why does add fail?", "agent"],
  ["explain this error", "agent"],
  ["fix the failing test", "agent"],
  ["refactor parse_args into its own module", "agent"],
  ["the build is broken", "agent"],
  ["!help", "shell"],
  ["?ls", "agent"],
  ["grep -rn TODO src", "shell"],
])("%s → %s", (input, mode) => {
  expect(classify(input, EXE)).toBe(mode);
});

test("empty input keeps the fallback; overrides are stripped", () => {
  expect(classify("  ", EXE, "shell")).toBe("shell");
  expect(stripOverride("!ls -la")).toBe("ls -la");
  expect(stripOverride("? why")).toBe("why");
  expect(stripOverride(" cargo test ")).toBe("cargo test");
});
