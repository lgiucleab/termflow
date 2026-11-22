// Tool calls as one-line timeline rows (M2d, OpenCode-style): icon · summary · status, click to
// expand args and output. Approvals moved to the PermissionDock above the composer.
import { createSignal, For, Show } from "solid-js";
import type { Part } from "../../bridge";
import { Icon } from "../../components/Icon";
import { t } from "../../i18n";

type ToolCall = Extract<Part, { type: "tool_call" }>;
type ToolResult = Extract<Part, { type: "tool_result" }>;

export interface Pending {
  summary: string;
  preview: string | null;
}

export interface Todo {
  content: string;
  status: string;
}

/** One-line description of a call from its arguments. */
export function callSummary(c: ToolCall): string {
  const a = (c.args ?? {}) as Record<string, unknown>;
  const s = (k: string) => (typeof a[k] === "string" ? (a[k] as string) : "");
  switch (c.name) {
    case "bash":
      return `$ ${s("command").split("\n")[0]}`;
    case "read":
    case "write":
    case "edit":
    case "ls":
      return `${c.name} ${s("path") || "."}`;
    case "grep":
    case "glob":
      return `${c.name} ${s("pattern")}`;
    case "webfetch":
      return `fetch ${s("url")}`;
    case "apply_patch":
      return `apply_patch ${(s("patch").match(/\*\*\* (?:Add|Update|Delete) File: (.+)/g) ?? []).map((l) => l.split(": ")[1]).join(", ")}`;
    case "todo":
      return t("tools.todo");
    default:
      return c.name;
  }
}

export const todosOf = (c: ToolCall): Todo[] =>
  ((c.args as { todos?: Todo[] })?.todos ?? []).slice(0, 50);

const ICONS: Record<string, string> = {
  bash: "terminal",
  read: "file",
  ls: "file",
  grep: "search",
  glob: "search",
  write: "edit",
  edit: "edit",
  apply_patch: "edit",
  webfetch: "globe",
  todo: "list",
};

function resultText(r: ToolResult): string {
  return r.content.map((c) => c.text).join("");
}

export function TodoList(props: { todos: Todo[] }) {
  return (
    <ul class="tf-todos">
      <For each={props.todos}>
        {(td) => (
          <li data-status={td.status}>
            <span class="tf-todo-glyph" aria-hidden="true">
              {td.status === "completed" ? "✓" : td.status === "in_progress" ? "◐" : "○"}
            </span>
            <span>{td.content}</span>
          </li>
        )}
      </For>
    </ul>
  );
}

/** Colour-coded unified diff (also used by the permission dock). */
export function Diff(props: { text: string }) {
  return (
    <pre class="tf-diff">
      {props.text.split("\n").map((l) => (
        <span
          data-kind={
            l.startsWith("+") && !l.startsWith("+++")
              ? "add"
              : l.startsWith("-") && !l.startsWith("---")
                ? "del"
                : ""
          }
        >
          {l}
          {"\n"}
        </span>
      ))}
    </pre>
  );
}

type Status = "approval" | "error" | "done" | "running" | "queued";

function statusOf(p: { result?: ToolResult; running: boolean; pending?: Pending }): Status {
  if (p.pending) return "approval";
  if (p.result) return p.result.is_error ? "error" : "done";
  return p.running ? "running" : "queued";
}

export function ToolCard(props: {
  call: ToolCall;
  result?: ToolResult;
  running: boolean;
  pending?: Pending;
}) {
  const [open, setOpen] = createSignal(false);
  const status = () => statusOf(props);
  return (
    <div class="tf-tool tf-row-tool" data-status={status()}>
      <button
        type="button"
        class="tf-tool-head"
        aria-expanded={open()}
        onClick={() => setOpen((o) => !o)}
      >
        <span class="tf-tool-icon">
          <Icon name={ICONS[props.call.name] ?? "tool"} size={13} />
        </span>
        <code>{callSummary(props.call)}</code>
        <span class="tf-tool-state">{t(`tools.status.${status()}`)}</span>
        <span class="tf-tool-chevron">
          <Icon name={open() ? "chevronDown" : "chevronRight"} size={12} />
        </span>
      </button>
      <Show when={props.call.name === "todo" && !open()}>
        <TodoList todos={todosOf(props.call)} />
      </Show>
      <Show when={open()}>
        <div class="tf-tool-body">
          <pre class="tf-tool-args">{JSON.stringify(props.call.args, null, 2)}</pre>
          <Show when={props.result}>{(r) => <pre class="tf-tool-out">{resultText(r())}</pre>}</Show>
        </div>
      </Show>
    </div>
  );
}

/** A run of read-only lookups shown as one "Explored N files" row. */
export function ExploreRow(props: {
  calls: ToolCall[];
  results: Map<string, ToolResult>;
  running: ReadonlySet<string>;
}) {
  const [open, setOpen] = createSignal(false);
  const busy = () => props.calls.some((c) => !props.results.has(c.call_id));
  return (
    <div class="tf-tool tf-row-tool" data-status={busy() ? "running" : "done"}>
      <button
        type="button"
        class="tf-tool-head"
        aria-expanded={open()}
        onClick={() => setOpen((o) => !o)}
      >
        <span class="tf-tool-icon">
          <Icon name="search" size={13} />
        </span>
        <span class="tf-tool-label">{t("layout.explored", { n: String(props.calls.length) })}</span>
        <span class="tf-tool-chevron">
          <Icon name={open() ? "chevronDown" : "chevronRight"} size={12} />
        </span>
      </button>
      <Show when={open()}>
        <div class="tf-tool-group">
          <For each={props.calls}>
            {(c) => (
              <ToolCard
                call={c}
                result={props.results.get(c.call_id)}
                running={props.running.has(c.call_id)}
              />
            )}
          </For>
        </div>
      </Show>
    </div>
  );
}
