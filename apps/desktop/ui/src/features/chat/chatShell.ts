// One shell per chat, shown as Warp-style blocks in the chat's timeline (decision 0002).
//
// The shell's own prompt is hidden: bytes between OSC 133 A and C are dropped, and the input
// box replaces the prompt. Output after C goes to the *live block* (a real xterm, so colours,
// progress bars and interactive input work). On D the block is frozen to static HTML and the
// xterm is disposed: at most one xterm per chat is alive. Alternate-screen programs (vim, htop,
// less) expand the live block over the whole timeline. Without shell integration the chat falls
// back to the raw terminal.
import { FitAddon } from "@xterm/addon-fit";
import { SerializeAddon } from "@xterm/addon-serialize";
import { Unicode11Addon } from "@xterm/addon-unicode11";
import { WebLinksAddon } from "@xterm/addon-web-links";
import { Terminal } from "@xterm/xterm";
import { createSignal } from "solid-js";
import type { AppEvent, Block, Bridge, TerminalId } from "../../bridge";
import { config, fontFamily, theme } from "../../stores/settings";
import { recordBytes } from "../terminal/perf";
import { StreamSplitter } from "./splitter";

export type ShellState = "starting" | "idle" | "running" | "degraded" | "exited";

/** Inline TUIs that draw over many lines without the alternate screen: open in the raw view. */
const INLINE_TUIS = new Set(["claude", "codex", "gemini", "opencode", "aider", "copilot", "top"]);
/** Rows the PTY reports while a command runs in a block (blocks show only the used lines). */
const BLOCK_ROWS = 24;
const RAW_LOG_CAP = 4 * 1024 * 1024;
/** No prompt mark this long after start → no shell integration → raw terminal. */
const INTEGRATION_TIMEOUT_MS = 6000;

export interface LiveBlock {
  command: string;
  startedAt: number;
  term: Terminal;
  el: HTMLDivElement;
  fit: FitAddon;
  serialize: SerializeAddon;
  /** Lines currently used (drives the visible height). */
  lines: () => number;
  alt: () => boolean;
  /** Pixel height of one terminal row, measured after the xterm opens. */
  rowH: () => number;
}

export interface FinishedBlock {
  key: string;
  command: string;
  exit: number | null;
  startedAt: number;
  finishedAt: number;
  html: string;
  /** Filled when the core's BlockFinished event arrives (cwd, branch, id). */
  meta?: Block;
}

function newTerm(rows: number, cols: number): Terminal {
  const cfg = config().terminal;
  return new Terminal({
    allowProposedApi: true,
    fontFamily: fontFamily(),
    fontSize: cfg.font_size,
    scrollback: 5000,
    rows,
    cols,
    theme: theme().terminal,
    cursorBlink: true,
    macOptionIsMeta: true,
  });
}

export class ChatShell {
  readonly sessionId: string;
  /** When this shell started; older blocks come from the database. */
  readonly createdAt = Date.now();
  terminalId: TerminalId | null = null;
  readonly state = createSignal<ShellState>("starting");
  readonly live = createSignal<LiveBlock | null>(null);
  readonly finished = createSignal<FinishedBlock[]>([]);
  /** Raw view open (escape hatch / fallback). */
  readonly rawOpen = createSignal(false);

  private readonly bridge: Bridge;
  private readonly splitter = new StreamSplitter();
  private cols = 120;
  private rawLog: Uint8Array[] = [];
  private rawLogBytes = 0;
  private raw: { term: Terminal; fit: FitAddon; el: HTMLDivElement } | null = null;
  private pendingCommand: string | null = null;
  private seq = 0;
  private unmatched: FinishedBlock[] = [];
  private earlyMeta: Block[] = [];
  private integrationTimer: ReturnType<typeof setTimeout> | undefined;
  private disposed = false;

  constructor(sessionId: string, bridge: Bridge) {
    this.sessionId = sessionId;
    this.bridge = bridge;
  }

  async start(cwd: string | undefined): Promise<void> {
    this.integrationTimer = setTimeout(() => {
      if (this.state[0]() === "starting") this.degrade();
    }, INTEGRATION_TIMEOUT_MS);
    try {
      this.terminalId = await this.bridge.openTerminal(
        { cwd, chatSessionId: this.sessionId, cols: this.cols, rows: BLOCK_ROWS },
        (bytes) => this.onBytes(bytes),
      );
      if (this.disposed) void this.bridge.closeTerminal(this.terminalId);
    } catch {
      this.state[1]("exited");
    }
  }

  /** Run a command typed in the input box. False if the shell isn't at a prompt. */
  run(command: string): boolean {
    const st = this.state[0]();
    if (!this.terminalId || (st !== "idle" && st !== "degraded")) return false;
    const first = command.trim().split(/\s+/)[0] ?? "";
    if (INLINE_TUIS.has(first) || st === "degraded") this.rawOpen[1](true);
    this.pendingCommand = command;
    void this.bridge.writeTerminal(this.terminalId, `${command}\r`);
    return true;
  }

  /** Keystrokes from a focused live block or the raw view. */
  send(data: string): void {
    if (this.terminalId) void this.bridge.writeTerminal(this.terminalId, data);
  }

  interrupt(): void {
    this.send("\x03");
  }

  private onBytes(bytes: Uint8Array): void {
    recordBytes(bytes.length);
    this.rawLog.push(bytes);
    this.rawLogBytes += bytes.length;
    while (this.rawLogBytes > RAW_LOG_CAP && this.rawLog.length > 1) {
      this.rawLogBytes -= this.rawLog.shift()?.length ?? 0;
    }
    this.raw?.term.write(bytes);
    if (this.state[0]() === "degraded") return;
    for (const ev of this.splitter.feed(bytes)) {
      if (ev.kind === "data") {
        const live = this.live[0]();
        if (live && this.state[0]() === "running") live.term.write(ev.bytes, () => this.grow(live));
      } else if (ev.kind === "alt") {
        const live = this.live[0]();
        if (live) this.setAlt(live, ev.on);
      } else if (ev.mark === "A") {
        clearTimeout(this.integrationTimer);
        if (this.state[0]() === "running") this.finish(null);
        this.state[1]("idle");
      } else if (ev.mark === "C") {
        this.begin();
      } else if (ev.mark === "D") {
        if (this.state[0]() === "running") this.finish(ev.exit ?? null);
        this.state[1]("idle");
      }
    }
  }

  private altSig = new WeakMap<Terminal, (v: boolean) => void>();
  private rowHSig = new WeakMap<Terminal, (v: number) => void>();
  private linesSig = new WeakMap<Terminal, (v: number) => void>();

  private begin(): void {
    const term = newTerm(BLOCK_ROWS, this.cols);
    const fit = new FitAddon();
    const serialize = new SerializeAddon();
    term.loadAddon(fit);
    term.loadAddon(serialize);
    term.loadAddon(new Unicode11Addon());
    term.unicode.activeVersion = "11";
    term.loadAddon(new WebLinksAddon((_e, uri) => void this.bridge.openUrl(uri)));
    term.onData((d) => this.send(d));
    const el = document.createElement("div");
    el.className = "tf-live-term";
    const [lines, setLines] = createSignal(1);
    const [alt, setAlt] = createSignal(false);
    const [rowH, setRowH] = createSignal(17);
    this.rowHSig.set(term, setRowH);
    this.altSig.set(term, setAlt);
    this.linesSig.set(term, setLines);
    const block: LiveBlock = {
      command: this.pendingCommand ?? "",
      startedAt: Date.now(),
      term,
      el,
      fit,
      serialize,
      lines,
      alt,
      rowH,
    };
    this.pendingCommand = null;
    this.state[1]("running");
    this.live[1](block);
  }

  /** Mount the live block's xterm (called by the timeline when the element is in the DOM). */
  mountLive(slot: HTMLElement): void {
    const live = this.live[0]();
    if (!live) return;
    slot.appendChild(live.el);
    if (!live.term.element) {
      live.term.open(live.el);
      const screen = live.el.querySelector(".xterm-screen");
      if (screen && live.term.rows)
        this.rowHSig.get(live.term)?.(screen.clientHeight / live.term.rows || 17);
      this.measure(live);
    }
    live.term.focus();
  }

  private measure(live: LiveBlock): void {
    const dims = live.fit.proposeDimensions();
    if (dims && dims.cols > 10 && dims.cols !== this.cols) {
      this.cols = dims.cols;
      live.term.resize(this.cols, live.alt() ? live.term.rows : BLOCK_ROWS);
      if (this.terminalId)
        void this.bridge.resizeTerminal(this.terminalId, this.cols, live.term.rows);
    }
  }

  private grow(live: LiveBlock): void {
    const b = live.term.buffer.active;
    // Used lines = last non-empty row below the cursor, at least the cursor row.
    let used = b.baseY + b.cursorY + 1;
    while (used > 1) {
      const line = b.getLine(used - 1)?.translateToString(true) ?? "";
      if (line.trim()) break;
      used--;
    }
    this.linesSig.get(live.term)?.(Math.max(1, used));
  }

  private setAlt(live: LiveBlock, on: boolean): void {
    this.altSig.get(live.term)?.(on);
    // Give the program the whole pane while it's full-screen, then go back to block rows.
    queueMicrotask(() => {
      if (on) live.fit.fit();
      else live.term.resize(this.cols, BLOCK_ROWS);
      if (this.terminalId)
        void this.bridge.resizeTerminal(this.terminalId, live.term.cols, live.term.rows);
      live.term.focus();
    });
  }

  /** Re-fit after the window or pane changed size. */
  relayout(): void {
    const live = this.live[0]();
    if (live?.term.element) {
      if (live.alt()) {
        live.fit.fit();
        if (this.terminalId)
          void this.bridge.resizeTerminal(this.terminalId, live.term.cols, live.term.rows);
      } else this.measure(live);
    }
    if (this.raw && this.rawOpen[0]()) this.fitRaw();
  }

  private finish(exit: number | null): void {
    const live = this.live[0]();
    if (!live) return;
    let html = "";
    try {
      html = live.serialize.serializeAsHTML({ includeGlobalBackground: false });
    } catch {
      html = "";
    }
    const block: FinishedBlock = {
      key: `${this.sessionId}:${this.seq++}`,
      command: live.command,
      exit,
      startedAt: live.startedAt,
      finishedAt: Date.now(),
      html,
    };
    const meta = this.earlyMeta.shift();
    if (meta) block.meta = meta;
    else this.unmatched.push(block);
    this.finished[1]((f) => [...f, block]);
    this.live[1](null);
    live.term.dispose();
    live.el.remove();
    // Programs may have changed the size while full-screen; restore it for the prompt.
    if (this.terminalId) void this.bridge.resizeTerminal(this.terminalId, this.cols, BLOCK_ROWS);
  }

  private degrade(): void {
    clearTimeout(this.integrationTimer);
    this.state[1]("degraded");
    this.rawOpen[1](true);
  }

  /** Core events routed here by terminal id. */
  handle(ev: AppEvent): void {
    switch (ev.type) {
      case "block_finished": {
        const local = this.unmatched.shift();
        if (local) {
          local.meta = ev.block;
          if (!local.command) local.command = ev.block.command;
          this.finished[1]((f) => [...f]);
        } else this.earlyMeta.push(ev.block);
        break;
      }
      case "blocks_degraded":
        this.degrade();
        break;
      case "terminal_exited":
        this.state[1]("exited");
        break;
    }
  }

  // ---- raw terminal (escape hatch, fallback) ----

  mountRaw(slot: HTMLElement): void {
    if (!this.raw) {
      const term = newTerm(BLOCK_ROWS, this.cols);
      const fit = new FitAddon();
      term.loadAddon(fit);
      term.loadAddon(new Unicode11Addon());
      term.unicode.activeVersion = "11";
      term.loadAddon(new WebLinksAddon((_e, uri) => void this.bridge.openUrl(uri)));
      term.onData((d) => this.send(d));
      const el = document.createElement("div");
      el.className = "tf-raw-term";
      this.raw = { term, fit, el };
      for (const chunk of this.rawLog) term.write(chunk);
    }
    slot.appendChild(this.raw.el);
    if (!this.raw.term.element) this.raw.term.open(this.raw.el);
    queueMicrotask(() => {
      this.fitRaw();
      this.raw?.term.focus();
    });
  }

  private fitRaw(): void {
    if (!this.raw) return;
    this.raw.fit.fit();
    if (this.terminalId)
      void this.bridge.resizeTerminal(this.terminalId, this.raw.term.cols, this.raw.term.rows);
  }

  closeRaw(): void {
    this.rawOpen[1](false);
    this.raw?.el.remove();
    if (this.terminalId) void this.bridge.resizeTerminal(this.terminalId, this.cols, BLOCK_ROWS);
  }

  applyTheme(): void {
    for (const t of [this.live[0]()?.term, this.raw?.term]) {
      if (t) {
        t.options.theme = theme().terminal;
        t.options.fontFamily = fontFamily();
      }
    }
  }

  dispose(): void {
    this.disposed = true;
    clearTimeout(this.integrationTimer);
    this.live[0]()?.term.dispose();
    this.raw?.term.dispose();
    if (this.terminalId) void this.bridge.closeTerminal(this.terminalId);
  }
}

// ---- registry: shells outlive the components showing them ----

const shells = new Map<string, ChatShell>();

export function shellFor(sessionId: string, bridge: Bridge, cwd: string | undefined): ChatShell {
  let s = shells.get(sessionId);
  if (!s) {
    s = new ChatShell(sessionId, bridge);
    shells.set(sessionId, s);
    void s.start(cwd);
  }
  return s;
}

export const existingShell = (sessionId: string): ChatShell | undefined => shells.get(sessionId);

export function routeShellEvent(ev: AppEvent): boolean {
  const tid = "block" in ev ? ev.block.terminal_id : "terminal_id" in ev ? ev.terminal_id : null;
  if (!tid) return false;
  for (const s of shells.values()) {
    if (s.terminalId === tid) {
      s.handle(ev);
      return true;
    }
  }
  return false;
}

export function disposeShell(sessionId: string): void {
  shells.get(sessionId)?.dispose();
  shells.delete(sessionId);
}

export function disposeAllShells(): void {
  for (const id of [...shells.keys()]) disposeShell(id);
}

export const allShells = (): ChatShell[] => [...shells.values()];
