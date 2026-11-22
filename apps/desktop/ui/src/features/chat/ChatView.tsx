import { createEffect, createMemo, createSignal, For, on, onCleanup, Show } from "solid-js";
import type { AgentSession, ApprovalKind, Block, ModelInfo, Part, Turn } from "../../bridge";
import { useBridge } from "../../bridge/context";
import { Icon } from "../../components/Icon";
import { t } from "../../i18n";
import { attach, attachments, detach } from "../../stores/agent";
import { onAgentEvent, sessionOf } from "../../stores/agentEvents";
import { busy, markBusy, project, refreshChats } from "../../stores/project";
import {
  credentials,
  LAST_MODEL_KEY,
  type ModelChoice,
  refreshCredentials,
  setProvidersOpen,
} from "../../stores/providers";
import { renderMarkdown } from "../agent/markdown";
import { rank } from "../palette/fuzzy";
import { blockMarkdown } from "../terminal/markdown";
import { matchSlash, mentionQuery, type SlashCommand, slashQuery } from "./slash";
import { type Pending, ToolCard } from "./ToolCard";
import { splitUserText } from "./userText";

const ERROR_CLASSES = [
  "transient_rate_limit",
  "quota_exhausted",
  "overloaded",
  "context_overflow",
  "auth",
  "content_policy",
  "bad_request",
  "cancelled",
  "unknown",
] as const;
type ErrorCls = (typeof ERROR_CLASSES)[number];
const errorLabel = (cls: string): string =>
  t(`errors.${(ERROR_CLASSES as readonly string[]).includes(cls) ? (cls as ErrorCls) : "unknown"}`);

type ToolResult = Extract<Part, { type: "tool_result" }>;

interface Live {
  turnId: string;
  model: string;
  text: string;
  reasoning: string;
}

interface MenuItem {
  key: string;
  label: string;
  hint?: string;
  apply: () => void;
}

function Markdown(props: { text: string }) {
  return <div class="tf-md" innerHTML={renderMarkdown(props.text)} />;
}

export function ChatView(props: {
  sessionId: string;
  onNewChat: () => void;
  onAttachLastBlock: () => void;
}) {
  const bridge = useBridge();
  const [session, setSession] = createSignal<AgentSession | null>(null);
  const [turns, setTurns] = createSignal<Turn[]>([]);
  const [live, setLive] = createSignal<Live | null>(null);
  const [error, setError] = createSignal<{ cls: string; message: string } | null>(null);
  const [notes, setNotes] = createSignal<string[]>([]);
  const [pending, setPending] = createSignal<Record<string, Pending>>({});
  const [running, setRunning] = createSignal<ReadonlySet<string>>(new Set());
  const [models, setModels] = createSignal<ModelInfo[]>([]);
  const [modelsError, setModelsError] = createSignal<string | null>(null);
  const [pickCred, setPickCred] = createSignal("");
  const [pickModel, setPickModel] = createSignal("");
  const [pickReasoning, setPickReasoning] = createSignal(true);
  const [input, setInput] = createSignal("");
  const [cursor, setCursor] = createSignal(0);
  const [menuSel, setMenuSel] = createSignal(0);
  const [files, setFiles] = createSignal<string[] | null>(null);
  const [blocks, setBlocks] = createSignal<Block[]>([]);
  let list!: HTMLDivElement;
  let box: HTMLTextAreaElement | undefined;
  let modelInput: HTMLInputElement | undefined;

  const isBusy = () => busy().has(props.sessionId);
  const plan = () => session()?.mode === "plan";

  const loadModels = async (credId: string) => {
    setModels([]);
    setModelsError(null);
    if (!credId) return;
    try {
      setModels(await bridge.listModels(credId));
    } catch (e) {
      setModelsError(String(e));
    }
  };

  // (Re)load when the selected chat changes.
  createEffect(
    on(
      () => props.sessionId,
      async (id) => {
        setLive(null);
        setError(null);
        setNotes([]);
        setPending({});
        setRunning(new Set<string>());
        await refreshCredentials(bridge).catch(() => {});
        try {
          const s = await bridge.agentSession(id);
          setSession(s);
          setTurns(await bridge.agentTranscript(id));
          setPickCred(s.credential_id ?? credentials()[0]?.id ?? "");
          setPickModel(s.model ?? "");
          setPickReasoning(s.reasoning);
          if (pickCred()) void loadModels(pickCred());
        } catch (e) {
          setError({ cls: "unknown", message: String(e) });
        }
        queueMicrotask(() => box?.focus());
      },
    ),
  );

  // Batch streamed deltas into one update per frame.
  let pendingDelta = { text: "", reasoning: "" };
  let raf = 0;
  const flush = () => {
    raf = 0;
    const p = pendingDelta;
    pendingDelta = { text: "", reasoning: "" };
    setLive((l) => (l ? { ...l, text: l.text + p.text, reasoning: l.reasoning + p.reasoning } : l));
  };

  const offAgent = onAgentEvent((e) => {
    if (sessionOf(e) !== props.sessionId) return;
    switch (e.type) {
      case "agent_turn_started":
        setError(null);
        setLive({ turnId: e.turn_id, model: e.model, text: "", reasoning: "" });
        break;
      case "agent_delta":
        if (e.reasoning) pendingDelta.reasoning += e.text;
        else pendingDelta.text += e.text;
        if (!raf) raf = requestAnimationFrame(flush);
        break;
      case "agent_turn_committed":
        setTurns((ts) => (ts.some((x) => x.id === e.turn.id) ? ts : [...ts, e.turn]));
        if (e.turn.role === "assistant") setLive(null);
        if (e.turn.role === "tool") {
          const done = new Set(
            e.turn.parts.flatMap((p) => (p.type === "tool_result" ? [p.call_id] : [])),
          );
          setRunning((r) => new Set([...r].filter((x) => !done.has(x))));
        }
        break;
      case "agent_error":
        setLive(null);
        setError({ cls: e.class, message: e.message });
        break;
    }
  });
  const offTools = onToolEvent((e) => {
    if (e.session_id !== props.sessionId) return;
    if (e.type === "tool_approval_needed") {
      setPending((p) => ({ ...p, [e.call_id]: { summary: e.summary, preview: e.preview } }));
    } else if (e.type === "tool_running") {
      setPending((p) => {
        const { [e.call_id]: _, ...rest } = p;
        return rest;
      });
      setRunning((r) => new Set([...r, e.call_id]));
    } else if (e.type === "agent_idle") {
      setPending({});
      setRunning(new Set<string>());
      setLive(null);
      void refreshChats(bridge);
    }
  });
  onCleanup(() => {
    offAgent();
    offTools();
    cancelAnimationFrame(raf);
  });

  createEffect(() => {
    turns();
    live();
    notes();
    pending();
    queueMicrotask(() => list?.scrollTo({ top: list.scrollHeight }));
  });

  const results = createMemo(() => {
    const m = new Map<string, ToolResult>();
    for (const turn of turns())
      for (const p of turn.parts) if (p.type === "tool_result") m.set(p.call_id, p);
    return m;
  });

  const decide = async (callId: string, kind: ApprovalKind, feedback: string | null) => {
    setPending((p) => {
      const { [callId]: _, ...rest } = p;
      return rest;
    });
    await bridge.agentApprove(callId, kind, feedback);
    box?.focus();
  };

  const applyModel = async () => {
    const s = session();
    if (!s || !pickCred() || !pickModel()) return;
    await bridge.setAgentModel(s.id, pickCred(), pickModel(), pickReasoning());
    const choice: ModelChoice = {
      credentialId: pickCred(),
      model: pickModel(),
      reasoning: pickReasoning(),
    };
    await bridge.settingSet(LAST_MODEL_KEY, JSON.stringify(choice));
    setSession(await bridge.agentSession(s.id));
  };

  const toggleMode = async () => {
    const s = session();
    if (!s) return;
    await bridge.setAgentMode(s.id, !plan());
    setSession(await bridge.agentSession(s.id));
  };

  const note = (text: string) => setNotes((n) => [...n, text]);

  const runSlash = async (c: SlashCommand) => {
    setInput("");
    const s = session();
    if (c.later) return note(t("slash.later", { cmd: `/${c.id}`, milestone: c.later }));
    switch (c.id) {
      case "plan":
      case "build":
        if (s && plan() !== (c.id === "plan")) await toggleMode();
        return note(t(c.id === "plan" ? "slash.nowPlan" : "slash.nowBuild"));
      case "model":
        return queueMicrotask(() => modelInput?.focus());
      case "clear":
        return props.onNewChat();
      case "attach":
        return props.onAttachLastBlock();
      case "init":
        if (!s?.model) return setError({ cls: "unknown", message: t("agent.pickModelFirst") });
        markBusy(props.sessionId, true);
        return bridge.agentInit(props.sessionId).catch((e) => {
          markBusy(props.sessionId, false);
          setError({ cls: "unknown", message: String(e) });
        });
      case "cost": {
        const u = await bridge.agentUsage(props.sessionId);
        return note(
          t("slash.costLine", {
            requests: String(u.requests ?? 0),
            input: (u.input ?? 0).toLocaleString(),
            output: (u.output ?? 0).toLocaleString(),
            cached: (u.cache_read ?? 0).toLocaleString(),
            reasoning: (u.reasoning ?? 0).toLocaleString(),
          }),
        );
      }
      case "help":
        return note(t("slash.helpText"));
    }
  };

  // ---- composer menus ----

  const mention = () => mentionQuery(input(), cursor());
  createEffect(
    on(mention, async (m) => {
      if (!m) return;
      const p = project();
      if (files() === null && p) setFiles(await bridge.projectFiles(p.id).catch(() => []));
      if ("block".startsWith(m.q.toLowerCase()) || m.q.toLowerCase().startsWith("block"))
        setBlocks(await bridge.recentBlocks(8).catch(() => []));
    }),
  );

  const replaceMention = (with_: string) => {
    const m = mention();
    if (!m) return;
    const v = input();
    const next = `${v.slice(0, m.start)}${with_}${v.slice(cursor())}`;
    setInput(next);
    const pos = m.start + with_.length;
    queueMicrotask(() => {
      box?.setSelectionRange(pos, pos);
      setCursor(pos);
    });
  };

  const menu = createMemo((): MenuItem[] => {
    const sq = slashQuery(input());
    if (sq !== null)
      return matchSlash(sq).map((c) => ({
        key: c.id,
        label: `/${c.id}`,
        hint: t(c.hint),
        apply: () => void runSlash(c),
      }));
    const m = mention();
    if (!m) return [];
    const fileItems = rank(files() ?? [], m.q, (f) => f)
      .slice(0, 8)
      .map((f) => ({ key: `f:${f}`, label: `@${f}`, apply: () => replaceMention(`@${f} `) }));
    const blockItems = (
      "block".startsWith(m.q.toLowerCase()) || m.q.toLowerCase().startsWith("block") ? blocks() : []
    ).map((b) => ({
      key: `b:${b.id}`,
      label: t("agent.mentionBlock", { cmd: b.command }),
      hint: t("blocks.exitCode", { code: String(b.exit_code ?? "?") }),
      apply: () => {
        replaceMention("");
        void bridge
          .blockOutput(b.id)
          .then((o) => attach({ block: b, markdown: blockMarkdown(b, o?.output ?? "") }));
      },
    }));
    return [...fileItems, ...blockItems];
  });
  createEffect(on(menu, () => setMenuSel(0)));

  const send = async () => {
    const s = session();
    const raw = input().trim();
    if (!s || !raw || isBusy()) return;
    const sq = slashQuery(raw);
    const exact = sq !== null ? matchSlash(sq).find((c) => c.id === sq) : undefined;
    if (exact) return runSlash(exact);
    const ctx = attachments()
      .map((a) => a.markdown)
      .join("\n\n");
    setInput("");
    for (const a of attachments()) detach(a.block.id);
    markBusy(s.id, true);
    try {
      await bridge.agentSend(s.id, ctx ? `${ctx}\n\n${raw}` : raw);
    } catch (e) {
      markBusy(s.id, false);
      setError({ cls: "unknown", message: String(e) });
      setInput(raw);
    }
  };

  const onKey = (e: KeyboardEvent) => {
    const items = menu();
    if (items.length > 0) {
      if (e.key === "ArrowDown" || e.key === "ArrowUp") {
        e.preventDefault();
        setMenuSel((i) => (i + (e.key === "ArrowDown" ? 1 : items.length - 1)) % items.length);
        return;
      }
      if ((e.key === "Enter" && !e.shiftKey) || e.key === "Tab") {
        e.preventDefault();
        items[menuSel()]?.apply();
        return;
      }
      if (e.key === "Escape") {
        e.preventDefault();
        setInput(`${input()} `);
        return;
      }
    }
    if (e.key === "Tab" && e.shiftKey) {
      e.preventDefault();
      void toggleMode();
    } else if (e.key === "Enter" && !e.shiftKey && !e.isComposing) {
      e.preventDefault();
      void send();
    } else if (e.key === "Escape" && isBusy()) {
      void bridge.agentCancel(props.sessionId);
    }
  };

  const onLinkClick = (e: MouseEvent) => {
    const a = (e.target as HTMLElement).closest("a");
    if (!a) return;
    e.preventDefault();
    const href = a.getAttribute("href") ?? "";
    if (/^https?:\/\//.test(href)) void bridge.openUrl(href);
  };

  return (
    <div class="tf-chatview">
      <header class="tf-agent-head">
        <button
          type="button"
          class="tf-mode"
          data-plan={plan() ? "1" : "0"}
          title={t("agent.modeHint")}
          onClick={() => void toggleMode()}
        >
          {plan() ? t("agent.plan") : t("agent.build")}
        </button>
        <span class="tf-agent-model" title={session()?.model ?? ""}>
          {session()?.title ?? t("chats.untitled")}
        </span>
        <Show
          when={credentials().length > 0}
          fallback={
            <button type="button" class="tf-primary-sm" onClick={() => setProvidersOpen(true)}>
              {t("agent.addProvider")}
            </button>
          }
        >
          <select
            aria-label={t("agent.credential")}
            value={pickCred()}
            onChange={(e) => {
              setPickCred(e.currentTarget.value);
              setPickModel("");
              void loadModels(e.currentTarget.value);
            }}
          >
            <For each={credentials()}>{(c) => <option value={c.id}>{c.label}</option>}</For>
          </select>
          <input
            ref={modelInput}
            list={`tf-models-${props.sessionId}`}
            aria-label={t("agent.model")}
            placeholder={t("agent.modelHint")}
            value={pickModel()}
            onInput={(e) => setPickModel(e.currentTarget.value)}
            onKeyDown={(e) => e.key === "Enter" && void applyModel()}
          />
          <datalist id={`tf-models-${props.sessionId}`}>
            <For each={models()}>
              {(m) => <option value={m.id}>{m.display_name ?? m.id}</option>}
            </For>
          </datalist>
          <label class="tf-check">
            <input
              type="checkbox"
              checked={pickReasoning()}
              onChange={(e) => setPickReasoning(e.currentTarget.checked)}
            />
            {t("agent.reasoningToggle")}
          </label>
          <button
            type="button"
            onClick={() => void applyModel()}
            disabled={
              !pickCred() ||
              !pickModel() ||
              (pickModel() === session()?.model &&
                pickCred() === session()?.credential_id &&
                pickReasoning() === session()?.reasoning)
            }
          >
            {t("agent.useModel")}
          </button>
        </Show>
      </header>
      <Show when={modelsError()}>
        <p class="tf-error" role="alert">
          {t("agent.modelsFailed")}: {modelsError()}
        </p>
      </Show>

      {/* biome-ignore lint/a11y/useKeyWithClickEvents: only intercepts link clicks; links stay keyboard-focusable. */}
      <div class="tf-chat" ref={list} role="log" aria-live="polite" onClick={onLinkClick}>
        <Show when={turns().length === 0 && !live()}>
          <div class="tf-chat-empty">
            <p>
              {project()
                ? t("agent.emptyProject", { name: project()?.name ?? "" })
                : t("agent.empty")}
            </p>
            <p class="tf-muted">{t("agent.emptyHint")}</p>
          </div>
        </Show>
        <For each={turns().filter((x) => x.role !== "tool")}>
          {(turn) => (
            <div class="tf-msg" data-role={turn.role}>
              <Show
                when={turn.role === "user"}
                fallback={
                  <For each={turn.parts}>
                    {(p) => {
                      if (p.type === "text") return <Markdown text={p.text} />;
                      if (p.type === "reasoning")
                        return (
                          <Show when={p.summary_text}>
                            <details class="tf-reasoning">
                              <summary>{t("agent.reasoning")}</summary>
                              <p>{p.summary_text}</p>
                            </details>
                          </Show>
                        );
                      if (p.type === "tool_call")
                        return (
                          <ToolCard
                            call={p}
                            result={results().get(p.call_id)}
                            running={running().has(p.call_id)}
                            pending={pending()[p.call_id]}
                            onDecide={(k, f) => void decide(p.call_id, k, f)}
                          />
                        );
                      return null;
                    }}
                  </For>
                }
              >
                {(() => {
                  const v = splitUserText(
                    turn.parts.map((p) => (p.type === "text" ? p.text : "")).join(""),
                  );
                  return (
                    <>
                      <p class="tf-user-text">{v.text}</p>
                      <For each={v.files}>
                        {(f) => (
                          <details class="tf-attached">
                            <summary>
                              <Icon name="attach" size={11} /> {f.path}
                            </summary>
                            <pre>{f.body}</pre>
                          </details>
                        )}
                      </For>
                    </>
                  );
                })()}
              </Show>
              <Show when={turn.role === "assistant" && turn.served_by}>
                <span class="tf-served">
                  {t("agent.servedBy", { model: turn.served_by?.model ?? "" })}
                </span>
              </Show>
            </div>
          )}
        </For>
        <Show when={live()}>
          {(l) => (
            <div class="tf-msg" data-role="assistant" aria-busy="true">
              <Show when={l().reasoning}>
                <details class="tf-reasoning" open>
                  <summary>{t("agent.thinking")}</summary>
                  <p>{l().reasoning}</p>
                </details>
              </Show>
              <Markdown text={l().text || "…"} />
              <span class="tf-served">{t("agent.servedBy", { model: l().model })}</span>
            </div>
          )}
        </Show>
        <For each={notes()}>{(n) => <div class="tf-note">{n}</div>}</For>
        <Show when={error()}>
          {(err) => (
            <div class="tf-error-banner" role="alert">
              <strong>{errorLabel(err().cls)}</strong> {err().message}
              <button type="button" aria-label={t("common.close")} onClick={() => setError(null)}>
                <Icon name="close" size={12} />
              </button>
            </div>
          )}
        </Show>
      </div>

      <Show when={attachments().length > 0}>
        <ul class="tf-chips" aria-label={t("agent.attached")}>
          <For each={attachments()}>
            {(a) => (
              <li class="tf-chip">
                <code>{a.block.command}</code>
                <button
                  type="button"
                  aria-label={t("agent.detach")}
                  onClick={() => detach(a.block.id)}
                >
                  <Icon name="close" size={10} />
                </button>
              </li>
            )}
          </For>
        </ul>
      </Show>
      <div class="tf-composer">
        <Show when={menu().length > 0}>
          <div class="tf-menu" role="listbox" aria-label={t("agent.suggestions")}>
            <For each={menu()}>
              {(item, i) => (
                <div
                  role="option"
                  tabIndex={-1}
                  aria-selected={i() === menuSel()}
                  data-selected={i() === menuSel() ? "1" : "0"}
                  onMouseDown={(e) => {
                    e.preventDefault();
                    item.apply();
                  }}
                  onKeyDown={(e) => e.key === "Enter" && item.apply()}
                >
                  <span>{item.label}</span>
                  <Show when={item.hint}>
                    <span class="tf-muted">{item.hint}</span>
                  </Show>
                </div>
              )}
            </For>
          </div>
        </Show>
        <textarea
          ref={box}
          rows={2}
          value={input()}
          placeholder={session()?.model ? t("agent.placeholder") : t("agent.pickModelFirst")}
          aria-label={t("agent.placeholder")}
          onInput={(e) => {
            setInput(e.currentTarget.value);
            setCursor(e.currentTarget.selectionStart ?? e.currentTarget.value.length);
          }}
          onKeyUp={(e) => setCursor(e.currentTarget.selectionStart ?? 0)}
          onClick={(e) => setCursor(e.currentTarget.selectionStart ?? 0)}
          onKeyDown={onKey}
        />
        <Show
          when={isBusy()}
          fallback={
            <button
              type="button"
              class="tf-primary-sm"
              disabled={!input().trim() || !session()?.model}
              onClick={() => void send()}
            >
              {t("agent.send")}
            </button>
          }
        >
          <button type="button" onClick={() => void bridge.agentCancel(props.sessionId)}>
            {t("agent.stop")}
          </button>
        </Show>
      </div>
    </div>
  );
}

// Tool and idle events reach chat views through this small fan-out (set up by App).
type ToolEvent = Extract<
  import("../../bridge").AppEvent,
  { type: "tool_approval_needed" | "tool_running" | "agent_idle" }
>;
const toolListeners = new Set<(e: ToolEvent) => void>();
export function onToolEvent(l: (e: ToolEvent) => void): () => void {
  toolListeners.add(l);
  return () => toolListeners.delete(l);
}
export function dispatchToolEvent(e: import("../../bridge").AppEvent): boolean {
  if (e.type !== "tool_approval_needed" && e.type !== "tool_running" && e.type !== "agent_idle")
    return false;
  if (e.type === "agent_idle") markBusy(e.session_id, false);
  for (const l of toolListeners) l(e);
  return true;
}
