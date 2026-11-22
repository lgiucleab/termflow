// Slash commands typed in the composer (BUILD_PROMPT §6.1).
export type SlashId =
  | "plan"
  | "build"
  | "model"
  | "clear"
  | "attach"
  | "init"
  | "cost"
  | "help"
  | "profile"
  | "compact"
  | "undo"
  | "redo";

export interface SlashCommand {
  id: SlashId;
  /** i18n key for the one-line description. */
  hint: `slash.${SlashId}`;
  /** Milestone that implements it, for placeholders. */
  later?: "M3" | "M4";
}

export const SLASH: SlashCommand[] = [
  { id: "plan", hint: "slash.plan" },
  { id: "build", hint: "slash.build" },
  { id: "model", hint: "slash.model" },
  { id: "clear", hint: "slash.clear" },
  { id: "attach", hint: "slash.attach" },
  { id: "init", hint: "slash.init" },
  { id: "cost", hint: "slash.cost" },
  { id: "help", hint: "slash.help" },
  { id: "profile", hint: "slash.profile", later: "M3" },
  { id: "compact", hint: "slash.compact", later: "M3" },
  { id: "undo", hint: "slash.undo", later: "M4" },
  { id: "redo", hint: "slash.redo", later: "M4" },
];

/** The command being typed, if the input is `/word` with nothing after it. */
export function slashQuery(input: string): string | null {
  const m = /^\/(\w*)$/.exec(input.trim());
  return m ? (m[1] ?? "") : null;
}

export function matchSlash(q: string): SlashCommand[] {
  return SLASH.filter((c) => c.id.startsWith(q.toLowerCase()));
}

/** `@word` right before the cursor: what's being mentioned (null if none). */
export function mentionQuery(input: string, cursor: number): { start: number; q: string } | null {
  const before = input.slice(0, cursor);
  const m = /(^|\s)@([^\s@]*)$/.exec(before);
  if (!m) return null;
  return { start: before.length - (m[2]?.length ?? 0) - 1, q: m[2] ?? "" };
}
