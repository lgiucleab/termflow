// The unified timeline (decision 0002): one chat = one shell + one agent session. Command
// blocks and agent turns share a timeline; one input box sends to the shell (`$`) or the agent
// (`✦`), auto-detected, with `!` / `?` / Ctrl+I overrides.
import DOMPurify from "dompurify";
import {
  createEffect,
  createMemo,
  createSignal,
  For,
  on,
  onCleanup,
  onMount,
  Show,
} from "solid-js";
import type { AgentSession, ApprovalKind, Block, ModelInfo, Part, Turn } from "../../bridge";
import { useBridge } from "../../bridge/context";
import { Icon } from "../../components/Icon";
import { t } from "../../i18n";
import { isMac } from "../../keybindings";
import { attachments, detach } from "../../stores/agent";
import { onAgentEvent, sessionOf } from "../../stores/agentEvents";
import { busy, markBusy, project, refreshChats } from "../../stores/project";
import {
  credentials,
  LAST_MODEL_KEY,
  type ModelChoice,
  refreshCredentials,
  setProvidersOpen,
} from "../../stores/providers";
import { showToast } from "../../stores/toast";
import { renderMarkdown } from "../agent/markdown";
import { rank } from "../palette/fuzzy";
import { blockMarkdown, formatDuration } from "../terminal/markdown";
import { type ChatShell, type FinishedBlock, shellFor } from "./chatShell";
import { classify, type InputMode, stripOverride } from "./classify";
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
/** Dollars with cents; specta types f64 as nullable (NaN/∞). */
const usd = (v: number | null): string => `$${(v ?? 0).toFixed(2)}`;

/** Local wall-clock time for an RFC 3339 timestamp. */
const clockTime = (iso: string): string =>
  new Date(iso).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });

const errorLabel = (cls: string): string =>
  t(`errors.${(ERROR_CLASSES as readonly string[]).includes(cls) ? (cls as ErrorCls) : "unknown"}`);

type ToolResult = Extract<Part, { type: "tool_result" }>;
/** Recent commands auto-attached to the next agent message. */
const AUTO_ATTACH = 5;
const ATTACH_CAP = 10_000;

interface Live {
  turnId: string;
  model: string;
  text: string;
  reasoning: string;
  at: number;
}

interface MenuItem {
  key: string;
  label: string;
  hint?: string;
  apply: () => void;
}

/** A command block from this run (frozen HTML) or from history (plain text). */
interface BlockItem {
  key: string;
  id?: string;
  command: string;
  exit: number | null;
  at: number;
  ms: number | null;
  cwd?: string | null;
  html?: string;
  text?: string;
}

type Item =
  | { kind: "turn"; at: number; turn: Turn }
  | { kind: "block"; at: number; block: BlockItem };

let executables: ReadonlySet<string> = new Set();
let executablesLoaded = false;

function Markdown(props: { text: string }) {
  return <div class="tf-md" innerHTML={renderMarkdown(props.text)} />;
}

function htmlToText(html: string): string {
  const d = new DOMParser().parseFromString(html, "text/html");
  return d.body.textContent ?? "";
}

function fromFinished(f: FinishedBlock): BlockItem {
  return {
    key: f.key,
    id: f.meta?.id,
    command: f.command || f.meta?.command || "",
    exit: f.exit ?? f.meta?.exit_code ?? null,
    at: f.startedAt,
    ms: f.finishedAt - f.startedAt,
    cwd: f.meta?.cwd,
    html: DOMPurify.sanitize(f.html, { ADD_ATTR: ["style"] }),
  };
}

function fromPast(b: Block, output: string): BlockItem {
  const start = Date.parse(b.started_at);
  return {
    key: b.id,
    id: b.id,
    command: b.command,
    exit: b.exit_code,
    at: start,
    ms: b.finished_at ? Date.parse(b.finished_at) - start : null,
    cwd: b.cwd,
    text: output,
  };
}

function BlockView(props: { block: BlockItem; onRerun: () => void; onAsk: () => void }) {
  const [open, setOpen] = createSignal(true);
  const b = () => props.block;
  const ok = () => b().exit === 0;
  const copy = (s: string, what: string) =>
    void navigator.clipboard.writeText(s).then(() => showToast(t("toast.copied", { what })));
  const outputText = () => b().text ?? htmlToText(b().html ?? "");
  return (
    <div class="tf-block" data-ok={b().exit === null ? "?" : ok() ? "1" : "0"}>
      <div class="tf-block-head">
        <button
          type="button"
          class="tf-block-toggle"
          aria-expanded={open()}
          onClick={() => setOpen((o) => !o)}
        >
          <span class="tf-prompt-glyph" aria-hidden="true">
            $
          </span>
          <code class="tf-block-command">{b().command || t("blocks.unknownCommand")}</code>
        </button>
        <span class="tf-block-actions">
          <button
            type="button"
            title={t("blocks.copyCommand")}
            aria-label={t("blocks.copyCommand")}
            onClick={() => copy(b().command, t("blocks.command"))}
          >
            <Icon name="copy" size={12} />
          </button>
          <button
            type="button"
            title={t("blocks.copyOutput")}
            aria-label={t("blocks.copyOutput")}
            onClick={() => copy(outputText(), t("blocks.output"))}
          >
            <Icon name="output" size={12} />
          </button>
          <button
            type="button"
            title={t("blocks.ask")}
            aria-label={t("blocks.ask")}
            onClick={props.onAsk}
          >
            <Icon name="attach" size={12} />
          </button>
          <button
            type="button"
            title={t("blocks.rerun")}
            aria-label={t("blocks.rerun")}
            onClick={props.onRerun}
          >
            <Icon name="rerun" size={12} />
          </button>
        </span>
        <span class="tf-block-meta">
          <Show when={b().ms !== null}>{formatDuration(b().ms ?? 0)}</Show>
        </span>
        <span
          class="tf-exit"
          data-ok={ok() ? "1" : "0"}
          title={t("blocks.exitCode", { code: String(b().exit ?? "?") })}
        >
          {b().exit === null ? "·" : ok() ? "✓" : `✗ ${b().exit}`}
        </span>
      </div>
      <Show when={open()}>
        <Show
          when={b().html}
          fallback={
            <Show when={b().text?.trim()}>
              <pre class="tf-block-out">{b().text}</pre>
            </Show>
          }
        >
          <div class="tf-block-out tf-block-html" innerHTML={b().html} />
        </Show>
      </Show>
    </div>
  );
}

export function ChatView(props: {
  sessionId: string;
  onNewChat: () => void;
  onAttachLastBlock: () => void;
}) {
  const bridge = useBridge();
  const shell: ChatShell = shellFor(props.sessionId, bridge, project()?.path);
  const [session, setSession] = createSignal<AgentSession | null>(null);
  const [turns, setTurns] = createSignal<Turn[]>([]);
  const [past, setPast] = createSignal<BlockItem[]>([]);
  const [live, setLive] = createSignal<Live | null>(null);
  const [error, setError] = createSignal<{ cls: string; message: string } | null>(null);
  const [notes, setNotes] = createSignal<string[]>([]);
  const [pending, setPending] = createSignal<Record<string, Pending>>({});
  const [running, setRunning] = createSignal<ReadonlySet<string>>(new Set());
  const [models, setModels] = createSignal<ModelInfo[]>([]);
  const [modelsError, setModelsError] = createSignal<string | null>(null);
  const [profiles, setProfiles] = createSignal<string[]>([]);
  void bridge
    .listProfiles()
    .then(setProfiles)
    .catch(() => {});
  const [pickCred, setPickCred] = createSignal("");
  const [pickModel, setPickModel] = createSignal("");
  const [pickReasoning, setPickReasoning] = createSignal(true);
  const [input, setInput] = createSignal("");
  const [cursor, setCursor] = createSignal(0);
  const [menuSel, setMenuSel] = createSignal(0);
  const [files, setFiles] = createSignal<string[] | null>(null);
  const [lock, setLock] = createSignal<InputMode | null>(null);
  const [lastMode, setLastMode] = createSignal<InputMode>("agent");
  const [dismissed, setDismissed] = createSignal<ReadonlySet<string>>(new Set());
  const [history, setHistory] = createSignal<string[]>([]);
  const [histIdx, setHistIdx] = createSignal(-1);
  let list!: HTMLDivElement;
  let box: HTMLTextAreaElement | undefined;
  let modelInput: HTMLInputElement | undefined;
  let rawSlot: HTMLDivElement | undefined;

  const isBusy = () => busy().has(props.sessionId);
  const plan = () => session()?.mode === "plan";
  const shellState = () => shell.state[0]();
  const liveBlock = () => shell.live[0]();
  const mode = (): InputMode => {
    const l = lock();
    if (l) return l;
    return classify(input(), executables, lastMode());
  };

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
    if (!executablesLoaded) {
      executablesLoaded = true;
      executables = new Set(await bridge.shellExecutables().catch(() => []));
    }
    await refreshCredentials(bridge).catch(() => {});
    try {
      const [s, ts, blocks] = await Promise.all([
        bridge.agentSession(props.sessionId),
        bridge.agentTranscript(props.sessionId),
        bridge.chatBlocks(props.sessionId).catch(() => []),
      ]);
      setSession(s);
      setTurns(ts);
      setPast(blocks.map((b) => fromPast(b.block, b.output)).filter((b) => b.at < shell.createdAt));
      setHistory([
        ...blocks.map((b) => b.block.command),
        ...ts
          .filter((x) => x.role === "user")
          .map(
            (x) =>
              splitUserText(x.parts.map((p) => (p.type === "text" ? p.text : "")).join("")).text,
          ),
      ]);
      setPickCred(s.credential_id ?? credentials()[0]?.id ?? "");
      setPickModel(s.model ?? "");
      setPickReasoning(s.reasoning);
      if (pickCred()) void loadModels(pickCred());
    } catch (e) {
      setError({ cls: "unknown", message: String(e) });
    }
    queueMicrotask(() => box?.focus());
    const ro = new ResizeObserver(() => shell.relayout());
    ro.observe(list);
    onCleanup(() => ro.disconnect());
  });

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
        setLive({ turnId: e.turn_id, model: e.model, text: "", reasoning: "", at: Date.now() });
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
      case "route_switched": {
        const reason =
          e.reason === "rate_limit_headers" ? t("route.reasonHeaders") : errorLabel(e.reason);
        const base = t("route.switched", { from: e.from, to: e.to, reason });
        note(e.until ? `${base} ${t("route.backAround", { time: clockTime(e.until) })}` : base);
        break;
      }
      case "route_waiting":
        note(t("route.waiting", { target: e.target, seconds: String(e.seconds) }));
        break;
      case "route_paused": {
        const list = e.candidates
          .map((c) =>
            c.until
              ? `${c.target} (${c.state}, ${clockTime(c.until)})`
              : `${c.target} (${c.state})`,
          )
          .join(", ");
        const tail = e.resume_at
          ? t("route.resumeAt", { time: clockTime(e.resume_at) })
          : t("route.noResume");
        setError({ cls: "overloaded", message: `${t("route.paused", { list })} ${tail}` });
        break;
      }
      case "budget_warning":
        note(
          t("route.budgetWarning", {
            scope: e.scope,
            spent: usd(e.spent_usd),
            limit: usd(e.limit_usd),
          }),
        );
        break;
      case "budget_stopped":
        setLive(null);
        setError({
          cls: "unknown",
          message: t("route.budgetStopped", {
            scope: e.scope,
            spent: usd(e.spent_usd),
            limit: usd(e.limit_usd),
          }),
        });
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

  // ---- timeline ----

  const finishedItems = createMemo(() => shell.finished[0]().map(fromFinished));
  const items = createMemo((): Item[] => {
    const out: Item[] = [
      ...turns()
        .filter((x) => x.role !== "tool")
        .map((turn): Item => ({ kind: "turn", at: Date.parse(turn.created_at), turn })),
      ...[...past(), ...finishedItems()].map(
        (block): Item => ({ kind: "block", at: block.at, block }),
      ),
    ];
    return out.sort((a, b) => a.at - b.at);
  });

  const results = createMemo(() => {
    const m = new Map<string, ToolResult>();
    for (const turn of turns())
      for (const p of turn.parts) if (p.type === "tool_result") m.set(p.call_id, p);
    return m;
  });

  /** Commands run since the agent last spoke, minus the ones the user removed. */
  const autoBlocks = createMemo(() => {
    const lastAgent = Math.max(0, ...turns().map((x) => Date.parse(x.created_at)));
    return [...past(), ...finishedItems()]
      .filter((b) => b.at > lastAgent && !dismissed().has(b.key))
      .slice(-AUTO_ATTACH);
  });

  createEffect(() => {
    items();
    live();
    notes();
    pending();
    liveBlock()?.lines();
    queueMicrotask(() => list?.scrollTo({ top: list.scrollHeight }));
  });

  // Focus follows the running command; back to the input box at the prompt.
  createEffect(
    on(shellState, (st) => {
      if (st === "idle") queueMicrotask(() => box?.focus());
    }),
  );

  createEffect(
    on(
      () => shell.rawOpen[0](),
      (open) => {
        if (open) queueMicrotask(() => rawSlot && shell.mountRaw(rawSlot));
      },
    ),
  );

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

  const applyProfile = async (name: string) => {
    const s = session();
    if (!s) return;
    await bridge.setAgentProfile(s.id, name || null);
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
        if (!s?.model && !s?.profile)
          return setError({ cls: "unknown", message: t("agent.pickModelFirst") });
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

  // ---- input menus (agent mode only) ----

  const mention = () => (mode() === "agent" ? mentionQuery(input(), cursor()) : null);
  createEffect(
    on(mention, async (m) => {
      const p = project();
      if (m && files() === null && p) setFiles(await bridge.projectFiles(p.id).catch(() => []));
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
    if (sq !== null) {
      return matchSlash(sq).map((c) => ({
        key: c.id,
        label: `/${c.id}`,
        hint: t(c.hint),
        apply: () => void runSlash(c),
      }));
    }
    const m = mention();
    if (!m) return [];
    return rank(files() ?? [], m.q, (f) => f)
      .slice(0, 8)
      .map((f) => ({ key: `f:${f}`, label: `@${f}`, apply: () => replaceMention(`@${f} `) }));
  });
  createEffect(on(menu, () => setMenuSel(0)));

  const blockToMarkdown = async (b: BlockItem): Promise<string> => {
    let output = b.text ?? (b.html ? htmlToText(b.html) : "");
    if (!output && b.id) output = (await bridge.blockOutput(b.id).catch(() => null))?.output ?? "";
    if (output.length > ATTACH_CAP) output = `…${output.slice(-ATTACH_CAP)}`;
    const fake = {
      id: b.id ?? b.key,
      terminal_id: "",
      seq: 0,
      command: b.command,
      cwd: b.cwd ?? null,
      git_branch: null,
      shell: "",
      exit_code: b.exit,
      started_at: new Date(b.at).toISOString(),
      finished_at: b.ms === null ? null : new Date(b.at + b.ms).toISOString(),
      chat_session_id: props.sessionId,
    } satisfies Block;
    return blockMarkdown(fake, output);
  };

  const sendToAgent = async (raw: string) => {
    const s = session();
    if (!s) return;
    if (!s.model && !s.profile) {
      setError({ cls: "unknown", message: t("agent.pickModelFirst") });
      return;
    }
    const auto = await Promise.all(autoBlocks().map(blockToMarkdown));
    const manual = attachments().map((a) => a.markdown);
    const ctx = [...auto, ...manual].join("\n\n");
    for (const a of attachments()) detach(a.block.id);
    setDismissed((d) => new Set([...d, ...autoBlocks().map((b) => b.key)]));
    markBusy(s.id, true);
    try {
      await bridge.agentSend(s.id, ctx ? `${ctx}\n\n${raw}` : raw);
    } catch (e) {
      markBusy(s.id, false);
      setError({ cls: "unknown", message: String(e) });
      setInput(raw);
    }
  };

  const submit = async () => {
    const raw = input().trim();
    if (!raw) return;
    const sq = slashQuery(raw);
    const exact = sq !== null ? matchSlash(sq).find((c) => c.id === sq) : undefined;
    if (exact) return runSlash(exact);
    const m = mode();
    const text = stripOverride(raw);
    if (!text) return;
    if (m === "shell") {
      if (!shell.run(text)) {
        showToast(shellState() === "running" ? t("shell.busy") : t("shell.notReady"));
        return;
      }
    } else {
      if (isBusy()) return;
      void sendToAgent(text);
    }
    setHistory((h) => [...h.filter((x) => x !== text), text]);
    setHistIdx(-1);
    setLastMode(m);
    setLock(null);
    setInput("");
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
    const mod = isMac() ? e.metaKey : e.ctrlKey;
    if (mod && e.key.toLowerCase() === "i") {
      e.preventDefault();
      setLock(mode() === "shell" ? "agent" : "shell");
    } else if (e.key === "Tab" && e.shiftKey) {
      e.preventDefault();
      void toggleMode();
    } else if (e.key === "Enter" && !e.shiftKey && !e.isComposing) {
      e.preventDefault();
      void submit();
    } else if (e.key === "Escape") {
      if (isBusy()) void bridge.agentCancel(props.sessionId);
      else if (lock()) setLock(null);
    } else if ((e.key === "ArrowUp" || e.key === "ArrowDown") && !input().includes("\n")) {
      const h = history();
      if (!h.length || (e.key === "ArrowUp" && input() && histIdx() === -1)) return;
      e.preventDefault();
      const idx =
        e.key === "ArrowUp"
          ? histIdx() === -1
            ? h.length - 1
            : Math.max(0, histIdx() - 1)
          : histIdx() + 1;
      if (idx >= h.length || (histIdx() === -1 && e.key === "ArrowDown")) {
        setHistIdx(-1);
        setInput("");
      } else {
        setHistIdx(idx);
        setInput(h[idx] ?? "");
      }
    }
  };

  const onLinkClick = (e: MouseEvent) => {
    const a = (e.target as HTMLElement).closest("a");
    if (!a) return;
    e.preventDefault();
    const href = a.getAttribute("href") ?? "";
    if (/^https?:\/\//.test(href)) void bridge.openUrl(href);
  };

  const placeholder = () => {
    if (shellState() === "running") return t("shell.runningHint");
    if (mode() === "shell") return t("input.shellHint");
    return session()?.model || session()?.profile
      ? t("input.agentHint")
      : t("agent.pickModelFirst");
  };

  const renderTurn = (turn: Turn) => (
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
              <p class="tf-user-text">
                <span class="tf-prompt-glyph" aria-hidden="true">
                  ✦
                </span>{" "}
                {v.text}
              </p>
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
        <span class="tf-served">{t("agent.servedBy", { model: turn.served_by?.model ?? "" })}</span>
      </Show>
    </div>
  );

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
          <Show when={profiles().length > 0}>
            <select
              aria-label={t("route.profile")}
              title={t("route.profileHint")}
              value={session()?.profile ?? ""}
              onChange={(e) => void applyProfile(e.currentTarget.value)}
            >
              <option value="">{t("route.pinned")}</option>
              <For each={profiles()}>{(p) => <option value={p}>{p}</option>}</For>
            </select>
          </Show>
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
        <button
          type="button"
          class="tf-raw-toggle"
          data-on={shell.rawOpen[0]() ? "1" : "0"}
          title={t("shell.rawHint")}
          onClick={() => (shell.rawOpen[0]() ? shell.closeRaw() : shell.rawOpen[1](true))}
        >
          {t("shell.raw")}
        </button>
      </header>
      <Show when={modelsError()}>
        <p class="tf-error" role="alert">
          {t("agent.modelsFailed")}: {modelsError()}
        </p>
      </Show>

      <div class="tf-timeline-wrap">
        {/* biome-ignore lint/a11y/useKeyWithClickEvents: only intercepts link clicks; links stay keyboard-focusable. */}
        <div class="tf-chat" ref={list} role="log" aria-live="polite" onClick={onLinkClick}>
          <Show when={items().length === 0 && !live() && !liveBlock()}>
            <div class="tf-chat-empty">
              <p>
                {project()
                  ? t("agent.emptyProject", { name: project()?.name ?? "" })
                  : t("agent.empty")}
              </p>
              <p class="tf-muted">{t("input.emptyHint")}</p>
            </div>
          </Show>
          <For each={items()}>
            {(it) =>
              it.kind === "turn" ? (
                renderTurn(it.turn)
              ) : (
                <BlockView
                  block={it.block}
                  onRerun={() => shell.run(it.block.command) || showToast(t("shell.busy"))}
                  onAsk={() => {
                    setDismissed((d) => {
                      const n = new Set(d);
                      n.delete(it.block.key);
                      return n;
                    });
                    setLock("agent");
                    box?.focus();
                  }}
                />
              )
            }
          </For>
          <Show when={liveBlock()}>
            {(lb) => (
              <div class="tf-block tf-block-live" data-alt={lb().alt() ? "1" : "0"}>
                <div class="tf-block-head">
                  <span class="tf-prompt-glyph" aria-hidden="true">
                    $
                  </span>
                  <code class="tf-block-command">{lb().command}</code>
                  <span class="tf-block-meta tf-running">{t("shell.running")}</span>
                  <button
                    type="button"
                    class="tf-stop"
                    onClick={() => shell.interrupt()}
                    title="Ctrl+C"
                  >
                    {t("shell.stop")}
                  </button>
                </div>
                <div
                  class="tf-live-slot"
                  style={
                    lb().alt()
                      ? undefined
                      : { height: `${Math.min(24, lb().lines()) * lb().rowH() + 4}px` }
                  }
                  ref={(el) => queueMicrotask(() => shell.mountLive(el))}
                />
              </div>
            )}
          </Show>
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
        <Show when={shell.rawOpen[0]()}>
          <section class="tf-raw" aria-label={t("shell.raw")}>
            <div class="tf-raw-bar">
              <span>{shellState() === "degraded" ? t("shell.degraded") : t("shell.rawTitle")}</span>
              <Show when={shellState() !== "degraded"}>
                <button type="button" onClick={() => shell.closeRaw()}>
                  {t("shell.backToTimeline")}
                </button>
              </Show>
            </div>
            <div class="tf-raw-slot" ref={rawSlot} />
          </section>
        </Show>
      </div>

      <Show when={autoBlocks().length > 0 || attachments().length > 0}>
        <ul class="tf-chips" aria-label={t("agent.attached")}>
          <For each={autoBlocks()}>
            {(b) => (
              <li class="tf-chip" title={t("input.autoAttached")}>
                <span aria-hidden="true">↳</span>
                <code>{b.command}</code>
                <span class="tf-exit" data-ok={b.exit === 0 ? "1" : "0"}>
                  {b.exit === 0 ? "✓" : `✗ ${b.exit ?? "?"}`}
                </span>
                <button
                  type="button"
                  aria-label={t("agent.detach")}
                  onClick={() => setDismissed((d) => new Set([...d, b.key]))}
                >
                  <Icon name="close" size={10} />
                </button>
              </li>
            )}
          </For>
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
      <div class="tf-composer" data-mode={mode()}>
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
        <button
          type="button"
          class="tf-mode-glyph"
          data-locked={lock() ? "1" : "0"}
          title={t("input.modeToggle", { key: isMac() ? "⌘I" : "Ctrl+I" })}
          aria-label={mode() === "shell" ? t("input.toShell") : t("input.toAgent")}
          onClick={() => {
            setLock(mode() === "shell" ? "agent" : "shell");
            box?.focus();
          }}
        >
          {mode() === "shell" ? "$" : "✦"}
        </button>
        <textarea
          ref={box}
          rows={1}
          value={input()}
          placeholder={placeholder()}
          aria-label={placeholder()}
          spellcheck={false}
          onInput={(e) => {
            setInput(e.currentTarget.value);
            setCursor(e.currentTarget.selectionStart ?? e.currentTarget.value.length);
            setHistIdx(-1);
          }}
          onKeyUp={(e) => setCursor(e.currentTarget.selectionStart ?? 0)}
          onClick={(e) => setCursor(e.currentTarget.selectionStart ?? 0)}
          onKeyDown={onKey}
        />
        <span class="tf-composer-meta">
          <Show
            when={isBusy()}
            fallback={<span>{mode() === "shell" ? t("input.shell") : t("input.agent")}</span>}
          >
            <button type="button" onClick={() => void bridge.agentCancel(props.sessionId)}>
              {t("agent.stop")}
            </button>
          </Show>
        </span>
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
