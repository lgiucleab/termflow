// The ONLY module that talks to Tauri. Everything else imports from here.
import type { UnlistenFn } from "@tauri-apps/api/event";
import { type AppEvent, type AppInfo, commands, events } from "./generated/bindings";

export type { AppEvent, AppInfo };

export interface Bridge {
  appInfo(): Promise<AppInfo>;
  startTicker(): Promise<void>;
  onCoreEvent(handler: (e: AppEvent) => void): Promise<UnlistenFn>;
}

export const tauriBridge: Bridge = {
  appInfo: () => commands.appInfo(),
  startTicker: async () => {
    await commands.startTicker();
  },
  onCoreEvent: (handler) => events.coreEvent.listen((e) => handler(e.payload)),
};
