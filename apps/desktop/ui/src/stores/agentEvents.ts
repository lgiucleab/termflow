// Fan-out of agent and router core events to the chat views that show each session.
import type { AppEvent } from "../bridge";

const TYPES = [
  "agent_turn_started",
  "agent_delta",
  "agent_turn_committed",
  "agent_error",
  "route_switched",
  "route_waiting",
  "route_paused",
  "budget_warning",
  "budget_stopped",
] as const;

type AgentEvent = Extract<AppEvent, { type: (typeof TYPES)[number] }>;
type Listener = (e: AgentEvent) => void;

const listeners = new Set<Listener>();

export function onAgentEvent(l: Listener): () => void {
  listeners.add(l);
  return () => listeners.delete(l);
}

export function dispatchAgentEvent(e: AppEvent): boolean {
  if (!(TYPES as readonly string[]).includes(e.type)) return false;
  for (const l of listeners) l(e as AgentEvent);
  return true;
}

export const sessionOf = (e: AgentEvent): string =>
  e.type === "agent_turn_committed" ? e.turn.session_id : e.session_id;
