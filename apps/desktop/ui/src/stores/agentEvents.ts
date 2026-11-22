// Fan-out of agent core events to the Agent tabs that show each session.
import type { AppEvent } from "../bridge";

type AgentEvent = Extract<
  AppEvent,
  { type: "agent_turn_started" | "agent_delta" | "agent_turn_committed" | "agent_error" }
>;
type Listener = (e: AgentEvent) => void;

const listeners = new Set<Listener>();

export function onAgentEvent(l: Listener): () => void {
  listeners.add(l);
  return () => listeners.delete(l);
}

export function dispatchAgentEvent(e: AppEvent): boolean {
  if (
    e.type !== "agent_turn_started" &&
    e.type !== "agent_delta" &&
    e.type !== "agent_turn_committed" &&
    e.type !== "agent_error"
  )
    return false;
  for (const l of listeners) l(e);
  return true;
}

export const sessionOf = (e: AgentEvent): string =>
  e.type === "agent_turn_committed" ? e.turn.session_id : e.session_id;
