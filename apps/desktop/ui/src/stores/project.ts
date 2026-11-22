// The open project, its chats, and the terminal panel.
import { createSignal } from "solid-js";
import type { AgentSession, Bridge, Workspace } from "../bridge";

export const LAST_PROJECT_KEY = "project.last";
export const PANEL_KEY = "ui.terminal_panel";

const [project, setProject] = createSignal<Workspace | null>(null);
const [chats, setChats] = createSignal<AgentSession[]>([]);
const [activeChat, setActiveChat] = createSignal<string | null>(null);
const [panelOpen, setPanelOpen] = createSignal(false);
const [panelHeight, setPanelHeight] = createSignal(280);
/** Session ids with the agent currently working. */
const [busy, setBusy] = createSignal<ReadonlySet<string>>(new Set());

export {
  activeChat,
  busy,
  chats,
  panelHeight,
  panelOpen,
  project,
  setActiveChat,
  setBusy,
  setChats,
  setPanelHeight,
  setPanelOpen,
  setProject,
};

export function markBusy(id: string, on: boolean): void {
  setBusy((s) => {
    const n = new Set(s);
    if (on) n.add(id);
    else n.delete(id);
    return n;
  });
}

export async function refreshChats(bridge: Bridge): Promise<void> {
  const p = project();
  setChats(p ? await bridge.agentSessions(p.id) : []);
}
