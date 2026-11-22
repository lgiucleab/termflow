// The ONLY module that talks to Tauri. Everything else imports from here.
import { Channel } from "@tauri-apps/api/core";
import type { UnlistenFn } from "@tauri-apps/api/event";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { openUrl } from "@tauri-apps/plugin-opener";
import {
  type AddCredential_Deserialize,
  type AgentCli,
  type AgentSession_Serialize,
  type AppEvent_Serialize,
  type AppInfo,
  type ApprovalKind,
  type Block_Serialize,
  type BlockId,
  type CallId,
  type Config_Serialize,
  type Credential,
  commands,
  events,
  type ModelInfo_Serialize,
  type Part_Serialize,
  type SessionId,
  type SessionUsage,
  type ShellSpec,
  type TerminalId,
  type Turn_Serialize,
  type Workspace_Serialize,
} from "./generated/bindings";

export type AppEvent = AppEvent_Serialize;
export type Block = Block_Serialize;
export type Config = Config_Serialize;
export type Turn = Turn_Serialize;
export type Part = Part_Serialize;
export type NewCredential = AddCredential_Deserialize;
export type AgentSession = AgentSession_Serialize;
export type Workspace = Workspace_Serialize;
export type { ApprovalKind, CallId, SessionUsage };
export type ModelInfo = ModelInfo_Serialize;
export type { AgentCli, AppInfo, BlockId, Credential, SessionId, ShellSpec, TerminalId };

export interface OpenTerminalOptions {
  shellId?: string;
  cwd?: string;
  agentId?: string;
  /** Chat that owns this shell (its blocks join that chat's timeline). */
  chatSessionId?: string;
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
  credentials(): Promise<Credential[]>;
  addCredential(req: NewCredential): Promise<Credential>;
  deleteCredential(id: string): Promise<void>;
  /** Also the connection test: errors come back classified. */
  listModels(credentialId: string): Promise<ModelInfo[]>;
  agentSessions(workspaceId: string | null): Promise<AgentSession[]>;
  agentSession(id: SessionId): Promise<AgentSession>;
  createAgentSession(
    workspaceId: string | null,
    credentialId: string | null,
    model: string | null,
  ): Promise<AgentSession>;
  setAgentMode(id: SessionId, plan: boolean): Promise<void>;
  renameAgentSession(id: SessionId, title: string): Promise<void>;
  deleteAgentSession(id: SessionId): Promise<void>;
  agentInit(id: SessionId): Promise<void>;
  /** Answer an approval card; false if it was no longer waiting. */
  agentApprove(callId: CallId, kind: ApprovalKind, feedback: string | null): Promise<boolean>;
  agentUsage(id: SessionId): Promise<SessionUsage>;
  recentWorkspaces(): Promise<Workspace[]>;
  openWorkspace(path: string): Promise<Workspace>;
  forgetWorkspace(id: string): Promise<void>;
  workspace(id: string): Promise<Workspace | null>;
  projectFiles(workspaceId: string): Promise<string[]>;
  /** A chat's command blocks with output, oldest first. */
  chatBlocks(id: SessionId): Promise<{ block: Block; output: string }[]>;
  /** Executable names on PATH (for shell-vs-agent detection). */
  shellExecutables(): Promise<string[]>;
  /** Native folder picker; null if cancelled. */
  pickFolder(): Promise<string | null>;
  setAgentModel(
    id: SessionId,
    credentialId: string,
    model: string,
    reasoning: boolean,
  ): Promise<void>;
  agentTranscript(id: SessionId): Promise<Turn[]>;
  agentSend(id: SessionId, text: string): Promise<void>;
  agentCancel(id: SessionId): Promise<void>;
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
      chat_session_id: opts.chatSessionId ?? null,
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
  credentials: () => commands.credentialsList(),
  addCredential: (req) => commands.credentialAdd(req),
  deleteCredential: (id) => done(commands.credentialDelete(id)),
  listModels: (id) => commands.modelsList(id),
  agentSessions: (w) => commands.agentSessions(w),
  agentSession: (id) => commands.agentSession(id),
  createAgentSession: (w, c, m) => commands.agentSessionCreate(w, c, m),
  setAgentMode: (id, plan) => done(commands.agentSetMode(id, plan)),
  renameAgentSession: (id, t) => done(commands.agentRename(id, t)),
  deleteAgentSession: (id) => done(commands.agentDelete(id)),
  agentInit: (id) => done(commands.agentInit(id)),
  agentApprove: (c, k, f) => commands.agentApprove(c, k, f),
  agentUsage: (id) => commands.agentUsage(id),
  recentWorkspaces: () => commands.workspacesRecent(),
  openWorkspace: (p) => commands.workspaceOpen(p),
  forgetWorkspace: (id) => done(commands.workspaceForget(id)),
  workspace: (id) => commands.workspaceGet(id),
  projectFiles: (w) => commands.projectFiles(w),
  chatBlocks: (id) => commands.chatBlocks(id),
  shellExecutables: () => commands.shellExecutables(),
  pickFolder: async () => {
    const r = await openDialog({ directory: true, multiple: false });
    return typeof r === "string" ? r : null;
  },
  setAgentModel: (id, c, m, r) => done(commands.agentSetModel(id, c, m, r)),
  agentTranscript: (id) => commands.agentTranscript(id),
  agentSend: (id, text) => done(commands.agentSend(id, text)),
  agentCancel: (id) => done(commands.agentCancel(id)),
};
