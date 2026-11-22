// Terminal controllers outlive the components that show them, so splitting or moving a pane
// never kills its shell. Controllers are disposed only when their pane is closed.
import type { AppEvent, Bridge, TerminalId } from "../../bridge";
import type { Pane } from "../layout/model";
import { TerminalController } from "./controller";

const byPane = new Map<string, TerminalController>();
/** Core events that arrived before `openTerminal` returned the id; replayed by the controller. */
const early = new Map<TerminalId, AppEvent[]>();

export function routeEvent(id: TerminalId, ev: AppEvent): void {
  const c = byTerminal(id);
  if (c) c.handle(ev);
  else early.set(id, [...(early.get(id) ?? []), ev]);
}

export function takeEarlyEvents(id: TerminalId): AppEvent[] {
  const evs = early.get(id) ?? [];
  early.delete(id);
  return evs;
}

export function controllerFor(
  pane: Pane,
  bridge: Bridge,
  onExitClose: (paneId: string) => void,
): TerminalController {
  let c = byPane.get(pane.id);
  if (!c) {
    const kind = pane.kind.type === "terminal" ? pane.kind : undefined;
    c = new TerminalController(pane.id, bridge, {
      shellId: kind?.shellId,
      agentId: kind?.agentId,
      onExitClose,
    });
    byPane.set(pane.id, c);
  }
  return c;
}

export const existingController = (paneId: string): TerminalController | undefined =>
  byPane.get(paneId);

export function byTerminal(id: TerminalId): TerminalController | undefined {
  for (const c of byPane.values()) if (c.terminalId === id) return c;
  return undefined;
}

export function disposeController(paneId: string): void {
  byPane.get(paneId)?.dispose();
  byPane.delete(paneId);
}

export const allControllers = (): TerminalController[] => [...byPane.values()];
