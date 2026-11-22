import { createSignal } from "solid-js";
import type { Config } from "../bridge";
import { applyTheme, type Theme, themeById } from "../features/themes/themes";

export const DEFAULT_FONT = '"JetBrains Mono Variable", ui-monospace, Menlo, Consolas, monospace';

const defaults: Config = {
  ui: { theme: "dark", locale: null },
  terminal: {
    shell: null,
    shell_integration: true,
    font_family: null,
    font_size: 13,
    ligatures: false,
    copy_on_select: false,
    warn_multiline_paste: true,
    scrollback: 10_000,
  },
  updates: { check: true },
};

const [config, setConfig] = createSignal<Config>(defaults);
const [theme, setThemeSignal] = createSignal<Theme>(themeById("dark"));

export { config, setConfig, theme };

export function setTheme(t: Theme): void {
  applyTheme(t);
  setThemeSignal(t);
}

export const fontFamily = (): string => config().terminal.font_family ?? DEFAULT_FONT;
