// One terminal: xterm.js view + PTY session + block decorations. Framework-agnostic on purpose
// (see TerminalView) so the renderer can be swapped for ghostty-web later.
import { FitAddon } from "@xterm/addon-fit";
import { SearchAddon } from "@xterm/addon-search";
import { SerializeAddon } from "@xterm/addon-serialize";
import { Unicode11Addon } from "@xterm/addon-unicode11";
import { WebLinksAddon } from "@xterm/addon-web-links";
import { WebglAddon } from "@xterm/addon-webgl";
import { type IDecoration, type IMarker, Terminal } from "@xterm/xterm";
import { createSignal } from "solid-js";
import { render } from "solid-js/web";
import type { AppEvent, Block, Bridge, TerminalId } from "../../bridge";
import { t } from "../../i18n";
import { isMac } from "../../keybindings";
import { attach as attachToAgent } from "../../stores/agent";
import { project } from "../../stores/project";
import { config, fontFamily, theme } from "../../stores/settings";
import { showToast } from "../../stores/toast";
import { BlockHeader } from "./BlockHeader";
import { blockMarkdown } from "./markdown";
import { recordBytes } from "./perf";
import { takeEarlyEvents } from "./registry";

/** Renderer-neutral surface the rest of the UI relies on. */
export interface TerminalView {
  readonly element: HTMLElement;
  focus(): void;
  fit(): void;
  dispose(): void;
}

/** Pause the PTY when this much output is queued in xterm, resume below LOW. */
const HIGH_WATER = 1024 * 1024;
const LOW_WATER = 128 * 1024;

interface Marks {
  prompt?: IMarker;
  output: IMarker;
  end?: IMarker;
}

export interface BlockUi {
  block: Block;
  collapsed: () => boolean;
  toggle: () => void;
}

export class TerminalController implements TerminalView {
  readonly element = document.createElement("div");
  readonly paneId: string;
  terminalId: TerminalId | null = null;

  private readonly bridge: Bridge;
  private readonly opts: {
    shellId?: string;
    agentId?: string;
    onExitClose: (paneId: string) => void;
  };
  private readonly term: Terminal;
  private readonly fitAddon = new FitAddon();
  private readonly search = new SearchAddon();
  private opened = false;
  private disposed = false;
  private inputQueue: string[] = [];
  private queued = 0;
  private paused = false;
  private resizeTimer: ReturnType<typeof setTimeout> | undefined;
  private lastSize = { cols: 0, rows: 0 };

  // Block marks recorded as xterm parses OSC 133; matched to core blocks by `seq`.
  private promptMarker: IMarker | undefined;
  private seq = 0;
  private marks = new Map<number, Marks>();
  private decorations: IDecoration[] = [];
  /** Core says a block finished, but xterm hasn't parsed its `D` mark yet (output is batched). */
  private finishedEarly = new Map<number, Block>();

  readonly degraded = createSignal(false);
  readonly cwd = createSignal<string | null>(null);
  readonly exited = createSignal<number | null | undefined>(undefined);
  readonly lastBlock = createSignal<Block | null>(null);
  readonly running = createSignal(false);
  readonly pendingPaste = createSignal<string | null>(null);
  readonly findOpen = createSignal(false);

  constructor(
    paneId: string,
    bridge: Bridge,
    opts: { shellId?: string; agentId?: string; onExitClose: (paneId: string) => void },
  ) {
    this.paneId = paneId;
    this.bridge = bridge;
    this.opts = opts;
    this.element.className = "tf-terminal";
    const cfg = config().terminal;
    this.term = new Terminal({
      allowProposedApi: true, // unicode11 and decorations
      fontFamily: fontFamily(),
      fontSize: cfg.font_size,
      scrollback: cfg.scrollback,
      theme: theme().terminal,
      cursorBlink: true,
      macOptionIsMeta: true,
      drawBoldTextInBrightColors: false,
    });
  }

  get agentId(): string | undefined {
    return this.opts.agentId;
  }

  /** Mount into `slot`. The first call opens xterm (it must be in the DOM to measure) and the PTY. */
  attachTo(slot: HTMLElement): void {
    slot.appendChild(this.element);
    if (this.opened) {
      this.fit();
      return;
    }
    this.opened = true;
    void this.open();
  }

  detach(): void {
    this.element.remove();
  }

  private async open(): Promise<void> {
    const term = this.term;
    try {
      await document.fonts.load(`${config().terminal.font_size}px ${fontFamily()}`);
    } catch {
      // Font loading API missing or font unavailable: xterm falls back to the next family.
    }
    term.open(this.element);
    term.loadAddon(this.fitAddon);
    term.loadAddon(this.search);
    term.loadAddon(new SerializeAddon());
    term.loadAddon(new Unicode11Addon());
    term.unicode.activeVersion = "11";
    term.loadAddon(new WebLinksAddon((_e, uri) => void this.bridge.openUrl(uri)));
    try {
      const webgl = new WebglAddon();
      webgl.onContextLoss(() => webgl.dispose()); // falls back to the DOM renderer
      term.loadAddon(webgl);
    } catch {
      // No WebGL: xterm's DOM renderer is used.
    }
    if (config().terminal.ligatures) {
      try {
        const { LigaturesAddon } = await import("@xterm/addon-ligatures");
        term.loadAddon(new LigaturesAddon());
      } catch {
        // Ligatures need local font access, which some WebViews don't offer.
      }
    }
    this.installBlockMarks();
    this.installInput();
    this.fitAddon.fit();
    this.lastSize = { cols: term.cols, rows: term.rows };

    try {
      const id = await this.bridge.openTerminal(
        {
          shellId: this.opts.shellId,
          agentId: this.opts.agentId,
          cwd: project()?.path,
          cols: term.cols,
          rows: term.rows,
        },
        (bytes) => this.onOutput(bytes),
      );
      this.terminalId = id;
      if (this.disposed) {
        void this.bridge.closeTerminal(id);
        return;
      }
      for (const ev of takeEarlyEvents(id)) this.handle(ev);
      for (const d of this.inputQueue) void this.bridge.writeTerminal(id, d);
      this.inputQueue = [];
      if (term.cols !== this.lastSize.cols || term.rows !== this.lastSize.rows) this.fit();
    } catch (e) {
      term.write(`\r\n\x1b[31m${t("terminal.openFailed")}: ${String(e)}\x1b[0m\r\n`);
    }
  }

  private onOutput(bytes: Uint8Array): void {
    recordBytes(bytes.length);
    this.queued += bytes.length;
    this.term.write(bytes, () => {
      this.queued -= bytes.length;
      if (this.paused && this.queued < LOW_WATER && this.terminalId) {
        this.paused = false;
        void this.bridge.setPaused(this.terminalId, false);
      }
    });
    if (!this.paused && this.queued > HIGH_WATER && this.terminalId) {
      this.paused = true;
      void this.bridge.setPaused(this.terminalId, true);
    }
  }

  private send(data: string): void {
    if (this.exited[0]() !== undefined) {
      if (data === "\r") this.opts.onExitClose(this.paneId);
      return;
    }
    if (this.terminalId) void this.bridge.writeTerminal(this.terminalId, data);
    else this.inputQueue.push(data);
  }

  private installInput(): void {
    const term = this.term;
    term.onData((d) => this.send(d));
    const mac = isMac();
    term.attachCustomKeyEventHandler((e) => {
      if (e.type !== "keydown") return true;
      // Windows/Linux: Ctrl+C copies when there is a selection (else it's SIGINT); Ctrl+V pastes.
      if (!mac && e.ctrlKey && !e.shiftKey && !e.altKey) {
        if (e.key === "c" && term.hasSelection()) {
          void navigator.clipboard.writeText(term.getSelection());
          term.clearSelection();
          return false;
        }
        if (e.key === "v") return false; // let the browser fire `paste` (handled below)
      }
      return true;
    });
    term.textarea?.addEventListener(
      "paste",
      (e) => {
        const text = e.clipboardData?.getData("text/plain") ?? "";
        e.preventDefault();
        e.stopImmediatePropagation();
        this.paste(text);
      },
      { capture: true },
    );
    term.onSelectionChange(() => {
      if (config().terminal.copy_on_select && term.hasSelection()) {
        void navigator.clipboard.writeText(term.getSelection());
      }
    });
  }

  /** Paste with the multi-line warning; xterm applies bracketed paste when the app asked for it. */
  paste(text: string): void {
    if (!text) return;
    if (config().terminal.warn_multiline_paste && /\r|\n/.test(text.replace(/[\r\n]+$/, ""))) {
      this.pendingPaste[1](text);
      return;
    }
    this.term.paste(text);
  }

  confirmPaste(ok: boolean): void {
    const text = this.pendingPaste[0]();
    this.pendingPaste[1](null);
    if (ok && text) this.term.paste(text);
    this.focus();
  }

  private installBlockMarks(): void {
    this.term.parser.registerOscHandler(133, (data) => {
      if (this.degraded[0]()) return true;
      switch (data[0]) {
        case "A":
          this.promptMarker = this.term.registerMarker(0);
          break;
        case "C": {
          const output = this.term.registerMarker(0);
          if (output) this.marks.set(this.seq, { prompt: this.promptMarker, output });
          this.seq += 1;
          break;
        }
        case "D": {
          const seq = this.seq - 1;
          const m = this.marks.get(seq);
          if (m && !m.end) m.end = this.term.registerMarker(0);
          const block = this.finishedEarly.get(seq);
          if (m && block) {
            this.finishedEarly.delete(seq);
            queueMicrotask(() => this.addHeader(block, m));
          }
          break;
        }
      }
      return true;
    });
  }

  // ---- core events (routed here by terminal id) ----

  handle(ev: AppEvent): void {
    switch (ev.type) {
      case "block_started":
        this.onBlockStarted(ev.block);
        break;
      case "block_finished":
        this.onBlockFinished(ev.block);
        break;
      case "blocks_degraded":
        this.onDegraded();
        break;
      case "terminal_exited":
        this.onExited(ev.exit_code);
        break;
    }
  }

  onBlockStarted(block: Block): void {
    this.running[1](true);
    if (block.cwd) this.cwd[1](block.cwd);
  }

  onBlockFinished(block: Block): void {
    this.running[1](false);
    this.lastBlock[1](block);
    if (block.cwd) this.cwd[1](block.cwd);
    const marks = this.marks.get(block.seq);
    if (!marks?.end) {
      this.finishedEarly.set(block.seq, block); // added when xterm reaches the D mark
      return;
    }
    this.addHeader(block, marks);
  }

  onDegraded(): void {
    this.degraded[1](true);
  }

  onExited(code: number | null): void {
    this.exited[1](code);
    this.term.write(`\r\n\x1b[2m${t("terminal.exited", { code: String(code ?? "?") })}\x1b[0m\r\n`);
  }

  private addHeader(block: Block, marks: Marks): void {
    const term = this.term;
    const prompt = marks.prompt;
    if (!prompt || prompt.isDisposed) return;
    const deco = term.registerDecoration({
      marker: prompt,
      width: term.cols,
      height: 1,
      layer: "top",
    });
    if (!deco) return;
    this.decorations.push(deco);
    const [collapsed, setCollapsed] = createSignal(false);
    let cover: IDecoration | undefined;
    const toggle = () => {
      if (cover) {
        cover.dispose();
        cover = undefined;
        setCollapsed(false);
        return;
      }
      const lines = marks.end ? marks.end.line - marks.output.line : 0;
      if (lines < 1) return;
      // xterm can't remove lines from its buffer, so "collapsed" covers the output in place.
      cover = term.registerDecoration({
        marker: marks.output,
        width: term.cols,
        height: lines,
        layer: "top",
      });
      cover?.onRender((el) => {
        el.style.width = "100%";
        if (el.dataset.mounted) return;
        el.dataset.mounted = "1";
        el.className = "tf-block-cover";
        el.textContent = t("blocks.collapsedLines", { n: String(lines) });
        el.onclick = toggle;
      });
      setCollapsed(true);
    };
    let unmount: (() => void) | undefined;
    deco.onRender((el) => {
      el.style.width = "100%";
      if (el.dataset.mounted) return;
      el.dataset.mounted = "1";
      el.classList.add("tf-block-header");
      unmount = render(
        () => BlockHeader({ ui: { block, collapsed, toggle }, controller: this }),
        el,
      );
    });
    deco.onDispose(() => {
      unmount?.();
      cover?.dispose();
    });
  }

  // ---- block actions ----

  async blockText(block: Block): Promise<string> {
    return (await this.bridge.blockOutput(block.id))?.output ?? "";
  }

  async copy(text: string, what: string): Promise<void> {
    await navigator.clipboard.writeText(text);
    showToast(t("toast.copied", { what }));
  }

  async copyMarkdown(block: Block): Promise<void> {
    await this.copy(blockMarkdown(block, await this.blockText(block)), t("blocks.markdown"));
  }

  async attach(block: Block): Promise<void> {
    attachToAgent({ block, markdown: blockMarkdown(block, await this.blockText(block)) });
    showToast(t("toast.attached", { cmd: block.command }));
  }

  rerun(block: Block): void {
    this.send(`${block.command}\r`);
    this.focus();
  }

  // ---- find ----

  findNext(q: string, back = false): boolean {
    const opts = { caseSensitive: false, decorations: undefined };
    return back ? this.search.findPrevious(q, opts) : this.search.findNext(q, opts);
  }

  clearFind(): void {
    this.search.clearDecorations();
  }

  // ---- view ----

  focus(): void {
    this.term.focus();
  }

  /** Debounced fit; tells the PTY only when the grid size actually changed. */
  fit(): void {
    clearTimeout(this.resizeTimer);
    this.resizeTimer = setTimeout(() => {
      if (!this.opened || !this.element.isConnected || this.element.clientWidth === 0) return;
      this.fitAddon.fit();
      const { cols, rows } = this.term;
      if (cols === this.lastSize.cols && rows === this.lastSize.rows) return;
      this.lastSize = { cols, rows };
      if (this.terminalId) void this.bridge.resizeTerminal(this.terminalId, cols, rows);
    }, 50);
  }

  applySettings(): void {
    const cfg = config().terminal;
    this.term.options.theme = theme().terminal;
    this.term.options.fontFamily = fontFamily();
    this.term.options.fontSize = cfg.font_size;
    this.fit();
  }

  dispose(): void {
    this.disposed = true;
    clearTimeout(this.resizeTimer);
    for (const d of this.decorations) d.dispose();
    if (this.terminalId) void this.bridge.closeTerminal(this.terminalId);
    this.term.dispose();
    this.element.remove();
  }
}
