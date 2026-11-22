import { createEffect, createSignal, onCleanup, onMount, Show } from "solid-js";
import { useBridge } from "../../bridge/context";
import { Icon } from "../../components/Icon";
import { t } from "../../i18n";
import type { Pane } from "../layout/model";
import { controllerFor } from "./registry";

export function TerminalPane(props: {
  pane: Pane;
  focused: boolean;
  onFocus: () => void;
  onExitClose: (paneId: string) => void;
}) {
  const bridge = useBridge();
  const c = controllerFor(props.pane, bridge, props.onExitClose);
  let slot!: HTMLDivElement;
  let findInput: HTMLInputElement | undefined;
  const [query, setQuery] = createSignal("");
  const [notFound, setNotFound] = createSignal(false);

  onMount(() => {
    c.attachTo(slot);
    const ro = new ResizeObserver(() => c.fit());
    ro.observe(slot);
    onCleanup(() => {
      ro.disconnect();
      c.detach();
    });
  });

  createEffect(() => {
    if (props.focused && !c.findOpen[0]() && !c.pendingPaste[0]()) queueMicrotask(() => c.focus());
  });
  createEffect(() => {
    if (c.findOpen[0]()) queueMicrotask(() => findInput?.focus());
  });

  const find = (back = false) => setNotFound(!!query() && !c.findNext(query(), back));
  const closeFind = () => {
    c.clearFind();
    c.findOpen[1](false);
    c.focus();
  };

  return (
    // biome-ignore lint/a11y/noStaticElementInteractions: mouse focus follows click; keyboard focus moves with pane shortcuts.
    <div
      class="tf-pane"
      data-focused={props.focused ? "1" : "0"}
      onMouseDown={() => props.onFocus()}
    >
      <div ref={slot} class="tf-pane-slot" />
      <Show when={c.degraded[0]()}>
        <output class="tf-pane-badge" title={t("blocks.unavailableHint")}>
          <Icon name="warn" size={12} /> {t("blocks.unavailable")}
        </output>
      </Show>
      <Show when={c.findOpen[0]()}>
        <search class="tf-findbar">
          <Icon name="search" size={12} />
          <input
            ref={findInput}
            value={query()}
            placeholder={t("find.placeholder")}
            aria-label={t("find.placeholder")}
            data-notfound={notFound() ? "1" : "0"}
            onInput={(e) => {
              setQuery(e.currentTarget.value);
              find();
            }}
            onKeyDown={(e) => {
              if (e.key === "Enter") find(e.shiftKey);
              if (e.key === "Escape") closeFind();
            }}
          />
          <button type="button" aria-label={t("find.previous")} onClick={() => find(true)}>
            <Icon name="up" size={12} />
          </button>
          <button type="button" aria-label={t("find.next")} onClick={() => find()}>
            <Icon name="down" size={12} />
          </button>
          <button type="button" aria-label={t("find.close")} onClick={closeFind}>
            <Icon name="close" size={12} />
          </button>
        </search>
      </Show>
      <Show when={c.pendingPaste[0]()}>
        {(text) => (
          <div class="tf-dialog-backdrop">
            <div
              class="tf-dialog"
              role="alertdialog"
              aria-modal="true"
              aria-labelledby="tf-paste-title"
            >
              <h2 id="tf-paste-title">{t("paste.title")}</h2>
              <p>{t("paste.body", { n: String(text().split(/\r\n|\r|\n/).length) })}</p>
              <pre>{text().slice(0, 600)}</pre>
              <div class="tf-dialog-actions">
                <button type="button" onClick={() => c.confirmPaste(false)}>
                  {t("common.cancel")}
                </button>
                <button
                  type="button"
                  class="tf-primary"
                  autofocus
                  onClick={() => c.confirmPaste(true)}
                >
                  {t("paste.confirm")}
                </button>
              </div>
            </div>
          </div>
        )}
      </Show>
    </div>
  );
}
