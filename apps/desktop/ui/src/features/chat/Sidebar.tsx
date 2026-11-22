import { createSignal, For, Show } from "solid-js";
import { useBridge } from "../../bridge/context";
import { Icon } from "../../components/Icon";
import { t } from "../../i18n";
import { isMac, shortcutLabel } from "../../keybindings";
import { activeChat, busy, chats, refreshChats, setActiveChat } from "../../stores/project";
import { ago } from "./time";

export function Sidebar(props: { onNewChat: () => void }) {
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
      <div class="tf-sidebar-head">
        <span>{t("chats.label")}</span>
        <button
          type="button"
          class="tf-icon-btn"
          aria-label={t("chats.new")}
          title={`${t("chats.new")} (${shortcutLabel("newChat", isMac())})`}
          onClick={props.onNewChat}
        >
          <Icon name="plus" size={14} />
        </button>
      </div>
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
                    <span class="tf-chat-title">
                      <Show when={busy().has(c.id)}>
                        <span class="tf-dot" title={t("chats.working")} />
                        <span class="tf-sr">{t("chats.working")}</span>
                      </Show>
                      <span class="tf-ellipsis">{c.title ?? t("chats.untitled")}</span>
                    </span>
                    <span class="tf-chat-meta">
                      {c.mode === "plan" ? t("agent.plan") : t("agent.build")}
                      {c.model ? ` · ${c.model}` : ""} · {ago(c.updated_at)}
                    </span>
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
