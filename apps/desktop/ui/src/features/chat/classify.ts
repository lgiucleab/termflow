// Decide where the input box sends text: the chat's shell or the agent (decision 0002).
// Heuristic, shown to the user as a glyph before Enter; `!` forces shell, `?` forces agent.

export type InputMode = "shell" | "agent";

const BUILTINS = new Set([
  "cd",
  "export",
  "unset",
  "alias",
  "unalias",
  "source",
  ".",
  "echo",
  "printf",
  "pwd",
  "exit",
  "history",
  "set",
  "pushd",
  "popd",
  "type",
  "which",
  "clear",
  "env",
  "sudo",
  "time",
  "nohup",
  "exec",
  "eval",
  "kill",
  "jobs",
  "fg",
  "bg",
  "wait",
  "test",
  "true",
  "false",
  "ulimit",
  "umask",
  "read",
  "ls",
  "cat",
  "rm",
  "mv",
  "cp",
  "mkdir",
  "touch",
  "git",
  "Get-ChildItem",
  "Set-Location",
  "dir",
]);

/** Words that make a line read like English rather than a command. */
const STOPWORDS = new Set([
  "the",
  "a",
  "an",
  "to",
  "of",
  "is",
  "are",
  "it",
  "this",
  "that",
  "these",
  "those",
  "why",
  "how",
  "what",
  "please",
  "my",
  "our",
  "your",
  "and",
  "for",
  "with",
  "in",
  "on",
  "all",
  "me",
  "us",
  "some",
  "so",
  "be",
  "can",
  "should",
  "would",
  "could",
  "does",
  "do",
  "did",
  "not",
  "into",
  "from",
  "about",
  "i",
  "we",
  "you",
]);

/** Flags, pipes, redirections, globs, assignments, paths: only commands look like this. */
const SHELL_SYNTAX = /(^|\s)--?[\w-]|\||&&|;|>|<|\$\(|`|\w=|\*|\/|\\|^\S+\.(sh|py|js|ts|rs)$/;

const PATH_LIKE = /^(\.{1,2}\/|~\/|\/|[A-Za-z]:\\)/;

export function classify(
  input: string,
  executables: ReadonlySet<string>,
  fallback: InputMode = "agent",
): InputMode {
  const text = input.trim();
  if (!text) return fallback;
  if (text.startsWith("!")) return "shell";
  if (text.startsWith("?")) return "agent";
  if (text.endsWith("?")) return "agent";
  const words = text.split(/\s+/);
  const first = words[0] ?? "";
  if (PATH_LIKE.test(first)) return "shell";
  const isCommand = BUILTINS.has(first) || executables.has(first);
  if (!isCommand) return "agent";
  if (SHELL_SYNTAX.test(text)) return "shell";
  const stop = words
    .slice(1)
    .filter((w) => STOPWORDS.has(w.toLowerCase().replace(/[.,!]$/, ""))).length;
  return stop >= 1 && words.length >= 3 ? "agent" : "shell";
}

/** Strip a leading `!` / `?` override before sending. */
export function stripOverride(input: string): string {
  const t = input.trimStart();
  return t.startsWith("!") || t.startsWith("?") ? t.slice(1).trimStart() : input.trim();
}
