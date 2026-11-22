// Full-screen settings page (M2d, OpenCode-style): left nav, 720px content column.
import { createSignal, For, Match, onMount, Show, Switch } from "solid-js";
import { useBridge } from "../../bridge/context";
import { Icon } from "../../components/Icon";
import { t } from "../../i18n";
import { ACTIONS, isMac, shortcutLabel } from "../../keybindings";
import {
  type Density,
  density,
  type SettingsPage,
  setDensity,
  setSettingsPage,
  settingsPage,
} from "../../stores/layout";
import { theme } from "../../stores/settings";
import { ProvidersPanel } from "../providers/ProvidersPanel";
import { THEMES, type Theme } from "../themes/themes";

const PAGES: SettingsPage[] = ["appearance", "providers", "shortcuts", "about"];
const DENSITIES: Density[] = ["everything", "compact", "messages"];
const densityLabel = (d: Density) =>
  t(
    d === "everything"
      ? "settings.densityEverything"
      : d === "compact"
        ? "settings.densityCompact"
        : "settings.densityMessages",
  );

export function Settings(props: { onTheme: (t: Theme) => void }) {
  const bridge = useBridge();
  const mac = isMac();
  const [version, setVersion] = createSignal("");
  onMount(
    () =>
      void bridge.appInfo().then(
        (i) => setVersion(i.version),
        () => {},
      ),
  );
  const close = () => setSettingsPage(null);

  return (
    <div
      class="tf-settings"
      role="dialog"
      aria-modal="true"
      aria-label={t("settings.title")}
      onKeyDown={(e) => e.key === "Escape" && close()}
    >
      <nav class="tf-settings-nav" aria-label={t("settings.title")}>
        <h1>{t("settings.title")}</h1>
        <For each={PAGES}>
          {(p) => (
            <button
              type="button"
              class="tf-row"
              data-active={settingsPage() === p ? "1" : "0"}
              aria-current={settingsPage() === p ? "page" : undefined}
              onClick={() => setSettingsPage(p)}
            >
              {t(`settings.${p}`)}
            </button>
          )}
        </For>
      </nav>
      <main class="tf-settings-body">
        <button
          type="button"
          class="tf-icon-btn tf-settings-close"
          aria-label={t("settings.close")}
          title={`${t("settings.close")} (Esc)`}
          onClick={close}
          ref={(el) => queueMicrotask(() => el.focus())}
        >
          <Icon name="close" size={16} />
        </button>
        <Switch>
          <Match when={settingsPage() === "appearance"}>
            <section class="tf-settings-section">
              <h2>{t("settings.theme")}</h2>
              <fieldset class="tf-theme-grid" aria-label={t("settings.theme")}>
                <For each={THEMES}>
                  {(th) => (
                    <label
                      data-checked={theme().id === th.id ? "1" : "0"}
                      class="tf-theme-card"
                      style={{ background: th.ui.bg, color: th.ui.text }}
                    >
                      <input
                        type="radio"
                        name="tf-theme"
                        class="tf-sr"
                        checked={theme().id === th.id}
                        onChange={() => props.onTheme(th)}
                      />
                      <span class="tf-theme-swatch" style={{ background: th.ui.deep }}>
                        <i style={{ background: th.ui.surface2 }} />
                        <i style={{ background: th.ui.accent }} />
                        <i style={{ background: th.ui.success }} />
                        <i style={{ background: th.ui.danger }} />
                      </span>
                      {th.name}
                    </label>
                  )}
                </For>
              </fieldset>
              <h2>{t("settings.density")}</h2>
              <p class="tf-muted">{t("settings.densityHint")}</p>
              <fieldset class="tf-segmented" aria-label={t("settings.density")}>
                <For each={DENSITIES}>
                  {(d) => (
                    <label data-checked={density() === d ? "1" : "0"}>
                      <input
                        type="radio"
                        name="tf-density"
                        class="tf-sr"
                        checked={density() === d}
                        onChange={() => setDensity(d)}
                      />
                      {densityLabel(d)}
                    </label>
                  )}
                </For>
              </fieldset>
            </section>
          </Match>
          <Match when={settingsPage() === "providers"}>
            <ProvidersPanel />
          </Match>
          <Match when={settingsPage() === "shortcuts"}>
            <section class="tf-settings-section">
              <h2>{t("settings.shortcuts")}</h2>
              <dl class="tf-keys">
                <For each={ACTIONS}>
                  {(a) => (
                    <div>
                      <dt>{t(`actions.${a}`)}</dt>
                      <dd>
                        <kbd>{shortcutLabel(a, mac)}</kbd>
                      </dd>
                    </div>
                  )}
                </For>
              </dl>
              <h2>{t("settings.composerKeys")}</h2>
              <dl class="tf-keys">
                <For
                  each={
                    [
                      ["keyEnter", "Enter"],
                      ["keyShiftEnter", "Shift+Enter"],
                      ["keyShiftTab", "Shift+Tab"],
                      ["keyModI", mac ? "⌘I" : "Ctrl+I"],
                      ["keyEsc", "Esc"],
                    ] as const
                  }
                >
                  {([k, key]) => (
                    <div>
                      <dt>{t(`settings.${k}`)}</dt>
                      <dd>
                        <kbd>{key}</kbd>
                      </dd>
                    </div>
                  )}
                </For>
              </dl>
            </section>
          </Match>
          <Match when={settingsPage() === "about"}>
            <section class="tf-settings-section">
              <h2>TermFlow</h2>
              <Show when={version()}>
                <p class="tf-muted">{t("settings.version", { v: version() })}</p>
              </Show>
              <p>{t("settings.aboutBody")}</p>
              <p class="tf-muted">{t("settings.credit")}</p>
            </section>
          </Match>
        </Switch>
      </main>
    </div>
  );
}
