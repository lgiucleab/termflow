import { createSignal, For, Show } from "solid-js";
import { useBridge } from "../../bridge/context";
import { Icon } from "../../components/Icon";
import { t } from "../../i18n";
import {
  activeChat,
  busy,
  chats,
  project,
  refreshChats,
  setActiveChat,
} from "../../stores/project";

export function Sidebar(props: { onNewChat: () => void; onSwitchProject: () => void }) {
  const bridge = useBridge();
  const [editing, setEditing] = createSignal<string | null>(null);
  const [confirmDelete, setConfirmDelete] = createSignal<string | null>(null);

  const rename = async (id: string, title: string) => {
    setEditing(null);
    if (title.trim()) await bridge.renameAgentSession(id, title.trim());
    await refreshChats(bridge);
  };

  const remove = async (id: string) => {
    setConfirmDelete(null);
    await bridge.deleteAgentSession(id);
    await refreshChats(bridge);
    if (activeChat() === id) setActiveChat(chats()[0]?.id ?? null);
    if (!activeChat()) props.onNewChat();
  };

  return (
    <nav class="tf-sidebar" aria-label={t("chats.label")}>
      <div class="tf-sidebar-project">
        <span class="tf-project-name" title={project()?.path}>
          {project()?.name}
        </span>
        <button
          type="button"
          class="tf-icon-btn"
          title={t("start.switch")}
          onClick={props.onSwitchProject}
        >
          {t("start.switchShort")}
        </button>
      </div>
      <button type="button" class="tf-newchat" onClick={props.onNewChat}>
        <Icon name="plus" size={12} /> {t("chats.new")}
      </button>
      <ul class="tf-chatlist">
        <For each={chats()}>
          {(c) => (
            <li data-active={activeChat() === c.id ? "1" : "0"}>
              <Show
                when={editing() === c.id}
                fallback={
                  <button
                    type="button"
                    class="tf-chat-item"
                    onClick={() => setActiveChat(c.id)}
                    onDblClick={() => setEditing(c.id)}
                    title={t("tabs.renameHint")}
                  >
                    <Show when={busy().has(c.id)}>
                      <span class="tf-dot" title={t("chats.working")} />
                      <span class="tf-sr">{t("chats.working")}</span>
                    </Show>
                    <span>{c.title ?? t("chats.untitled")}</span>
                    <Show when={c.mode === "plan"}>
                      <span class="tf-badge-plan">{t("agent.plan")}</span>
                    </Show>
                  </button>
                }
              >
                <input
                  class="tf-tab-input"
                  value={c.title ?? ""}
                  aria-label={t("tabs.rename")}
                  ref={(el) => queueMicrotask(() => el.select())}
                  onKeyDown={(e) => {
                    if (e.key === "Enter") void rename(c.id, e.currentTarget.value);
                    if (e.key === "Escape") setEditing(null);
                  }}
                  onBlur={(e) => void rename(c.id, e.currentTarget.value)}
                />
              </Show>
              <Show
                when={confirmDelete() === c.id}
                fallback={
                  <button
                    type="button"
                    class="tf-chat-del"
                    aria-label={t("chats.delete")}
                    title={t("chats.delete")}
                    onClick={() => setConfirmDelete(c.id)}
                  >
                    <Icon name="close" size={10} />
                  </button>
                }
              >
                <button type="button" class="tf-chat-confirm" onClick={() => void remove(c.id)}>
                  {t("chats.confirmDelete")}
                </button>
              </Show>
            </li>
          )}
        </For>
      </ul>
    </nav>
  );
}
