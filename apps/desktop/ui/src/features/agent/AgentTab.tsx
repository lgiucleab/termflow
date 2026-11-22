import { createEffect, createSignal, For, onCleanup, onMount, Show } from "solid-js";
import type { AgentSession, ModelInfo, Part, Turn } from "../../bridge";
import { useBridge } from "../../bridge/context";
import { Icon } from "../../components/Icon";
import { t } from "../../i18n";
import { attachments, detach } from "../../stores/agent";
import { onAgentEvent, sessionOf } from "../../stores/agentEvents";
import {
  credentials,
  LAST_MODEL_KEY,
  type ModelChoice,
  refreshCredentials,
  setProvidersOpen,
} from "../../stores/providers";
import type { Pane } from "../layout/model";
import { renderMarkdown } from "./markdown";

interface Live {
  turnId: string;
  model: string;
  text: string;
  reasoning: string;
}

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

function Markdown(props: { text: string }) {
  return <div class="tf-md" innerHTML={renderMarkdown(props.text)} />;
}

function AssistantParts(props: { parts: Part[] }) {
  return (
    <For each={props.parts}>
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
            <div class="tf-toolcard">
              <code>{p.name}</code>{" "}
              <span class="tf-muted">{JSON.stringify(p.args).slice(0, 120)}</span>
            </div>
          );
        return null;
      }}
    </For>
  );
}

export function AgentTab(props: {
  pane: Pane;
  focused: boolean;
  onFocus: () => void;
  onSession: (paneId: string, sessionId: string) => void;
}) {
  const bridge = useBridge();
  const [session, setSession] = createSignal<AgentSession | null>(null);
  const [turns, setTurns] = createSignal<Turn[]>([]);
  const [live, setLive] = createSignal<Live | null>(null);
  const [error, setError] = createSignal<{ cls: string; message: string } | null>(null);
  const [models, setModels] = createSignal<ModelInfo[]>([]);
  const [modelsError, setModelsError] = createSignal<string | null>(null);
  const [pickCred, setPickCred] = createSignal("");
  const [pickModel, setPickModel] = createSignal("");
  const [pickReasoning, setPickReasoning] = createSignal(true);
  const [input, setInput] = createSignal("");
  let list!: HTMLDivElement;
  let box: HTMLTextAreaElement | undefined;

  const sid = () => session()?.id;

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

  onMount(async () => {
    await refreshCredentials(bridge).catch(() => {});
    const existing = props.pane.kind.type === "agent" ? props.pane.kind.sessionId : undefined;
    try {
      let s: AgentSession | null = null;
      if (existing) s = await bridge.agentSession(existing).catch(() => null);
      if (!s) {
        const last = JSON.parse(
          (await bridge.settingGet(LAST_MODEL_KEY)) ?? "null",
        ) as ModelChoice | null;
        const valid = last && credentials().some((c) => c.id === last.credentialId);
        s = await bridge.createAgentSession(
          valid ? last.credentialId : null,
          valid ? last.model : null,
        );
        if (valid && !last.reasoning)
          await bridge.setAgentModel(s.id, last.credentialId, last.model, false);
        props.onSession(props.pane.id, s.id);
      }
      setSession(s);
      setTurns(await bridge.agentTranscript(s.id));
      setPickCred(s.credential_id ?? credentials()[0]?.id ?? "");
      setPickModel(s.model ?? "");
      setPickReasoning(s.reasoning);
      if (pickCred()) void loadModels(pickCred());
    } catch (e) {
      setError({ cls: "unknown", message: String(e) });
    }
  });

  // Batch streamed deltas into one update per frame.
  let pending = { text: "", reasoning: "" };
  let raf = 0;
  const flush = () => {
    raf = 0;
    const p = pending;
    pending = { text: "", reasoning: "" };
    setLive((l) => (l ? { ...l, text: l.text + p.text, reasoning: l.reasoning + p.reasoning } : l));
  };

  const off = onAgentEvent((e) => {
    if (sessionOf(e) !== sid()) return;
    switch (e.type) {
      case "agent_turn_started":
        setError(null);
        setLive({ turnId: e.turn_id, model: e.model, text: "", reasoning: "" });
        break;
      case "agent_delta":
        if (e.reasoning) pending.reasoning += e.text;
        else pending.text += e.text;
        if (!raf) raf = requestAnimationFrame(flush);
        break;
      case "agent_turn_committed":
        setTurns((ts) => (ts.some((x) => x.id === e.turn.id) ? ts : [...ts, e.turn]));
        if (e.turn.role === "assistant") setLive(null);
        break;
      case "agent_error":
        setLive(null);
        setError({ cls: e.class, message: e.message });
        break;
    }
  });
  onCleanup(() => {
    off();
    cancelAnimationFrame(raf);
  });

  createEffect(() => {
    turns();
    live();
    queueMicrotask(() => list?.scrollTo({ top: list.scrollHeight }));
  });
  createEffect(() => {
    if (props.focused) queueMicrotask(() => box?.focus());
  });

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

  const send = async () => {
    const s = session();
    const text = input().trim();
    if (!s || !text || live()) return;
    const ctx = attachments()
      .map((a) => a.markdown)
      .join("\n\n");
    setInput("");
    for (const a of attachments()) detach(a.block.id);
    try {
      await bridge.agentSend(s.id, ctx ? `${ctx}\n\n${text}` : text);
    } catch (e) {
      setError({ cls: "unknown", message: String(e) });
      setInput(text);
    }
  };

  const onLinkClick = (e: MouseEvent) => {
    const a = (e.target as HTMLElement).closest("a");
    if (!a) return;
    e.preventDefault();
    const href = a.getAttribute("href") ?? "";
    if (/^https?:\/\//.test(href)) void bridge.openUrl(href);
  };

  const modelLabel = () => {
    const s = session();
    if (!s?.model) return t("agent.noModel");
    const c = credentials().find((x) => x.id === s.credential_id);
    return `${s.model}${c ? ` · ${c.label}` : ""}`;
  };

  return (
    // biome-ignore lint/a11y/noStaticElementInteractions: mouse focus follows click; keyboard focus moves with pane shortcuts.
    <div
      class="tf-agent"
      data-focused={props.focused ? "1" : "0"}
      onMouseDown={() => props.onFocus()}
    >
      <header class="tf-agent-head">
        <span class="tf-agent-model" title={modelLabel()}>
          {modelLabel()}
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
            list={`tf-models-${props.pane.id}`}
            aria-label={t("agent.model")}
            placeholder={t("agent.modelHint")}
            value={pickModel()}
            onInput={(e) => setPickModel(e.currentTarget.value)}
          />
          <datalist id={`tf-models-${props.pane.id}`}>
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
            disabled={!pickCred() || !pickModel()}
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
          <p class="tf-muted tf-chat-empty">{t("agent.empty")}</p>
        </Show>
        <For each={turns()}>
          {(turn) => (
            <div class="tf-msg" data-role={turn.role}>
              <Show when={turn.role === "user"} fallback={<AssistantParts parts={turn.parts} />}>
                <p class="tf-user-text">
                  {turn.parts.map((p) => (p.type === "text" ? p.text : "")).join("")}
                </p>
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
        <textarea
          ref={box}
          rows={2}
          value={input()}
          placeholder={session()?.model ? t("agent.placeholder") : t("agent.pickModelFirst")}
          aria-label={t("agent.placeholder")}
          onInput={(e) => setInput(e.currentTarget.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && !e.shiftKey && !e.isComposing) {
              e.preventDefault();
              void send();
            } else if (e.key === "Escape" && live()) {
              const s = sid();
              if (s) void bridge.agentCancel(s);
            }
          }}
        />
        <Show
          when={live()}
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
          <button
            type="button"
            onClick={() => {
              const s = sid();
              if (s) void bridge.agentCancel(s);
            }}
          >
            {t("agent.stop")}
          </button>
        </Show>
      </div>
    </div>
  );
}
