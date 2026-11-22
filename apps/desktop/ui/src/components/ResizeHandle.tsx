// Drag handle for panel edges. Reports the pointer delta from the drag start; the caller
// clamps. Arrow keys nudge by 16px so it works from the keyboard too.
export function ResizeHandle(props: {
  axis: "x" | "y";
  label: string;
  /** Size at drag start. */
  size: () => number;
  /** +1 when moving right/down grows the panel, -1 when it shrinks it. */
  dir: 1 | -1;
  onResize: (size: number) => void;
}) {
  const start = (e: PointerEvent) => {
    e.preventDefault();
    const el = e.currentTarget as HTMLElement;
    el.setPointerCapture(e.pointerId);
    const origin = props.axis === "x" ? e.clientX : e.clientY;
    const base = props.size();
    const move = (m: PointerEvent) =>
      props.onResize(base + props.dir * ((props.axis === "x" ? m.clientX : m.clientY) - origin));
    const up = () => {
      el.removeEventListener("pointermove", move);
      el.removeEventListener("pointerup", up);
    };
    el.addEventListener("pointermove", move);
    el.addEventListener("pointerup", up);
  };
  const key = (e: KeyboardEvent) => {
    const grow = props.axis === "x" ? "ArrowRight" : "ArrowDown";
    const shrink = props.axis === "x" ? "ArrowLeft" : "ArrowUp";
    if (e.key !== grow && e.key !== shrink) return;
    e.preventDefault();
    props.onResize(props.size() + props.dir * (e.key === grow ? 16 : -16));
  };
  return (
    // biome-ignore lint/a11y/useSemanticElements: a focusable separator is the ARIA pattern for splitters.
    <div
      class="tf-resize"
      data-axis={props.axis}
      role="separator"
      tabIndex={0}
      aria-orientation={props.axis === "x" ? "vertical" : "horizontal"}
      aria-label={props.label}
      aria-valuenow={Math.round(props.size())}
      onPointerDown={start}
      onKeyDown={key}
    />
  );
}
