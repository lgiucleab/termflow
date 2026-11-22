// App shortcuts. macOS: Cmd+key (Cmd is never sent to the terminal). Windows/Linux: Ctrl+Shift+key,
// because plain Ctrl+T/W/D/K/F mean things inside shells (new line ops, EOF, kill line, ...).

export type Action =
  | "palette"
  | "newTab"
  | "closePane"
  | "splitRight"
  | "splitDown"
  | "find"
  | "attachLastBlock"
  | "nextTab"
  | "prevTab"
  | "nextPane"
  | "prevPane"
  | "toggleTerminal"
  | "newChat";

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

export function actionFor(e: KeyLike, mac: boolean): Action | null {
  const k = e.key.toLowerCase();
  if (e.altKey) return null;
  if (mac) {
    if (!e.metaKey || e.ctrlKey) return null;
    if (e.shiftKey) {
      if (k === "d") return "splitDown";
      if (k === "a") return "attachLastBlock";
      if (k === "n") return "newChat";
      if (k === "]" || k === "}") return "nextTab";
      if (k === "[" || k === "{") return "prevTab";
      return null;
    }
    const plain: Record<string, Action> = {
      k: "palette",
      t: "newTab",
      w: "closePane",
      d: "splitRight",
      f: "find",
      j: "toggleTerminal",
      "]": "nextPane",
      "[": "prevPane",
    };
    return plain[k] ?? null;
  }
  if (!e.ctrlKey || !e.shiftKey || e.metaKey) return null;
  const map: Record<string, Action> = {
    k: "palette",
    p: "palette",
    t: "newTab",
    w: "closePane",
    d: "splitRight",
    e: "splitDown",
    f: "find",
    a: "attachLastBlock",
    j: "toggleTerminal",
    n: "newChat",
    pagedown: "nextTab",
    pageup: "prevTab",
    "]": "nextPane",
    "[": "prevPane",
  };
  return map[k] ?? null;
}

/** Human label for menus/palette, e.g. "⌘⇧D" or "Ctrl+Shift+E". */
export function shortcutLabel(action: Action, mac: boolean): string {
  const macKeys: Partial<Record<Action, string>> = {
    palette: "⌘K",
    newTab: "⌘T",
    closePane: "⌘W",
    splitRight: "⌘D",
    splitDown: "⌘⇧D",
    find: "⌘F",
    attachLastBlock: "⌘⇧A",
    toggleTerminal: "⌘J",
    newChat: "⌘⇧N",
  };
  const otherKeys: Partial<Record<Action, string>> = {
    palette: "Ctrl+Shift+K",
    newTab: "Ctrl+Shift+T",
    closePane: "Ctrl+Shift+W",
    splitRight: "Ctrl+Shift+D",
    splitDown: "Ctrl+Shift+E",
    find: "Ctrl+Shift+F",
    attachLastBlock: "Ctrl+Shift+A",
    toggleTerminal: "Ctrl+Shift+J",
    newChat: "Ctrl+Shift+N",
  };
  return (mac ? macKeys : otherKeys)[action] ?? "";
}
