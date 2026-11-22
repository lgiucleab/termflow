import * as i18n from "@solid-primitives/i18n";
import { createMemo, createSignal } from "solid-js";
import { type Dict, en } from "./en";
import { esES } from "./es-ES";

export type Locale = "en" | "es-ES";
const dicts: Record<Locale, Dict> = { en, "es-ES": esES };

const initial: Locale = navigator.language.toLowerCase().startsWith("es") ? "es-ES" : "en";
const [locale, setLocale] = createSignal<Locale>(initial);

export { locale, setLocale };

const dict = createMemo(() => i18n.flatten(dicts[locale()]));
export const t = i18n.translator(dict, i18n.resolveTemplate);
