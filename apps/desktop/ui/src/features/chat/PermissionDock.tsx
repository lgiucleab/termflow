// Permission request docked above the composer (M2d, OpenCode-style). Shows the oldest pending
// call; the rest wait their turn.
import { createSignal, Show } from "solid-js";
import type { ApprovalKind } from "../../bridge";
import { Icon } from "../../components/Icon";
import { t } from "../../i18n";
import { Diff, type Pending } from "./ToolCard";

export function PermissionDock(props: {
  pending: Record<string, Pending>;
  onDecide: (callId: string, kind: ApprovalKind, feedback: string | null) => void;
}) {
  const [feedback, setFeedback] = createSignal("");
  const [askFeedback, setAskFeedback] = createSignal(false);
  const first = () => Object.entries(props.pending)[0];
  const more = () => Object.keys(props.pending).length - 1;
  const decide = (kind: ApprovalKind, fb: string | null) => {
    const f = first();
    if (!f) return;
    setAskFeedback(false);
    setFeedback("");
    props.onDecide(f[0], kind, fb);
  };
  return (
    <Show when={first()}>
      {(f) => (
        <section class="tf-dock" role="alertdialog" aria-label={t("layout.permission")}>
          <header>
            <span class="tf-dock-icon">
              <Icon name="warn" size={14} />
            </span>
            <strong>{t("layout.permission")}</strong>
            <Show when={more() > 0}>
              <span class="tf-faint">+{more()}</span>
            </Show>
          </header>
          <p class="tf-muted">{t("layout.permissionBody")}</p>
          <code class="tf-dock-summary">{f()[1].summary}</code>
          <Show when={f()[1].preview}>{(p) => <Diff text={p()} />}</Show>
          <Show when={askFeedback()}>
            <input
              class="tf-input"
              placeholder={t("tools.feedbackHint")}
              value={feedback()}
              ref={(el) => queueMicrotask(() => el.focus())}
              onInput={(e) => setFeedback(e.currentTarget.value)}
              onKeyDown={(e) => e.key === "Enter" && decide("deny", feedback())}
            />
          </Show>
          <footer>
            <button
              type="button"
              class="tf-btn"
              data-variant="ghost"
              onClick={() => (askFeedback() ? decide("deny", feedback()) : setAskFeedback(true))}
            >
              {t("tools.denyFeedback")}
            </button>
            <button
              type="button"
              class="tf-btn"
              data-variant="ghost"
              onClick={() => decide("deny", null)}
            >
              {t("tools.deny")}
            </button>
            <span class="tf-spacer" />
            <button type="button" class="tf-btn" onClick={() => decide("allow_session", null)}>
              {t("tools.allowSession")}
            </button>
            <button
              type="button"
              class="tf-btn"
              data-variant="primary"
              ref={(el) => queueMicrotask(() => el.focus())}
              onClick={() => decide("allow_once", null)}
            >
              {t("tools.allowOnce")}
            </button>
          </footer>
        </section>
      )}
    </Show>
  );
}
