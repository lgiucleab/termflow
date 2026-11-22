import { createMemo, createSignal, onCleanup, onMount, Show } from "solid-js";
import type { AgentCli, Bridge, Workspace as Project, ShellSpec } from "./bridge";
import { BridgeProvider } from "./bridge/context";
import { ChatView, dispatchToolEvent } from "./features/chat/ChatView";
import { Sidebar } from "./features/chat/Sidebar";
import { StartScreen } from "./features/chat/StartScreen";
import {
  activeTab,
  allPanes,
  closePane,
  closeTab,
  cycleFocus,
  focusPane,
  initialLayout,
  type Layout,
  newTab,
  type Pane,
  type PaneKind,
  panes,
  parseLayout,
  renameTab,
  selectTab,
  setRatio,
  splitFocused,
} from "./features/layout/model";
import { Workspace } from "./features/layout/Workspace";
import { Palette, type PaletteItem } from "./features/palette/Palette";
import { ProvidersDialog } from "./features/providers/ProvidersDialog";
import { blockMarkdown } from "./features/terminal/markdown";
import { startPerfMonitor } from "./features/terminal/perf";
import {
  allControllers,
  disposeController,
  existingController,
  routeEvent,
} from "./features/terminal/registry";
import { THEMES, themeById } from "./features/themes/themes";
import { t } from "./i18n";
import { type Action, actionFor, isMac, shortcutLabel } from "./keybindings";
import { attach } from "./stores/agent";
import { dispatchAgentEvent } from "./stores/agentEvents";
import {
  activeChat,
  chats,
  LAST_PROJECT_KEY,
  PANEL_KEY,
  panelHeight,
  panelOpen,
  project,
  refreshChats,
  setActiveChat,
  setPanelHeight,
  setPanelOpen,
  setProject,
} from "./stores/project";
import {
  credentials,
  LAST_MODEL_KEY,
  type ModelChoice,
  providersOpen,
  refreshCredentials,
  setProvidersOpen,
} from "./stores/providers";
import { setConfig, setTheme, theme } from "./stores/settings";
import { showToast, toast } from "./stores/toast";

const LAYOUT_KEY = "layout.v1";
const THEME_KEY = "theme";

export function App(props: { bridge: Bridge }) {
  const bridge = props.bridge;
  const [ready, setReady] = createSignal(false);
  const [layout, setLayoutRaw] = createSignal<Layout>(initialLayout());
  const [shells, setShells] = createSignal<ShellSpec[]>([]);
  const [agents, setAgents] = createSignal<AgentCli[]>([]);
  const [paletteItems, setPaletteItems] = createSignal<PaletteItem[] | null>(null);
  const mac = isMac();

  let saveTimer: ReturnType<typeof setTimeout> | undefined;
  const setLayout = (next: Layout) => {
    // Dispose shells whose panes disappeared.
    const keep = new Set(allPanes(next).map((p) => p.id));
    for (const p of allPanes(layout())) if (!keep.has(p.id)) disposeController(p.id);
    setLayoutRaw(next);
    clearTimeout(saveTimer);
    saveTimer = setTimeout(() => void bridge.settingSet(LAYOUT_KEY, JSON.stringify(next)), 500);
  };

  const focusedPane = createMemo((): Pane | undefined => {
    const tab = activeTab(layout());
    return tab ? panes(tab.root).find((p) => p.id === tab.focused) : undefined;
  });
  const focusedController = () => {
    const p = focusedPane();
    return p ? existingController(p.id) : undefined;
  };

  const open = (kind: PaneKind) => {
    setPanelOpen(true);
    setLayout(newTab(layout(), kind));
  };

  const savePanel = () =>
    void bridge.settingSet(PANEL_KEY, JSON.stringify({ open: panelOpen(), height: panelHeight() }));

  const togglePanel = () => {
    setPanelOpen((o) => !o);
    savePanel();
  };

  const newChat = async () => {
    const p = project();
    if (!p) return;
    const last = JSON.parse(
      (await bridge.settingGet(LAST_MODEL_KEY)) ?? "null",
    ) as ModelChoice | null;
    const valid = last && credentials().some((c) => c.id === last.credentialId);
    const s = await bridge.createAgentSession(
      p.id,
      valid ? last.credentialId : null,
      valid ? last.model : null,
    );
    if (valid && !last.reasoning)
      await bridge.setAgentModel(s.id, last.credentialId, last.model, false);
    await refreshChats(bridge);
    setActiveChat(s.id);
  };

  const openProject = async (w: Project) => {
    setProject(w);
    void bridge.settingSet(LAST_PROJECT_KEY, w.id);
    await refreshChats(bridge);
    const first = chats()[0];
    if (first) setActiveChat(first.id);
    else await newChat();
  };

  const switchProject = () => {
    setProject(null);
    setActiveChat(null);
  };

  const attachLastBlock = async () => {
    const c = focusedController();
    const b = c?.lastBlock[0]();
    if (c && b) return c.attach(b);
    const [recent] = await bridge.recentBlocks(1).catch(() => []);
    if (!recent) return showToast(t("toast.noBlock"));
    const out = await bridge.blockOutput(recent.id);
    attach({ block: recent, markdown: blockMarkdown(recent, out?.output ?? "") });
    showToast(t("toast.attached", { cmd: recent.command }));
  };

  const run = (a: Action) => {
    switch (a) {
      case "palette":
        void openPalette();
        break;
      case "newTab":
        open({ type: "terminal" });
        break;
      case "toggleTerminal":
        togglePanel();
        break;
      case "newChat":
        void newChat();
        break;
      case "closePane": {
        const p = focusedPane();
        if (p) setLayout(closePane(layout(), p.id));
        break;
      }
      case "splitRight":
        setPanelOpen(true);
        setLayout(splitFocused(layout(), "row").layout);
        break;
      case "splitDown":
        setPanelOpen(true);
        setLayout(splitFocused(layout(), "column").layout);
        break;
      case "find": {
        const c = focusedController();
        c?.findOpen[1](true);
        break;
      }
      case "attachLastBlock":
        void attachLastBlock();
        break;
      case "nextTab":
        setLayout(selectTab(layout(), 1));
        break;
      case "prevTab":
        setLayout(selectTab(layout(), -1));
        break;
      case "nextPane":
        setLayout(cycleFocus(layout(), 1));
        break;
      case "prevPane":
        setLayout(cycleFocus(layout(), -1));
        break;
    }
  };

  const openPalette = async () => {
    const actions: [Action, string][] = [
      ["newChat", t("actions.newChat")],
      ["toggleTerminal", t("actions.toggleTerminal")],
      ["newTab", t("actions.newTab")],
      ["splitRight", t("actions.splitRight")],
      ["splitDown", t("actions.splitDown")],
      ["closePane", t("actions.closePane")],
      ["find", t("actions.find")],
      ["attachLastBlock", t("actions.attachLastBlock")],
    ];
    const items: PaletteItem[] = [
      ...actions.map(([a, label]) => ({
        id: a,
        label,
        hint: shortcutLabel(a, mac),
        run: () => run(a),
      })),
      { id: "open-project", label: t("actions.openProject"), run: () => switchProject() },
      ...shells().map((s) => ({
        id: `shell:${s.id}`,
        label: t("actions.newTerminalWith", { shell: s.name }),
        run: () => open({ type: "terminal", shellId: s.id }),
      })),
      ...agents().map((a) => ({
        id: `cli:${a.id}`,
        label: t("actions.newExternalAgent", { name: a.name }),
        run: () => open({ type: "terminal", agentId: a.id }),
      })),
      { id: "providers", label: t("actions.providers"), run: () => setProvidersOpen(true) },
      ...THEMES.map((th) => ({
        id: `theme:${th.id}`,
        label: t("actions.theme", { name: th.name }),
        hint: th.id === theme().id ? "✓" : undefined,
        run: () => {
          setTheme(th);
          for (const c of allControllers()) c.applySettings();
          void bridge.settingSet(THEME_KEY, th.id);
        },
      })),
    ];
    try {
      for (const b of await bridge.recentBlocks(20)) {
        items.push({
          id: `block:${b.id}`,
          label: t("actions.attachBlock", { cmd: b.command, code: String(b.exit_code ?? "?") }),
          run: async () => {
            const out = await bridge.blockOutput(b.id);
            attach({ block: b, markdown: blockMarkdown(b, out?.output ?? "") });
            showToast(t("toast.attached", { cmd: b.command }));
          },
        });
      }
    } catch {
      // Recent blocks are optional palette content.
    }
    setPaletteItems(items);
  };

  const onKey = (e: KeyboardEvent) => {
    const a = actionFor(e, mac);
    if (!a) return;
    e.preventDefault();
    e.stopPropagation();
    run(a);
  };

  onMount(async () => {
    window.addEventListener("keydown", onKey, { capture: true });
    onCleanup(() => window.removeEventListener("keydown", onKey, { capture: true }));
    const unlisten = await bridge.onCoreEvent((ev) => {
      if (dispatchAgentEvent(ev) || dispatchToolEvent(ev)) return;
      if ("block" in ev) routeEvent(ev.block.terminal_id, ev);
      else if ("terminal_id" in ev) routeEvent(ev.terminal_id, ev);
    });
    onCleanup(unlisten);

    try {
      const [cfg, savedTheme, savedLayout, sh, ag, lastProject, panel] = await Promise.all([
        bridge.appConfig(),
        bridge.settingGet(THEME_KEY),
        bridge.settingGet(LAYOUT_KEY),
        bridge.listShells(),
        bridge.listAgents(),
        bridge.settingGet(LAST_PROJECT_KEY),
        bridge.settingGet(PANEL_KEY),
      ]);
      await refreshCredentials(bridge).catch(() => {});
      const pnl = JSON.parse(panel ?? "null") as { open: boolean; height: number } | null;
      if (pnl) {
        setPanelOpen(pnl.open);
        setPanelHeight(Math.max(120, pnl.height));
      }
      // Re-open the last project if its folder still exists.
      const w = lastProject ? await bridge.workspace(lastProject).catch(() => null) : null;
      if (w) {
        const reopened = await bridge.openWorkspace(w.path).catch(() => null);
        if (reopened) await openProject(reopened);
      }
      setConfig(cfg);
      setTheme(themeById(savedTheme ?? cfg.ui.theme));
      setShells(sh);
      setAgents(ag);
      const restored = parseLayout(savedLayout);
      if (restored) setLayoutRaw(restored);
    } catch (e) {
      showToast(String(e));
    }
    if (import.meta.env.DEV || import.meta.env.TAURI_ENV_DEBUG === "true") startPerfMonitor(bridge);
    setReady(true);
  });

  const blocksStatus = () => {
    const c = focusedController();
    if (!c) return null;
    return c.degraded[0]() ? t("status.blocksOff") : t("status.blocksOn");
  };

  return (
    <BridgeProvider bridge={bridge}>
      <div class="tf-app">
        <Show when={ready()} fallback={<div class="tf-loading">{t("app.loading")}</div>}>
          <Show when={project()} fallback={<StartScreen onOpen={(w) => void openProject(w)} />}>
            <div class="tf-shell">
              <Sidebar onNewChat={() => void newChat()} onSwitchProject={switchProject} />
              <div class="tf-main">
                <Show when={activeChat()} keyed>
                  {(id) => (
                    <ChatView
                      sessionId={id}
                      onNewChat={() => void newChat()}
                      onAttachLastBlock={() => void attachLastBlock()}
                    />
                  )}
                </Show>
                <Show when={panelOpen()}>
                  {/* biome-ignore lint/a11y/useSemanticElements: an <hr> can't be a draggable flex child. */}
                  <div
                    class="tf-panel-resize"
                    role="separator"
                    aria-orientation="horizontal"
                    onPointerDown={(e) => {
                      const handle = e.currentTarget;
                      handle.setPointerCapture(e.pointerId);
                      const startY = e.clientY;
                      const startH = panelHeight();
                      const move = (ev: PointerEvent) =>
                        setPanelHeight(
                          Math.min(
                            window.innerHeight - 160,
                            Math.max(120, startH + startY - ev.clientY),
                          ),
                        );
                      const up = () => {
                        handle.removeEventListener("pointermove", move);
                        handle.removeEventListener("pointerup", up);
                        savePanel();
                      };
                      handle.addEventListener("pointermove", move);
                      handle.addEventListener("pointerup", up);
                    }}
                  />
                  <section
                    class="tf-panel"
                    style={{ height: `${panelHeight()}px` }}
                    aria-label={t("panel.label")}
                  >
                    <Workspace
                      layout={layout()}
                      onSelectTab={(id) => setLayout({ ...layout(), active: id })}
                      onCloseTab={(id) => setLayout(closeTab(layout(), id).layout)}
                      onRenameTab={(id, title) => setLayout(renameTab(layout(), id, title))}
                      onFocusPane={(id) => {
                        if (focusedPane()?.id !== id) setLayout(focusPane(layout(), id));
                      }}
                      onClosePane={(id) => setLayout(closePane(layout(), id))}
                      onRatio={(tab, path, r) => setLayout(setRatio(layout(), tab, path, r))}
                      onOpenProviders={() => setProvidersOpen(true)}
                      onNewTab={() => run("newTab")}
                      onOpenPalette={() => void openPalette()}
                    />
                  </section>
                </Show>
              </div>
            </div>
          </Show>
        </Show>
        <footer class="tf-status">
          <button
            type="button"
            class="tf-status-btn"
            onClick={togglePanel}
            title={shortcutLabel("toggleTerminal", mac)}
          >
            {panelOpen() ? t("panel.hide") : t("panel.show")}
          </button>
          <button type="button" class="tf-status-btn" onClick={() => setProvidersOpen(true)}>
            {t("providers.title")}
          </button>
          <button type="button" class="tf-status-btn" onClick={() => void openPalette()}>
            {t("palette.open")}
          </button>
          <span>
            {(() => {
              const p = focusedPane();
              if (!p) return "";
              if (p.kind.type === "agent") return t("tabs.agent");
              const id = p.kind.agentId ?? p.kind.shellId;
              return (
                agents().find((a) => a.id === id)?.name ??
                shells().find((s) => s.id === id)?.name ??
                shells()[0]?.name ??
                ""
              );
            })()}
          </span>
          <span class="tf-status-cwd">{project()?.path ?? ""}</span>
          <span class="tf-spacer" />
          <Show when={blocksStatus()}>
            <span data-off={focusedController()?.degraded[0]() ? "1" : "0"}>{blocksStatus()}</span>
          </Show>
        </footer>
        <Show when={providersOpen()}>
          <ProvidersDialog onClose={() => setProvidersOpen(false)} />
        </Show>
        <Show when={paletteItems()}>
          {(items) => <Palette items={items()} onClose={() => setPaletteItems(null)} />}
        </Show>
        <Show when={toast()}>
          <div class="tf-toast" role="status">
            {toast()}
          </div>
        </Show>
      </div>
    </BridgeProvider>
  );
}
