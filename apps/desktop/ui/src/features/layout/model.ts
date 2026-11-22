// Tab / split layout as an immutable tree. Pure functions only (unit tested).

export type PaneKind =
  | { type: "terminal"; shellId?: string; agentId?: string }
  | { type: "agent"; sessionId?: string };

export interface Pane {
  id: string;
  kind: PaneKind;
}

export type Dir = "row" | "column";

export type LayoutNode =
  | { type: "pane"; pane: Pane }
  | { type: "split"; dir: Dir; ratio: number; a: LayoutNode; b: LayoutNode };

export interface Tab {
  id: string;
  title?: string;
  root: LayoutNode;
  focused: string;
}

export interface Layout {
  tabs: Tab[];
  active: string;
}

let counter = 0;
export const newId = (): string =>
  globalThis.crypto?.randomUUID?.() ?? `id-${Date.now()}-${++counter}`;

export function panes(node: LayoutNode): Pane[] {
  return node.type === "pane" ? [node.pane] : [...panes(node.a), ...panes(node.b)];
}

export function allPanes(layout: Layout): Pane[] {
  return layout.tabs.flatMap((t) => panes(t.root));
}

export function activeTab(layout: Layout): Tab | undefined {
  return layout.tabs.find((t) => t.id === layout.active) ?? layout.tabs[0];
}

function makeTab(kind: PaneKind): Tab {
  const pane = { id: newId(), kind };
  return { id: newId(), root: { type: "pane", pane }, focused: pane.id };
}

export function initialLayout(kind: PaneKind = { type: "terminal" }): Layout {
  const tab = makeTab(kind);
  return { tabs: [tab], active: tab.id };
}

export function newTab(layout: Layout, kind: PaneKind): Layout {
  const tab = makeTab(kind);
  return { tabs: [...layout.tabs, tab], active: tab.id };
}

function mapTab(layout: Layout, id: string, f: (t: Tab) => Tab): Layout {
  return { ...layout, tabs: layout.tabs.map((t) => (t.id === id ? f(t) : t)) };
}

function replacePane(node: LayoutNode, id: string, f: (n: LayoutNode) => LayoutNode): LayoutNode {
  if (node.type === "pane") return node.pane.id === id ? f(node) : node;
  return { ...node, a: replacePane(node.a, id, f), b: replacePane(node.b, id, f) };
}

/** Split the focused pane of the active tab; the new pane is a plain terminal with the same shell. */
export function splitFocused(layout: Layout, dir: Dir): { layout: Layout; pane?: Pane } {
  const tab = activeTab(layout);
  if (!tab) return { layout };
  const focused = panes(tab.root).find((p) => p.id === tab.focused);
  if (focused?.kind.type !== "terminal") return { layout };
  const pane: Pane = { id: newId(), kind: { type: "terminal", shellId: focused.kind.shellId } };
  const next = mapTab(layout, tab.id, (t) => ({
    ...t,
    focused: pane.id,
    root: replacePane(t.root, focused.id, (n) => ({
      type: "split",
      dir,
      ratio: 0.5,
      a: n,
      b: { type: "pane", pane },
    })),
  }));
  return { layout: next, pane };
}

function removePane(node: LayoutNode, id: string): LayoutNode | null {
  if (node.type === "pane") return node.pane.id === id ? null : node;
  const a = removePane(node.a, id);
  const b = removePane(node.b, id);
  if (!a) return b;
  if (!b) return a;
  return { ...node, a, b };
}

/** Remove a pane; empty tabs disappear. Never returns zero tabs: closing the last opens a fresh terminal. */
export function closePane(layout: Layout, id: string): Layout {
  const tabs: Tab[] = [];
  for (const t of layout.tabs) {
    const root = removePane(t.root, id);
    if (!root) continue;
    const remaining = panes(root);
    const focused = remaining.some((p) => p.id === t.focused)
      ? t.focused
      : (remaining[0]?.id ?? "");
    tabs.push({ ...t, root, focused });
  }
  if (tabs.length === 0) return initialLayout();
  const active = tabs.some((t) => t.id === layout.active)
    ? layout.active
    : (tabs[Math.max(0, layout.tabs.findIndex((t) => t.id === layout.active) - 1)]?.id ??
      tabs[0]?.id ??
      "");
  return { tabs, active };
}

export function closeTab(layout: Layout, tabId: string): { layout: Layout; closed: Pane[] } {
  const tab = layout.tabs.find((t) => t.id === tabId);
  if (!tab) return { layout, closed: [] };
  const closed = panes(tab.root);
  return { layout: closed.reduce((l, p) => closePane(l, p.id), layout), closed };
}

export type Path = ("a" | "b")[];

export function setRatio(layout: Layout, tabId: string, path: Path, ratio: number): Layout {
  const clamped = Math.min(0.9, Math.max(0.1, ratio));
  const walk = (n: LayoutNode, i: number): LayoutNode => {
    if (n.type !== "split") return n;
    if (i === path.length) return { ...n, ratio: clamped };
    const side = path[i];
    return side === "a" ? { ...n, a: walk(n.a, i + 1) } : { ...n, b: walk(n.b, i + 1) };
  };
  return mapTab(layout, tabId, (t) => ({ ...t, root: walk(t.root, 0) }));
}

export function focusPane(layout: Layout, paneId: string): Layout {
  const tab = layout.tabs.find((t) => panes(t.root).some((p) => p.id === paneId));
  if (!tab) return layout;
  return { ...mapTab(layout, tab.id, (t) => ({ ...t, focused: paneId })), active: tab.id };
}

/** Move focus to the next/previous pane within the active tab. */
export function cycleFocus(layout: Layout, delta: number): Layout {
  const tab = activeTab(layout);
  if (!tab) return layout;
  const ps = panes(tab.root);
  const i = ps.findIndex((p) => p.id === tab.focused);
  const next = ps[(i + delta + ps.length) % ps.length];
  return next ? focusPane(layout, next.id) : layout;
}

export function selectTab(layout: Layout, delta: number): Layout {
  const i = layout.tabs.findIndex((t) => t.id === layout.active);
  const next = layout.tabs[(i + delta + layout.tabs.length) % layout.tabs.length];
  return next ? { ...layout, active: next.id } : layout;
}

/** Remember which agent session an agent pane shows (restored on restart). */
export function setAgentSession(layout: Layout, paneId: string, sessionId: string): Layout {
  const walk = (n: LayoutNode): LayoutNode =>
    n.type === "split"
      ? { ...n, a: walk(n.a), b: walk(n.b) }
      : n.pane.id === paneId && n.pane.kind.type === "agent"
        ? { type: "pane", pane: { ...n.pane, kind: { type: "agent", sessionId } } }
        : n;
  return { ...layout, tabs: layout.tabs.map((t) => ({ ...t, root: walk(t.root) })) };
}

export function renameTab(layout: Layout, tabId: string, title: string): Layout {
  return mapTab(layout, tabId, (t) => ({ ...t, title: title.trim() || undefined }));
}

/** For restore: agent CLIs are not relaunched automatically; their panes come back as plain shells. */
export function forRestore(layout: Layout): Layout {
  const strip = (n: LayoutNode): LayoutNode =>
    n.type === "split"
      ? { ...n, a: strip(n.a), b: strip(n.b) }
      : n.pane.kind.type === "terminal"
        ? {
            type: "pane",
            pane: { ...n.pane, kind: { type: "terminal", shellId: n.pane.kind.shellId } },
          }
        : n;
  return { ...layout, tabs: layout.tabs.map((t) => ({ ...t, root: strip(t.root) })) };
}

function isNode(v: unknown): v is LayoutNode {
  if (typeof v !== "object" || v === null) return false;
  const n = v as Record<string, unknown>;
  if (n.type === "pane") {
    const p = n.pane as Record<string, unknown> | undefined;
    const k = p?.kind as Record<string, unknown> | undefined;
    return typeof p?.id === "string" && (k?.type === "terminal" || k?.type === "agent");
  }
  return (
    n.type === "split" &&
    (n.dir === "row" || n.dir === "column") &&
    typeof n.ratio === "number" &&
    isNode(n.a) &&
    isNode(n.b)
  );
}

/** Parse a saved layout; anything malformed gives `null` (caller starts fresh). */
export function parseLayout(json: string | null | undefined): Layout | null {
  if (!json) return null;
  try {
    const v = JSON.parse(json) as Partial<Layout>;
    if (!Array.isArray(v.tabs) || v.tabs.length === 0 || typeof v.active !== "string") return null;
    for (const t of v.tabs) {
      if (typeof t?.id !== "string" || !isNode(t.root)) return null;
    }
    return forRestore(v as Layout);
  } catch {
    return null;
  }
}
