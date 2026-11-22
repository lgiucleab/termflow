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
  terminal: "M4 5h16v14H4z M8 10l3 2-3 2 M13 15h3",
  file: "M6 3h8l4 4v14H6z M14 3v4h4",
  edit: "M4 20h4L19 9l-4-4L4 16z M13 7l4 4",
  globe: "M12 3a9 9 0 1 0 0 18 9 9 0 0 0 0-18z M3 12h18 M12 3c3 3 3 15 0 18 M12 3c-3 3-3 15 0 18",
  list: "M9 6h11 M9 12h11 M9 18h11 M4 6h1 M4 12h1 M4 18h1",
  tool: "M14 6a4 4 0 0 0 5 5l-8 8a2 2 0 0 1-3-3l8-8a4 4 0 0 0-2-2z",
  sidebar: "M4 5h16v14H4z M9 5v14",
  panelRight: "M4 5h16v14H4z M15 5v14",
  settings:
    "M12 9a3 3 0 1 0 0 6 3 3 0 0 0 0-6z M12 2v3 M12 19v3 M4.9 4.9l2.1 2.1 M17 17l2.1 2.1 M2 12h3 M19 12h3 M4.9 19.1L7 17 M17 7l2.1-2.1",
  command: "M9 6a3 3 0 1 0-3 3h12a3 3 0 1 0-3-3v12a3 3 0 1 0 3-3H6a3 3 0 1 0 3 3z",
  arrowUp: "M12 19V5 M5 12l7-7 7 7",
  stop: "M7 7h10v10H7z",
  enter: "M20 5v7a3 3 0 0 1-3 3H5 M9 11l-4 4 4 4",
  folder: "M3 6h6l2 2h10v11H3z",
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
