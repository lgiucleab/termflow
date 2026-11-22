import { expect, test } from "vitest";
import { type SplitEvent, StreamSplitter } from "./splitter";

const enc = new TextEncoder();
const dec = new TextDecoder();

const SAMPLE = enc.encode(
  "\x1b]6973;cwd=/w\x07\x1b]133;A\x07\x1b[1m$\x1b[0m \x1b]133;B\x07cargo test\r\n\x1b]133;C\x07" +
    "\x1b[32mok\x1b[0m\r\n\x1b]0;title\x07\x1b[?1049hTUI\x1b[?1049l done\r\n\x1b]133;D;101\x1b\\\x1b]133;A\x07",
);

/** Events with adjacent data merged and decoded, so chunking differences disappear. */
function normalize(events: SplitEvent[]): (string | object)[] {
  const out: (string | object)[] = [];
  for (const e of events) {
    if (e.kind === "data") {
      const s = dec.decode(e.bytes);
      const last = out[out.length - 1];
      if (typeof last === "string") out[out.length - 1] = last + s;
      else out.push(s);
    } else out.push(e);
  }
  return out;
}

test("marks become events, metadata is dropped, other escapes pass through", () => {
  expect(normalize(new StreamSplitter().feed(SAMPLE))).toEqual([
    { kind: "mark", mark: "A" },
    "\x1b[1m$\x1b[0m ",
    { kind: "mark", mark: "B" },
    "cargo test\r\n",
    { kind: "mark", mark: "C" },
    "\x1b[32mok\x1b[0m\r\n\x1b]0;title\x07\x1b[?1049h",
    { kind: "alt", on: true },
    "TUI\x1b[?1049l",
    { kind: "alt", on: false },
    " done\r\n",
    { kind: "mark", mark: "D", exit: 101 },
    { kind: "mark", mark: "A" },
  ]);
});

test("every chunking gives the same events", () => {
  const whole = normalize(new StreamSplitter().feed(SAMPLE));
  for (let i = 1; i < SAMPLE.length; i++) {
    for (const j of [i + 1, i + 7, SAMPLE.length]) {
      const s = new StreamSplitter();
      const ev = [
        ...s.feed(SAMPLE.slice(0, i)),
        ...s.feed(SAMPLE.slice(i, j)),
        ...s.feed(SAMPLE.slice(j)),
      ];
      expect(normalize(ev)).toEqual(whole);
    }
  }
});

test("garbage never throws and unterminated OSC eventually flushes", () => {
  const s = new StreamSplitter();
  const junk = new Uint8Array(20000).map((_, i) => (i * 7919) % 256);
  expect(() => s.feed(junk)).not.toThrow();
  const long = enc.encode(`\x1b]2;${"x".repeat(9000)}`);
  const ev = new StreamSplitter().feed(long);
  expect(ev.some((e) => e.kind === "data")).toBe(true);
});
