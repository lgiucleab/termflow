import { createSignal, For, onMount, Show } from "solid-js";
import type { Workspace } from "../../bridge";
import { useBridge } from "../../bridge/context";
import { Icon } from "../../components/Icon";
import { t } from "../../i18n";

export function StartScreen(props: { onOpen: (w: Workspace) => void }) {
  const bridge = useBridge();
  const [recent, setRecent] = createSignal<Workspace[]>([]);
  const [error, setError] = createSignal<string | null>(null);

  onMount(async () => setRecent(await bridge.recentWorkspaces().catch(() => [])));

  const open = async (path: string) => {
    setError(null);
    try {
      props.onOpen(await bridge.openWorkspace(path));
    } catch (e) {
      setError(String(e));
    }
  };

  const pick = async () => {
    const p = await bridge.pickFolder();
    if (p) await open(p);
  };

  return (
    <div class="tf-start">
      <div class="tf-start-card">
        <h1>TermFlow</h1>
        <p class="tf-muted">{t("start.tagline")}</p>
        <button type="button" class="tf-start-open" onClick={() => void pick()}>
          {t("start.open")}
        </button>
        <Show when={error()}>
          <p class="tf-error" role="alert">
            {error()}
          </p>
        </Show>
        <Show when={recent().length > 0}>
          <h2>{t("start.recent")}</h2>
          <ul class="tf-recent">
            <For each={recent()}>
              {(w) => (
                <li>
                  <button
                    type="button"
                    class="tf-recent-open"
                    onClick={() => void open(w.path)}
                    title={w.path}
                  >
                    <strong>{w.name}</strong>
                    <span class="tf-muted">{w.path}</span>
                  </button>
                  <button
                    type="button"
                    class="tf-recent-forget"
                    aria-label={t("start.forget", { name: w.name })}
                    onClick={async () => {
                      await bridge.forgetWorkspace(w.id);
                      setRecent((r) => r.filter((x) => x.id !== w.id));
                    }}
                  >
                    <Icon name="close" size={11} />
                  </button>
                </li>
              )}
            </For>
          </ul>
        </Show>
      </div>
    </div>
  );
}
