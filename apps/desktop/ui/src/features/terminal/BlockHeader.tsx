import { Show } from "solid-js";
import { Icon } from "../../components/Icon";
import { t } from "../../i18n";
import type { BlockUi, TerminalController } from "./controller";
import { blockDurationMs, formatDuration } from "./markdown";

function shortPath(p: string | null): string {
  if (!p) return "";
  const parts = p.split(/[\\/]/).filter(Boolean);
  return parts.length <= 2 ? p : `…/${parts.slice(-2).join("/")}`;
}

export function BlockHeader(props: { ui: BlockUi; controller: TerminalController }) {
  const b = props.ui.block;
  const ok = b.exit_code === 0;
  const dur = blockDurationMs(b);
  const c = props.controller;
  const btn = (label: string, icon: string, run: () => void) => (
    <button
      type="button"
      class="tf-block-btn"
      aria-label={label}
      title={label}
      onMouseDown={(e) => e.stopPropagation()}
      onClick={(e) => {
        e.stopPropagation();
        run();
      }}
    >
      <Icon name={icon} size={12} />
    </button>
  );
  return (
    <div class="tf-block-row" data-ok={ok ? "1" : "0"}>
      <span
        class="tf-badge"
        data-ok={ok ? "1" : "0"}
        title={t("blocks.exitCode", { code: String(b.exit_code ?? "?") })}
      >
        {ok ? "✓" : "✗"} {b.exit_code ?? "?"}
      </span>
      <span class="tf-block-cmd" title={b.command}>
        {b.command || t("blocks.unknownCommand")}
      </span>
      <Show when={dur !== null}>
        <span class="tf-block-meta">{formatDuration(dur ?? 0)}</span>
      </Show>
      <Show when={b.cwd}>
        <span class="tf-block-meta" title={b.cwd ?? ""}>
          {shortPath(b.cwd)}
        </span>
      </Show>
      <Show when={b.git_branch}>
        <span class="tf-block-meta">⎇ {b.git_branch}</span>
      </Show>
      <span class="tf-block-actions">
        {btn(t("blocks.copyCommand"), "copy", () => void c.copy(b.command, t("blocks.command")))}
        {btn(
          t("blocks.copyOutput"),
          "output",
          () => void c.blockText(b).then((o) => c.copy(o, t("blocks.output"))),
        )}
        {btn(t("blocks.copyMarkdown"), "markdown", () => void c.copyMarkdown(b))}
        {btn(t("blocks.attach"), "attach", () => void c.attach(b))}
        {btn(t("blocks.rerun"), "rerun", () => c.rerun(b))}
        {btn(
          props.ui.collapsed() ? t("blocks.expand") : t("blocks.collapse"),
          props.ui.collapsed() ? "chevronRight" : "chevronDown",
          props.ui.toggle,
        )}
      </span>
    </div>
  );
}
