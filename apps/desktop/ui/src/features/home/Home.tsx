import { createResource, createSignal, onCleanup, onMount, Show } from "solid-js";
import type { Bridge } from "../../bridge";
import { t } from "../../i18n";

export function Home(props: { bridge: Bridge }) {
  const [info] = createResource(() => props.bridge.appInfo());
  const [ticks, setTicks] = createSignal(0);

  onMount(async () => {
    const unlisten = await props.bridge.onCoreEvent((e) => {
      if (e.type === "tick") setTicks(e.n);
    });
    onCleanup(unlisten);
    await props.bridge.startTicker();
  });

  return (
    <main class="flex h-full flex-col items-center justify-center gap-3 font-ui">
      <h1 class="text-3xl font-semibold tracking-tight">TermFlow</h1>
      <p class="text-muted">{t("app.tagline")}</p>
      <Show when={info()} fallback={<p class="text-muted">{t("home.loading")}</p>}>
        {(i) => <p data-testid="version">{t("home.version", { version: i().version })}</p>}
      </Show>
      <p
        data-testid="ticks"
        class="rounded-[var(--tf-radius)] border border-border bg-surface px-3 py-1 font-mono text-sm"
        aria-live="polite"
      >
        {t("home.events", { n: String(ticks()) })}
      </p>
    </main>
  );
}
