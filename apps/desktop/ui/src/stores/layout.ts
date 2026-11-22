// Window layout (M2d): sidebar, right panel, bottom terminal, settings page. Sizes are a
// per-machine convenience, so they live in localStorage (wrapped: it can be unavailable).
import { createSignal } from "solid-js";

export type SettingsPage = "appearance" | "providers" | "shortcuts" | "about";
export type Density = "everything" | "compact" | "messages";
const DENSITIES: readonly Density[] = ["everything", "compact", "messages"];
export type RightTab = "todo" | "context";

export const SIDEBAR = { min: 136, max: 640, initial: 240 } as const;
export const RIGHT = { min: 320, max: 640, initial: 360 } as const;
export const BOTTOM = { min: 120, maxRatio: 0.7, initial: 280 } as const;

export const clamp = (v: number, min: number, max: number): number =>
  Math.round(Math.min(Math.max(v, min), Math.max(min, max)));

function stored(key: string, fallback: number): number {
  try {
    const v = Number(localStorage.getItem(`layout.${key}`));
    return Number.isFinite(v) && v > 0 ? v : fallback;
  } catch {
    return fallback;
  }
}
function storedFlag(key: string, fallback: boolean): boolean {
  try {
    const v = localStorage.getItem(`layout.${key}`);
    return v === null ? fallback : v === "1";
  } catch {
    return fallback;
  }
}
function storedDensity(): Density {
  try {
    const v = localStorage.getItem("layout.density");
    return DENSITIES.find((d) => d === v) ?? "everything";
  } catch {
    return "everything";
  }
}
function persist(key: string, v: number | boolean | string): void {
  try {
    localStorage.setItem(`layout.${key}`, String(v));
  } catch {
    // private mode / blocked storage: the layout just won't be remembered
  }
}

const [sidebarOpen, setSidebarOpenSignal] = createSignal(storedFlag("sidebarOpen", true));
const [sidebarWidth, setSidebarWidthSignal] = createSignal(
  clamp(stored("sidebarWidth", SIDEBAR.initial), SIDEBAR.min, SIDEBAR.max),
);
const [rightOpen, setRightOpenSignal] = createSignal(storedFlag("rightOpen", false));
const [rightTab, setRightTab] = createSignal<RightTab>("todo");
const [rightWidth, setRightWidthSignal] = createSignal(
  clamp(stored("rightWidth", RIGHT.initial), RIGHT.min, RIGHT.max),
);
const [bottomHeight, setBottomHeightSignal] = createSignal(stored("bottomHeight", BOTTOM.initial));
const [settingsPage, setSettingsPage] = createSignal<SettingsPage | null>(null);
const [density, setDensitySignal] = createSignal<Density>(storedDensity());

export const setDensity = (d: Density) => {
  setDensitySignal(d);
  persist("density", d);
};
export {
  bottomHeight,
  density,
  rightOpen,
  rightTab,
  rightWidth,
  setRightTab,
  setSettingsPage,
  settingsPage,
  sidebarOpen,
  sidebarWidth,
};

export const setSidebarOpen = (v: boolean) => {
  setSidebarOpenSignal(v);
  persist("sidebarOpen", v ? 1 : 0);
};
export const setRightOpen = (v: boolean) => {
  setRightOpenSignal(v);
  persist("rightOpen", v ? 1 : 0);
};
export const setSidebarWidth = (v: number) => {
  const w = clamp(v, SIDEBAR.min, SIDEBAR.max);
  setSidebarWidthSignal(w);
  persist("sidebarWidth", w);
};
export const setRightWidth = (v: number) => {
  const w = clamp(v, RIGHT.min, RIGHT.max);
  setRightWidthSignal(w);
  persist("rightWidth", w);
};
/** Bottom terminal height, capped at 70% of the available height. */
export const setBottomHeight = (v: number, available: number) => {
  const h = clamp(v, BOTTOM.min, available * BOTTOM.maxRatio);
  setBottomHeightSignal(h);
  persist("bottomHeight", h);
};
