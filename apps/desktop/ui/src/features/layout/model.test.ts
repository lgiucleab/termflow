import { describe, expect, test } from "vitest";
import {
  activeTab,
  closePane,
  closeTab,
  cycleFocus,
  initialLayout,
  newTab,
  panes,
  parseLayout,
  setRatio,
  splitFocused,
} from "./model";

describe("layout model", () => {
  test("split puts the new pane next to the focused one and focuses it", () => {
    const l0 = initialLayout({ type: "terminal", shellId: "zsh" });
    const { layout, pane } = splitFocused(l0, "row");
    const tab = activeTab(layout);
    expect(tab?.root.type).toBe("split");
    expect(panes(tab!.root).map((p) => p.id)).toEqual([activeTab(l0)!.focused, pane!.id]);
    expect(tab?.focused).toBe(pane!.id);
    expect(pane?.kind).toEqual({ type: "terminal", shellId: "zsh" });
  });

  test("closing a pane collapses its split and moves focus", () => {
    const l0 = initialLayout();
    const first = activeTab(l0)!.focused;
    const { layout, pane } = splitFocused(l0, "column");
    const l2 = closePane(layout, pane!.id);
    const tab = activeTab(l2)!;
    expect(tab.root).toEqual({ type: "pane", pane: { id: first, kind: { type: "terminal" } } });
    expect(tab.focused).toBe(first);
  });

  test("closing the last pane opens a fresh terminal tab", () => {
    const l0 = initialLayout();
    const l1 = closePane(l0, activeTab(l0)!.focused);
    expect(l1.tabs).toHaveLength(1);
    expect(activeTab(l1)!.focused).not.toBe(activeTab(l0)!.focused);
  });

  test("closing a tab activates its left neighbour", () => {
    const l = newTab(newTab(initialLayout(), { type: "terminal" }), { type: "agent" });
    const [t0, , t2] = l.tabs;
    const { layout, closed } = closeTab({ ...l, active: t2!.id }, t2!.id);
    expect(closed).toHaveLength(1);
    expect(layout.tabs).toHaveLength(2);
    expect(layout.active).toBe(l.tabs[1]!.id);
    expect(layout.tabs[0]!.id).toBe(t0!.id);
  });

  test("ratio is clamped and focus cycles", () => {
    const { layout } = splitFocused(initialLayout(), "row");
    const tab = activeTab(layout)!;
    const r = setRatio(layout, tab.id, [], 2);
    expect(activeTab(r)!.root.type === "split" && activeTab(r)!.root).toMatchObject({ ratio: 0.9 });
    const c = cycleFocus(layout, 1);
    expect(activeTab(c)!.focused).toBe(panes(tab.root)[0]!.id);
  });

  test("parse rejects garbage and strips agent launch on restore", () => {
    expect(parseLayout("nope")).toBeNull();
    expect(parseLayout('{"tabs":[],"active":"x"}')).toBeNull();
    const l = initialLayout({ type: "terminal", shellId: "bash", agentId: "claude" });
    const back = parseLayout(JSON.stringify(l))!;
    expect(panes(back.tabs[0]!.root)[0]!.kind).toEqual({ type: "terminal", shellId: "bash" });
  });
});
