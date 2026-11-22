import type { Block } from "../../bridge";

export function formatDuration(ms: number): string {
  if (ms < 1000) return `${Math.round(ms)} ms`;
  if (ms < 60_000) return `${(ms / 1000).toFixed(1)} s`;
  const m = Math.floor(ms / 60_000);
  return `${m}m ${Math.round((ms % 60_000) / 1000)}s`;
}

export function blockDurationMs(b: Block): number | null {
  if (!b.finished_at) return null;
  return Date.parse(b.finished_at) - Date.parse(b.started_at);
}

/** Markdown for sharing or attaching a block. Fences grow if the output contains backticks. */
export function blockMarkdown(b: Block, output: string): string {
  const meta = [
    b.exit_code === null ? "exit ?" : `exit ${b.exit_code}`,
    blockDurationMs(b) === null ? null : formatDuration(blockDurationMs(b) ?? 0),
    b.cwd ? `\`${b.cwd}\`` : null,
    b.git_branch ? `(${b.git_branch})` : null,
  ]
    .filter(Boolean)
    .join(" · ");
  const longest = Math.max(2, ...[...output.matchAll(/`+/g)].map((m) => m[0].length));
  const fence = "`".repeat(longest + 1);
  const body = output.endsWith("\n") ? output : `${output}\n`;
  return `**\`$ ${b.command}\`** — ${meta}\n\n${fence}text\n${body}${fence}\n`;
}
