import { createContext, type JSX, useContext } from "solid-js";
import type { Bridge } from ".";

const BridgeContext = createContext<Bridge>();

export function BridgeProvider(props: { bridge: Bridge; children: JSX.Element }) {
  return <BridgeContext.Provider value={props.bridge}>{props.children}</BridgeContext.Provider>;
}

export function useBridge(): Bridge {
  const b = useContext(BridgeContext);
  if (!b) throw new Error("useBridge outside BridgeProvider");
  return b;
}
