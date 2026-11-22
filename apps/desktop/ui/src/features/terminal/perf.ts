// Dev-only frame-time monitor (BUILD_PROMPT §14.4): while output is flowing, record the longest
// gap between animation frames and report it to the core log every 2 s.
import type { Bridge } from "../../bridge";

let bytes = 0;
export const recordBytes = (n: number): void => {
  bytes += n;
};

export function startPerfMonitor(bridge: Bridge): void {
  let last = performance.now();
  let max = 0;
  let over50 = 0;
  let over100 = 0;
  let windowStart = last;
  let worst = 0;
  // Browsers pause animation frames for hidden windows; that pause is not a UI freeze.
  document.addEventListener("visibilitychange", () => {
    last = performance.now();
  });
  const frame = (now: number) => {
    const gap = now - last;
    last = now;
    if (bytes > 0 && !document.hidden) {
      max = Math.max(max, gap);
      if (gap > 50) over50++;
      if (gap > 100) over100++;
    }
    if (now - windowStart >= 2000) {
      if (bytes > 0) {
        worst = Math.max(worst, max);
        void bridge.perfReport(
          JSON.stringify({
            window_ms: Math.round(now - windowStart),
            bytes,
            max_frame_ms: Math.round(max),
            frames_over_50ms: over50,
            frames_over_100ms: over100,
            worst_so_far_ms: Math.round(worst),
          }),
        );
      }
      bytes = 0;
      max = 0;
      over50 = 0;
      over100 = 0;
      windowStart = now;
    }
    requestAnimationFrame(frame);
  };
  requestAnimationFrame(frame);
}
