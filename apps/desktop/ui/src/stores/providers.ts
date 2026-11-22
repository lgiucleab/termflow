import { createSignal } from "solid-js";
import type { Bridge, Credential } from "../bridge";
import { setSettingsPage } from "./layout";

const [credentials, setCredentials] = createSignal<Credential[]>([]);

export { credentials };
/** Providers live on a Settings page now (M2d). */
export const setProvidersOpen = (open: boolean) => setSettingsPage(open ? "providers" : null);

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
