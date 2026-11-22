// Minimal stroke icons (24x24 grid). Decorative: buttons carry their own aria-label.
import type { JSX } from "solid-js";

const paths: Record<string, string> = {
  copy: "M9 9h10v10H9z M5 15V5h10",
  output: "M5 4h14v16H5z M8 9h8 M8 13h8 M8 17h5",
  markdown: "M4 6h16v12H4z M7 15V9l2.5 3L12 9v6 M15 9v6 M15 15l2-2 M15 15l-2-2",
  attach: "M16 7l-7.5 7.5a2 2 0 0 0 3 3L19 10a4 4 0 0 0-6-6l-7.5 7.5a6 6 0 0 0 9 9L20 15",
  rerun: "M4 12a8 8 0 1 0 2.5-5.8 M4 4v4h4",
  chevronDown: "M6 9l6 6 6-6",
  chevronRight: "M9 6l6 6-6 6",
  plus: "M12 5v14 M5 12h14",
  close: "M6 6l12 12 M18 6L6 18",
  warn: "M12 3l9 16H3z M12 10v4 M12 17v.5",
  search: "M11 4a7 7 0 1 0 0 14 7 7 0 0 0 0-14z M20 20l-4-4",
  up: "M6 15l6-6 6 6",
  down: "M6 9l6 6 6-6",
};

export function Icon(props: { name: keyof typeof paths | string; size?: number }): JSX.Element {
  return (
    <svg
      width={props.size ?? 14}
      height={props.size ?? 14}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      stroke-width="2"
      stroke-linecap="round"
      stroke-linejoin="round"
      aria-hidden="true"
    >
      <path d={paths[props.name] ?? ""} />
    </svg>
  );
}
