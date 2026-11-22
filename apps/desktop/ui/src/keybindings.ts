// App shortcuts. macOS: Cmd+key (Cmd never reaches the shell). Windows/Linux: Ctrl+Shift+key,
// because plain Ctrl+K/J/N/A mean things inside shells and programs.

export type Action =
  | "palette"
  | "newChat"
  | "toggleTerminal"
  | "attachLastBlock"
  | "nextChat"
  | "prevChat";

export interface KeyLike {
  key: string;
  metaKey: boolean;
  ctrlKey: boolean;
  shiftKey: boolean;
  altKey: boolean;
}

export const isMac = (): boolean =>
  typeof navigator !== "undefined" &&
  /Mac|iPhone|iPad/.test(navigator.platform || navigator.userAgent);

const MAC: Record<string, Action> = {
  k: "palette",
  j: "toggleTerminal",
  "shift+n": "newChat",
  "shift+a": "attachLastBlock",
  "shift+]": "nextChat",
  "shift+}": "nextChat",
  "shift+[": "prevChat",
  "shift+{": "prevChat",
};

const OTHER: Record<string, Action> = {
  k: "palette",
  p: "palette",
  j: "toggleTerminal",
  n: "newChat",
  a: "attachLastBlock",
  pagedown: "nextChat",
  pageup: "prevChat",
};

export function actionFor(e: KeyLike, mac: boolean): Action | null {
  if (e.altKey) return null;
  const k = e.key.toLowerCase();
  if (mac) {
    if (!e.metaKey || e.ctrlKey) return null;
    return MAC[e.shiftKey ? `shift+${k}` : k] ?? null;
  }
  if (!e.ctrlKey || !e.shiftKey || e.metaKey) return null;
  return OTHER[k] ?? null;
}

const LABELS: Record<Action, [string, string]> = {
  palette: ["⌘K", "Ctrl+Shift+K"],
  newChat: ["⌘⇧N", "Ctrl+Shift+N"],
  toggleTerminal: ["⌘J", "Ctrl+Shift+J"],
  attachLastBlock: ["⌘⇧A", "Ctrl+Shift+A"],
  nextChat: ["⌘⇧]", "Ctrl+Shift+PgDn"],
  prevChat: ["⌘⇧[", "Ctrl+Shift+PgUp"],
};

/** Human label for menus/palette, e.g. "⌘⇧N" or "Ctrl+Shift+N". */
export const shortcutLabel = (action: Action, mac: boolean): string => LABELS[action][mac ? 0 : 1];
