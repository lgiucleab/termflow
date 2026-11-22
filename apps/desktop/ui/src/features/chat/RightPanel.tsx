// Right side panel (M2d): the agent's todo list and what's attached to the next message.
// M4 adds a Review tab here.
import { For, type JSX, Show } from "solid-js";
import { Icon } from "../../components/Icon";
import { t } from "../../i18n";
import { type RightTab, rightTab, setRightOpen, setRightTab } from "../../stores/layout";
import { type Todo, TodoList } from "./ToolCard";

export interface ContextItem {
  key: string;
  label: string;
  detail?: string;
  onRemove: () => void;
}

const TABS: RightTab[] = ["todo", "context"];

export function RightPanel(props: { todos: Todo[]; context: ContextItem[] }): JSX.Element {
  return (
    <aside class="tf-right" aria-label={t("layout.toggleRight")}>
      <div class="tf-right-tabs" role="tablist">
        <For each={TABS}>
          {(tab) => (
            <button
              type="button"
              role="tab"
              class="tf-tab"
              aria-selected={rightTab() === tab}
              onClick={() => setRightTab(tab)}
            >
              {t(`layout.${tab}`)}
              <Show when={tab === "todo" ? props.todos.length : props.context.length}>
                {(n) => <span class="tf-count">{n()}</span>}
              </Show>
            </button>
          )}
        </For>
        <span class="tf-spacer" />
        <button
          type="button"
          class="tf-icon-btn"
          aria-label={t("common.close")}
          onClick={() => setRightOpen(false)}
        >
          <Icon name="close" size={13} />
        </button>
      </div>
      <div class="tf-right-body" role="tabpanel">
        <Show when={rightTab() === "todo"}>
          <Show when={props.todos.length} fallback={<p class="tf-empty">{t("layout.noTodos")}</p>}>
            <TodoList todos={props.todos} />
          </Show>
        </Show>
        <Show when={rightTab() === "context"}>
          <Show
            when={props.context.length}
            fallback={<p class="tf-empty">{t("layout.noContext")}</p>}
          >
            <ul class="tf-context">
              <For each={props.context}>
                {(c) => (
                  <li class="tf-row">
                    <Icon name="terminal" size={13} />
                    <code>{c.label}</code>
                    <Show when={c.detail}>
                      <span class="tf-faint">{c.detail}</span>
                    </Show>
                    <button
                      type="button"
                      class="tf-icon-btn"
                      aria-label={t("agent.detach")}
                      onClick={c.onRemove}
                    >
                      <Icon name="close" size={11} />
                    </button>
                  </li>
                )}
              </For>
            </ul>
          </Show>
        </Show>
      </div>
    </aside>
  );
}
