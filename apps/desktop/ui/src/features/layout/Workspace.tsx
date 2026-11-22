import { createSignal, For, type JSX, Match, Show, Switch } from "solid-js";
import { Icon } from "../../components/Icon";
import { t } from "../../i18n";
import { AgentTab } from "../agent/AgentTab";
import { existingController } from "../terminal/registry";
import { TerminalPane } from "../terminal/TerminalPane";
import type { Layout, LayoutNode, Path, Tab } from "./model";

export interface WorkspaceProps {
  layout: Layout;
  onSelectTab: (id: string) => void;
  onCloseTab: (id: string) => void;
  onRenameTab: (id: string, title: string) => void;
  onFocusPane: (id: string) => void;
  onClosePane: (id: string) => void;
  onRatio: (tabId: string, path: Path, ratio: number) => void;
  onNewTab: () => void;
  onOpenPalette: () => void;
}

export function tabTitle(tab: Tab): string {
  if (tab.title) return tab.title;
  const find = (n: LayoutNode): LayoutNode | undefined =>
    n.type === "pane" ? (n.pane.id === tab.focused ? n : undefined) : (find(n.a) ?? find(n.b));
  const node = find(tab.root);
  if (node?.type !== "pane") return t("tabs.terminal");
  if (node.pane.kind.type === "agent") return t("tabs.agent");
  const c = existingController(node.pane.id);
  const cwd = c?.cwd[0]();
  const base = cwd ? (cwd.split(/[\\/]/).filter(Boolean).pop() ?? "/") : null;
  return node.pane.kind.agentId ?? base ?? t("tabs.terminal");
}

function TabButton(props: { tab: Tab; active: boolean; p: WorkspaceProps }) {
  const [editing, setEditing] = createSignal(false);
  return (
    <div class="tf-tab" data-active={props.active ? "1" : "0"}>
      <Show
        when={editing()}
        fallback={
          <button
            type="button"
            role="tab"
            aria-selected={props.active}
            class="tf-tab-label"
            onClick={() => props.p.onSelectTab(props.tab.id)}
            onDblClick={() => setEditing(true)}
            title={t("tabs.renameHint")}
          >
            {tabTitle(props.tab)}
          </button>
        }
      >
        <input
          class="tf-tab-input"
          value={props.tab.title ?? tabTitle(props.tab)}
          aria-label={t("tabs.rename")}
          ref={(el) => queueMicrotask(() => el.select())}
          onKeyDown={(e) => {
            if (e.key === "Enter") {
              props.p.onRenameTab(props.tab.id, e.currentTarget.value);
              setEditing(false);
            }
            if (e.key === "Escape") setEditing(false);
          }}
          onBlur={(e) => {
            props.p.onRenameTab(props.tab.id, e.currentTarget.value);
            setEditing(false);
          }}
        />
      </Show>
      <button
        type="button"
        class="tf-tab-close"
        aria-label={t("tabs.close")}
        onClick={() => props.p.onCloseTab(props.tab.id)}
      >
        <Icon name="close" size={11} />
      </button>
    </div>
  );
}

function SplitView(props: {
  node: Extract<LayoutNode, { type: "split" }>;
  tab: Tab;
  path: Path;
  p: WorkspaceProps;
}) {
  let box!: HTMLDivElement;
  const onDown = (e: PointerEvent) => {
    e.preventDefault();
    const handle = e.currentTarget as HTMLElement;
    handle.setPointerCapture(e.pointerId);
    const move = (ev: PointerEvent) => {
      const r = box.getBoundingClientRect();
      const ratio =
        props.node.dir === "row"
          ? (ev.clientX - r.left) / r.width
          : (ev.clientY - r.top) / r.height;
      props.p.onRatio(props.tab.id, props.path, ratio);
    };
    const up = () => {
      handle.removeEventListener("pointermove", move);
      handle.removeEventListener("pointerup", up);
    };
    handle.addEventListener("pointermove", move);
    handle.addEventListener("pointerup", up);
  };
  return (
    <div ref={box} class="tf-split" data-dir={props.node.dir}>
      <div class="tf-split-part" style={{ flex: `${props.node.ratio} 1 0` }}>
        <NodeView node={props.node.a} tab={props.tab} path={[...props.path, "a"]} p={props.p} />
      </div>
      {/* biome-ignore lint/a11y/useSemanticElements: an <hr> can't be a draggable flex child. */}
      <div
        class="tf-divider"
        role="separator"
        aria-orientation={props.node.dir === "row" ? "vertical" : "horizontal"}
        onPointerDown={onDown}
      />
      <div class="tf-split-part" style={{ flex: `${1 - props.node.ratio} 1 0` }}>
        <NodeView node={props.node.b} tab={props.tab} path={[...props.path, "b"]} p={props.p} />
      </div>
    </div>
  );
}

function NodeView(props: {
  node: LayoutNode;
  tab: Tab;
  path: Path;
  p: WorkspaceProps;
}): JSX.Element {
  return (
    <Switch>
      <Match when={props.node.type === "split" && props.node}>
        {(split) => <SplitView node={split()} tab={props.tab} path={props.path} p={props.p} />}
      </Match>
      <Match when={props.node.type === "pane" && props.node}>
        {(leaf) => {
          const pane = leaf().pane;
          const focused = () => props.tab.focused === pane.id;
          return pane.kind.type === "agent" ? (
            <AgentTab focused={focused()} onFocus={() => props.p.onFocusPane(pane.id)} />
          ) : (
            <TerminalPane
              pane={pane}
              focused={focused()}
              onFocus={() => props.p.onFocusPane(pane.id)}
              onExitClose={props.p.onClosePane}
            />
          );
        }}
      </Match>
    </Switch>
  );
}

export function Workspace(props: WorkspaceProps) {
  return (
    <div class="tf-workspace">
      <div class="tf-tabbar" role="tablist">
        <For each={props.layout.tabs}>
          {(tab) => <TabButton tab={tab} active={tab.id === props.layout.active} p={props} />}
        </For>
        <button
          type="button"
          class="tf-tab-new"
          aria-label={t("tabs.new")}
          onClick={props.onNewTab}
        >
          <Icon name="plus" size={13} />
        </button>
        <span class="tf-spacer" />
        <button type="button" class="tf-tab-palette" onClick={props.onOpenPalette}>
          {t("palette.open")}
        </button>
      </div>
      <For each={props.layout.tabs}>
        {(tab) => (
          <div class="tf-tabpanel" role="tabpanel" hidden={tab.id !== props.layout.active}>
            <NodeView node={tab.root} tab={tab} path={[]} p={props} />
          </div>
        )}
      </For>
    </div>
  );
}
