import { For, Show } from "solid-js";
import { Icon } from "../../components/Icon";
import { t } from "../../i18n";
import { attachments, detach } from "../../stores/agent";

/** M1 placeholder agent session: shows attached blocks. Model calls arrive in M2. */
export function AgentTab(props: { focused: boolean; onFocus: () => void }) {
  return (
    // biome-ignore lint/a11y/noStaticElementInteractions: mouse focus follows click; keyboard focus moves with pane shortcuts.
    <div
      class="tf-agent"
      data-focused={props.focused ? "1" : "0"}
      onMouseDown={() => props.onFocus()}
    >
      <header>
        <h2>{t("agent.title")}</h2>
        <p class="tf-muted">{t("agent.placeholder")}</p>
      </header>
      <Show when={attachments().length > 0} fallback={<p class="tf-muted">{t("agent.empty")}</p>}>
        <ul class="tf-attachments" aria-label={t("agent.attached")}>
          <For each={attachments()}>
            {(a) => (
              <li>
                <div class="tf-attachment-head">
                  <span class="tf-badge" data-ok={a.block.exit_code === 0 ? "1" : "0"}>
                    {a.block.exit_code === 0 ? "✓" : "✗"} {a.block.exit_code ?? "?"}
                  </span>
                  <code>{a.block.command}</code>
                  <button
                    type="button"
                    aria-label={t("agent.detach")}
                    title={t("agent.detach")}
                    onClick={() => detach(a.block.id)}
                  >
                    <Icon name="close" size={12} />
                  </button>
                </div>
                <pre>{a.markdown}</pre>
              </li>
            )}
          </For>
        </ul>
      </Show>
    </div>
  );
}
