// The ONLY module that talks to Tauri. Everything else imports from here.
import { Channel } from "@tauri-apps/api/core";
import type { UnlistenFn } from "@tauri-apps/api/event";
import { openUrl } from "@tauri-apps/plugin-opener";
import {
  type AgentCli,
  type AppEvent_Serialize,
  type AppInfo,
  type Block_Serialize,
  type BlockId,
  type Config_Serialize,
  commands,
  events,
  type ShellSpec,
  type TerminalId,
} from "./generated/bindings";

export type AppEvent = AppEvent_Serialize;
export type Block = Block_Serialize;
export type Config = Config_Serialize;
export type { AgentCli, AppInfo, BlockId, ShellSpec, TerminalId };

export interface OpenTerminalOptions {
  shellId?: string;
  cwd?: string;
  agentId?: string;
  cols: number;
  rows: number;
}

export interface Bridge {
  appInfo(): Promise<AppInfo>;
  appConfig(): Promise<Config>;
  listShells(): Promise<ShellSpec[]>;
  listAgents(): Promise<AgentCli[]>;
  /** `onData` receives raw PTY bytes, batched by the core (~8 ms / 64 KiB). */
  openTerminal(opts: OpenTerminalOptions, onData: (bytes: Uint8Array) => void): Promise<TerminalId>;
  writeTerminal(id: TerminalId, data: string): Promise<void>;
  resizeTerminal(id: TerminalId, cols: number, rows: number): Promise<void>;
  setPaused(id: TerminalId, paused: boolean): Promise<void>;
  closeTerminal(id: TerminalId): Promise<void>;
  blockOutput(id: BlockId): Promise<{ block: Block; output: string } | null>;
  recentBlocks(limit: number): Promise<Block[]>;
  settingGet(key: string): Promise<string | null>;
  settingSet(key: string, value: string): Promise<void>;
  perfReport(report: string): Promise<void>;
  openUrl(url: string): Promise<void>;
  onCoreEvent(handler: (e: AppEvent) => void): Promise<UnlistenFn>;
}

const done = async (p: Promise<unknown>): Promise<void> => {
  await p;
};

export const tauriBridge: Bridge = {
  appInfo: () => commands.appInfo(),
  appConfig: () => commands.appConfig(),
  listShells: () => commands.listShells(),
  listAgents: () => commands.listAgents(),
  openTerminal: (opts, onData) => {
    const channel = new Channel<ArrayBuffer>();
    channel.onmessage = (buf) => onData(new Uint8Array(buf));
    return commands.terminalOpen({
      shell_id: opts.shellId ?? null,
      cwd: opts.cwd ?? null,
      agent_id: opts.agentId ?? null,
      cols: opts.cols,
      rows: opts.rows,
      on_output: channel.toJSON(),
    });
  },
  writeTerminal: (id, data) => done(commands.terminalWrite(id, data)),
  resizeTerminal: (id, c, r) => done(commands.terminalResize(id, c, r)),
  setPaused: (id, p) => done(commands.terminalSetPaused(id, p)),
  closeTerminal: (id) => done(commands.terminalClose(id)),
  blockOutput: (id) => commands.blockOutput(id),
  recentBlocks: (limit) => commands.recentBlocks(limit),
  settingGet: (key) => commands.settingGet(key),
  settingSet: (key, value) => done(commands.settingSet(key, value)),
  perfReport: (r) => done(commands.perfReport(r)),
  openUrl: (url) => openUrl(url),
  onCoreEvent: (handler) =>
    events.coreEvent.listen((e) => handler(e.payload as AppEvent_Serialize)),
};
