import type { Part } from "../../bridge";

type ToolCall = Extract<Part, { type: "tool_call" }>;

/** Read-only lookups that collapse into one "Explored N files" row when they run back to back. */
const EXPLORE = new Set(["read", "grep", "glob", "ls"]);

export type Row = { kind: "part"; part: Part } | { kind: "explore"; calls: ToolCall[] };

/** Groups runs of 2+ consecutive explore calls; everything else stays one row per part. */
export function groupParts(parts: readonly Part[]): Row[] {
  const out: Row[] = [];
  let run: ToolCall[] = [];
  const flush = () => {
    if (run.length > 1) out.push({ kind: "explore", calls: run });
    else for (const c of run) out.push({ kind: "part", part: c });
    run = [];
  };
  for (const p of parts) {
    if (p.type === "tool_call" && EXPLORE.has(p.name)) run.push(p);
    else {
      flush();
      out.push({ kind: "part", part: p });
    }
  }
  flush();
  return out;
}
