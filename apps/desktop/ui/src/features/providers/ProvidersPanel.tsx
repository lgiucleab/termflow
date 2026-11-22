import { createSignal, For, onMount, Show } from "solid-js";
import type { Credential } from "../../bridge";
import { useBridge } from "../../bridge/context";
import { t } from "../../i18n";
import { credentials, refreshCredentials } from "../../stores/providers";

type Kind = "anthropic" | "openai" | "openrouter" | "ollama" | "custom";
type Wire = "anthropic_messages" | "openai_responses" | "openai_chat" | "ollama";
const KINDS: Kind[] = ["anthropic", "openai", "openrouter", "ollama", "custom"];
const WIRES: Wire[] = ["openai_chat", "openai_responses", "anthropic_messages", "ollama"];

/** Providers page of Settings: credentials list + add form. */
export function ProvidersPanel() {
  const bridge = useBridge();
  const [kind, setKind] = createSignal<Kind>("anthropic");
  const [label, setLabel] = createSignal("");
  const [baseUrl, setBaseUrl] = createSignal("");
  const [wire, setWire] = createSignal<Wire>("openai_chat");
  const [key, setKey] = createSignal("");
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);
  const [status, setStatus] = createSignal<Record<string, string>>({});

  onMount(() => void refreshCredentials(bridge).catch((e) => setError(String(e))));

  const needsKey = () => kind() !== "ollama";
  const add = async (e: Event) => {
    e.preventDefault();
    setBusy(true);
    setError(null);
    try {
      const c = await bridge.addCredential({
        kind: kind(),
        label: label(),
        base_url: baseUrl() || null,
        wire: kind() === "custom" ? wire() : null,
        api_key: key() || null,
      });
      setKey(""); // never keep the key in the UI
      setLabel("");
      setBaseUrl("");
      await refreshCredentials(bridge);
      await test(c);
    } catch (err) {
      setError(String(err));
    } finally {
      setBusy(false);
    }
  };

  const test = async (c: Credential) => {
    setStatus((s) => ({ ...s, [c.id]: t("providers.testing") }));
    try {
      const models = await bridge.listModels(c.id);
      setStatus((s) => ({
        ...s,
        [c.id]: `✓ ${t("providers.models", { n: String(models.length) })}`,
      }));
    } catch (err) {
      setStatus((s) => ({ ...s, [c.id]: `✗ ${String(err)}` }));
    }
  };

  const remove = async (c: Credential) => {
    await bridge.deleteCredential(c.id);
    await refreshCredentials(bridge);
  };

  return (
    <section class="tf-settings-section" aria-labelledby="tf-providers-title">
      <h2 id="tf-providers-title">{t("providers.title")}</h2>
      <p>{t("providers.privacy")}</p>
      <ul class="tf-cred-list">
        <For each={credentials()} fallback={<li class="tf-muted">{t("providers.none")}</li>}>
          {(c) => (
            <li>
              <div>
                <strong>{c.label}</strong>
                <span class="tf-muted">
                  {" "}
                  · {t(`providers.kind.${c.kind}`)} · {c.base_url}
                </span>
                <Show when={status()[c.id]}>
                  <div class="tf-cred-status" data-ok={status()[c.id]?.startsWith("✓") ? "1" : "0"}>
                    {status()[c.id]}
                  </div>
                </Show>
              </div>
              <div class="tf-cred-actions">
                <button type="button" onClick={() => void test(c)}>
                  {t("providers.test")}
                </button>
                <button type="button" onClick={() => void remove(c)}>
                  {t("providers.remove")}
                </button>
              </div>
            </li>
          )}
        </For>
      </ul>
      <form class="tf-form" onSubmit={add}>
        <h3>{t("providers.add")}</h3>
        <label>
          {t("providers.provider")}
          <select value={kind()} onChange={(e) => setKind(e.currentTarget.value as Kind)}>
            <For each={KINDS}>{(k) => <option value={k}>{t(`providers.kind.${k}`)}</option>}</For>
          </select>
        </label>
        <label>
          {t("providers.label")}
          <input
            value={label()}
            placeholder={t("providers.labelHint")}
            onInput={(e) => setLabel(e.currentTarget.value)}
          />
        </label>
        <label>
          {kind() === "custom" ? t("providers.baseUrl") : t("providers.baseUrlOptional")}
          <input
            value={baseUrl()}
            required={kind() === "custom"}
            placeholder={
              kind() === "custom" ? "http://localhost:1234/v1" : t("providers.defaultUrl")
            }
            onInput={(e) => setBaseUrl(e.currentTarget.value)}
          />
        </label>
        <Show when={kind() === "custom"}>
          <label>
            {t("providers.wire")}
            <select value={wire()} onChange={(e) => setWire(e.currentTarget.value as Wire)}>
              <For each={WIRES}>
                {(w) => <option value={w}>{t(`providers.wireName.${w}`)}</option>}
              </For>
            </select>
          </label>
        </Show>
        <Show when={needsKey()}>
          <label>
            {t("providers.apiKey")}
            <input
              type="password"
              autocomplete="off"
              value={key()}
              required={kind() !== "custom"}
              onInput={(e) => setKey(e.currentTarget.value)}
            />
          </label>
        </Show>
        <Show when={error()}>
          <p class="tf-error" role="alert">
            {error()}
          </p>
        </Show>
        <div class="tf-dialog-actions">
          <button type="submit" class="tf-primary" disabled={busy()}>
            {busy() ? t("providers.adding") : t("providers.addButton")}
          </button>
        </div>
      </form>
    </section>
  );
}
