import { createSignal, onCleanup, onMount, Show } from "solid-js";
import type { AgentCli, Bridge, Workspace as Project } from "./bridge";
import { BridgeProvider } from "./bridge/context";
import { ChatView, dispatchToolEvent } from "./features/chat/ChatView";
import {
  allShells,
  disposeAllShells,
  existingShell,
  routeShellEvent,
} from "./features/chat/chatShell";
import { Sidebar } from "./features/chat/Sidebar";
import { StartScreen } from "./features/chat/StartScreen";
import { Palette, type PaletteItem } from "./features/palette/Palette";
import { ProvidersDialog } from "./features/providers/ProvidersDialog";
import { blockMarkdown } from "./features/terminal/markdown";
import { startPerfMonitor } from "./features/terminal/perf";
import { THEMES, themeById } from "./features/themes/themes";
import { t } from "./i18n";
import { type Action, actionFor, isMac, shortcutLabel } from "./keybindings";
import { attach } from "./stores/agent";
import { dispatchAgentEvent } from "./stores/agentEvents";
import {
  activeChat,
  chats,
  LAST_PROJECT_KEY,
  project,
  refreshChats,
  setActiveChat,
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

const THEME_KEY = "theme";

export function App(props: { bridge: Bridge }) {
  const bridge = props.bridge;
  const [ready, setReady] = createSignal(false);
  const [agents, setAgents] = createSignal<AgentCli[]>([]);
  const [paletteItems, setPaletteItems] = createSignal<PaletteItem[] | null>(null);
  const mac = isMac();

  const activeShell = () => {
    const id = activeChat();
    return id ? existingShell(id) : undefined;
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
    disposeAllShells();
    setProject(w);
    void bridge.settingSet(LAST_PROJECT_KEY, w.id);
    await refreshChats(bridge);
    const first = chats()[0];
    if (first) setActiveChat(first.id);
    else await newChat();
  };

  const switchProject = () => {
    disposeAllShells();
    setProject(null);
    setActiveChat(null);
  };

  const attachLastBlock = async () => {
    const [recent] = await bridge.recentBlocks(1).catch(() => []);
    if (!recent) return showToast(t("toast.noBlock"));
    const out = await bridge.blockOutput(recent.id);
    attach({ block: recent, markdown: blockMarkdown(recent, out?.output ?? "") });
    showToast(t("toast.attached", { cmd: recent.command }));
  };

  const selectChat = (delta: number) => {
    const list = chats();
    const i = list.findIndex((c) => c.id === activeChat());
    const next = list[(i + delta + list.length) % list.length];
    if (next) setActiveChat(next.id);
  };

  const run = (a: Action) => {
    switch (a) {
      case "palette":
        void openPalette();
        break;
      case "newChat":
        void newChat();
        break;
      case "toggleTerminal": {
        const s = activeShell();
        if (s) s.rawOpen[0]() ? s.closeRaw() : s.rawOpen[1](true);
        break;
      }
      case "attachLastBlock":
        void attachLastBlock();
        break;
      case "nextChat":
        selectChat(1);
        break;
      case "prevChat":
        selectChat(-1);
        break;
    }
  };

  const openPalette = async () => {
    const actions: [Action, string][] = [
      ["newChat", t("actions.newChat")],
      ["toggleTerminal", t("actions.toggleTerminal")],
      ["attachLastBlock", t("actions.attachLastBlock")],
      ["nextChat", t("actions.nextChat")],
      ["prevChat", t("actions.prevChat")],
    ];
    const items: PaletteItem[] = [
      ...actions.map(([a, label]) => ({
        id: a,
        label,
        hint: shortcutLabel(a, mac),
        run: () => run(a),
      })),
      { id: "open-project", label: t("actions.openProject"), run: () => switchProject() },
      { id: "providers", label: t("actions.providers"), run: () => setProvidersOpen(true) },
      ...agents().map((a) => ({
        id: `cli:${a.id}`,
        label: t("actions.runExternalAgent", { name: a.name }),
        run: () => {
          const s = activeShell();
          if (!s?.run(a.id)) showToast(t("shell.notReady"));
        },
      })),
      ...THEMES.map((th) => ({
        id: `theme:${th.id}`,
        label: t("actions.theme", { name: th.name }),
        hint: th.id === theme().id ? "✓" : undefined,
        run: () => {
          setTheme(th);
          for (const s of allShells()) s.applyTheme();
          void bridge.settingSet(THEME_KEY, th.id);
        },
      })),
    ];
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
      routeShellEvent(ev);
    });
    onCleanup(unlisten);

    try {
      const [cfg, savedTheme, ag, lastProject] = await Promise.all([
        bridge.appConfig(),
        bridge.settingGet(THEME_KEY),
        bridge.listAgents(),
        bridge.settingGet(LAST_PROJECT_KEY),
      ]);
      setConfig(cfg);
      setTheme(themeById(savedTheme ?? cfg.ui.theme));
      setAgents(ag);
      await refreshCredentials(bridge).catch(() => {});
      // Re-open the last project if its folder still exists.
      const w = lastProject ? await bridge.workspace(lastProject).catch(() => null) : null;
      if (w) {
        const reopened = await bridge.openWorkspace(w.path).catch(() => null);
        if (reopened) await openProject(reopened);
      }
    } catch (e) {
      showToast(String(e));
    }
    if (import.meta.env.DEV || import.meta.env.TAURI_ENV_DEBUG === "true") startPerfMonitor(bridge);
    setReady(true);
  });

  const shellStatus = () => {
    const st = activeShell()?.state[0]();
    if (!st) return "";
    return t(`shell.state.${st}`);
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
              </div>
            </div>
          </Show>
        </Show>
        <footer class="tf-status">
          <span class="tf-status-cwd">{project()?.path ?? ""}</span>
          <span class="tf-spacer" />
          <span data-off={activeShell()?.state[0]() === "degraded" ? "1" : "0"}>
            {shellStatus()}
          </span>
          <button type="button" class="tf-status-btn" onClick={() => setProvidersOpen(true)}>
            {t("providers.title")}
          </button>
          <button
            type="button"
            class="tf-status-btn"
            onClick={() => void openPalette()}
            title={shortcutLabel("palette", mac)}
          >
            {t("palette.open")}
          </button>
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
