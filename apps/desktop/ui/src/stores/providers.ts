import { createSignal } from "solid-js";
import type { Bridge, Credential } from "../bridge";

const [credentials, setCredentials] = createSignal<Credential[]>([]);
const [providersOpen, setProvidersOpen] = createSignal(false);

export { credentials, providersOpen, setProvidersOpen };

export async function refreshCredentials(bridge: Bridge): Promise<void> {
  setCredentials(await bridge.credentials());
}

/** Last model picked anywhere; new agent sessions start with it. */
export interface ModelChoice {
  credentialId: string;
  model: string;
  reasoning: boolean;
}
export const LAST_MODEL_KEY = "agent.last_model";
