import { createSignal, Show } from "solid-js";
import type { ApprovalKind, Part } from "../../bridge";
import { Icon } from "../../components/Icon";
import { t } from "../../i18n";

type ToolCall = Extract<Part, { type: "tool_call" }>;
type ToolResult = Extract<Part, { type: "tool_result" }>;

export interface Pending {
  summary: string;
  preview: string | null;
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

function resultText(r: ToolResult): string {
  return r.content.map((c) => c.text).join("");
}

function TodoList(props: { args: unknown }) {
  const todos = () =>
    ((props.args as { todos?: { content: string; status: string }[] })?.todos ?? []).slice(0, 50);
  return (
    <ul class="tf-todos">
      {todos().map((td) => (
        <li data-status={td.status}>
          <span aria-hidden="true">
            {td.status === "completed" ? "✓" : td.status === "in_progress" ? "◐" : "○"}
          </span>{" "}
          {td.content}
        </li>
      ))}
    </ul>
  );
}

export function ToolCard(props: {
  call: ToolCall;
  result?: ToolResult;
  running: boolean;
  pending?: Pending;
  onDecide: (kind: ApprovalKind, feedback: string | null) => void;
}) {
  const [open, setOpen] = createSignal(false);
  const [feedback, setFeedback] = createSignal("");
  const [askFeedback, setAskFeedback] = createSignal(false);
  const status = () =>
    props.pending
      ? "approval"
      : props.result
        ? props.result.is_error
          ? "error"
          : "done"
        : props.running
          ? "running"
          : "queued";
  return (
    <div class="tf-toolcard" data-status={status()}>
      <button
        type="button"
        class="tf-toolcard-head"
        aria-expanded={open()}
        onClick={() => setOpen((o) => !o)}
      >
        <Icon name={open() ? "chevronDown" : "chevronRight"} size={11} />
        <span class="tf-tool-status" aria-hidden="true">
          {status() === "done"
            ? "✓"
            : status() === "error"
              ? "✗"
              : status() === "running"
                ? "…"
                : status() === "approval"
                  ? "?"
                  : "·"}
        </span>
        <code>{callSummary(props.call)}</code>
        <span class="tf-tool-state">{t(`tools.status.${status()}`)}</span>
      </button>
      <Show when={props.call.name === "todo"}>
        <TodoList args={props.call.args} />
      </Show>
      <Show when={props.pending}>
        {(p) => (
          <section class="tf-approval" aria-label={t("tools.approvalTitle")}>
            <Show when={p().preview}>
              <pre class="tf-diff">
                {(p().preview ?? "").split("\n").map((l) => (
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
            </Show>
            <Show when={askFeedback()}>
              <input
                class="tf-feedback"
                placeholder={t("tools.feedbackHint")}
                value={feedback()}
                onInput={(e) => setFeedback(e.currentTarget.value)}
                onKeyDown={(e) => e.key === "Enter" && props.onDecide("deny", feedback())}
              />
            </Show>
            <div class="tf-approval-actions">
              <button
                type="button"
                class="tf-primary-sm"
                onClick={() => props.onDecide("allow_once", null)}
              >
                {t("tools.allowOnce")}
              </button>
              <button type="button" onClick={() => props.onDecide("allow_session", null)}>
                {t("tools.allowSession")}
              </button>
              <button type="button" onClick={() => props.onDecide("deny", null)}>
                {t("tools.deny")}
              </button>
              <button
                type="button"
                onClick={() =>
                  askFeedback() ? props.onDecide("deny", feedback()) : setAskFeedback(true)
                }
              >
                {t("tools.denyFeedback")}
              </button>
            </div>
          </section>
        )}
      </Show>
      <Show when={open()}>
        <div class="tf-toolcard-body">
          <pre class="tf-tool-args">{JSON.stringify(props.call.args, null, 2)}</pre>
          <Show when={props.result}>{(r) => <pre class="tf-tool-out">{resultText(r())}</pre>}</Show>
        </div>
      </Show>
    </div>
  );
}
