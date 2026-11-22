// Splits a shell's PTY byte stream for the unified timeline: shell-integration marks
// (OSC 133 A/B/C/D) become events and are removed; OSC 6973 / 7 metadata is dropped (the core
// already parsed it); everything else passes through as data. Alternate-screen switches
// (`CSI ? 1049 h/l`, also 47/1047) are reported after their bytes. Chunk boundaries are safe.

export type Mark = "A" | "B" | "C" | "D";
export type SplitEvent =
  | { kind: "data"; bytes: Uint8Array }
  | { kind: "mark"; mark: Mark; exit?: number }
  | { kind: "alt"; on: boolean };

const ESC = 0x1b;
const BEL = 0x07;
const MAX_OSC = 8192;

enum S {
  Normal,
  Esc,
  Osc,
  OscEsc,
  Csi,
}

const dec = new TextDecoder();

export class StreamSplitter {
  private s = S.Normal;
  private out: number[] = [];
  private osc: number[] = [];
  private csi: number[] = [];
  private events: SplitEvent[] = [];

  feed(bytes: Uint8Array): SplitEvent[] {
    for (const b of bytes) this.step(b);
    this.flush();
    const e = this.events;
    this.events = [];
    return e;
  }

  private flush(): void {
    if (this.out.length) {
      this.events.push({ kind: "data", bytes: Uint8Array.from(this.out) });
      this.out = [];
    }
  }

  private step(b: number): void {
    switch (this.s) {
      case S.Normal:
        if (b === ESC) this.s = S.Esc;
        else this.out.push(b);
        return;
      case S.Esc:
        if (b === 0x5d /* ] */) {
          this.s = S.Osc;
          this.osc = [];
        } else if (b === 0x5b /* [ */) {
          this.out.push(ESC, b);
          this.csi = [];
          this.s = S.Csi;
        } else {
          this.out.push(ESC, b);
          this.s = b === ESC ? S.Esc : S.Normal;
          if (b === ESC) this.out.pop();
        }
        return;
      case S.Osc:
        if (b === BEL) this.endOsc([BEL]);
        else if (b === ESC) this.s = S.OscEsc;
        else if (this.osc.length >= MAX_OSC) this.abortOsc([b]);
        else this.osc.push(b);
        return;
      case S.OscEsc:
        if (b === 0x5c /* \ */) this.endOsc([ESC, b]);
        else this.abortOsc([ESC, b]);
        return;
      case S.Csi:
        this.out.push(b);
        this.csi.push(b);
        if (b >= 0x40 && b <= 0x7e) {
          const seq = dec.decode(Uint8Array.from(this.csi));
          const m = /^\?(?:1049|1047|47)([hl])$/.exec(seq);
          if (m) {
            this.flush();
            this.events.push({ kind: "alt", on: m[1] === "h" });
          }
          this.s = S.Normal;
        }
        return;
    }
  }

  private endOsc(term: number[]): void {
    const text = dec.decode(Uint8Array.from(this.osc));
    this.s = S.Normal;
    if (text.startsWith("133;")) {
      const kind = text[4] as Mark;
      if (kind === "A" || kind === "B" || kind === "C" || kind === "D") {
        this.flush();
        const exit = kind === "D" ? Number.parseInt(text.split(";")[2] ?? "", 10) : Number.NaN;
        this.events.push(
          Number.isNaN(exit) ? { kind: "mark", mark: kind } : { kind: "mark", mark: kind, exit },
        );
      }
      return;
    }
    if (text.startsWith("6973;") || text.startsWith("7;")) return;
    // Other OSC (window title, hyperlinks…) belong to the program: pass through unchanged.
    this.out.push(ESC, 0x5d, ...this.osc, ...term);
  }

  private abortOsc(tail: number[]): void {
    this.out.push(ESC, 0x5d, ...this.osc, ...tail);
    this.osc = [];
    this.s = S.Normal;
  }
}
