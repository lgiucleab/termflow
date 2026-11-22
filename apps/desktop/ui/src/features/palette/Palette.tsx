import { createMemo, createSignal, For, onMount, Show } from "solid-js";
import { t } from "../../i18n";
import { rank } from "./fuzzy";

export interface PaletteItem {
  id: string;
  label: string;
  hint?: string;
  run: () => void;
}

export function Palette(props: { items: PaletteItem[]; onClose: () => void }) {
  const [q, setQ] = createSignal("");
  const [sel, setSel] = createSignal(0);
  const results = createMemo(() => rank(props.items, q(), (i) => i.label).slice(0, 50));
  let input!: HTMLInputElement;
  onMount(() => input.focus());

  const run = (i: PaletteItem | undefined) => {
    if (!i) return;
    props.onClose();
    i.run();
  };

  return (
    // biome-ignore lint/a11y/noStaticElementInteractions: clicking the backdrop is a mouse shortcut; Esc closes from the keyboard.
    <div class="tf-dialog-backdrop" onMouseDown={props.onClose}>
      <div
        class="tf-palette"
        role="dialog"
        aria-modal="true"
        aria-label={t("palette.title")}
        onMouseDown={(e) => e.stopPropagation()}
      >
        <input
          ref={input}
          value={q()}
          placeholder={t("palette.placeholder")}
          aria-label={t("palette.placeholder")}
          aria-controls="tf-palette-list"
          aria-activedescendant={`tf-pi-${sel()}`}
          onInput={(e) => {
            setQ(e.currentTarget.value);
            setSel(0);
          }}
          onKeyDown={(e) => {
            if (e.key === "ArrowDown") {
              e.preventDefault();
              setSel((s) => Math.min(s + 1, results().length - 1));
            } else if (e.key === "ArrowUp") {
              e.preventDefault();
              setSel((s) => Math.max(s - 1, 0));
            } else if (e.key === "Enter") {
              run(results()[sel()]);
            } else if (e.key === "Escape") {
              props.onClose();
            }
          }}
        />
        <div id="tf-palette-list" role="listbox" class="tf-palette-list">
          <For each={results()}>
            {(item, i) => (
              <div
                id={`tf-pi-${i()}`}
                role="option"
                tabIndex={-1}
                aria-selected={i() === sel()}
                data-selected={i() === sel() ? "1" : "0"}
                onMouseEnter={() => setSel(i())}
                onClick={() => run(item)}
                onKeyDown={(e) => e.key === "Enter" && run(item)}
              >
                <span>{item.label}</span>
                <Show when={item.hint}>
                  <kbd>{item.hint}</kbd>
                </Show>
              </div>
            )}
          </For>
          <Show when={results().length === 0}>
            <div class="tf-muted tf-palette-empty">{t("palette.noResults")}</div>
          </Show>
        </div>
      </div>
    </div>
  );
}
