import { createSignal } from "solid-js";

const [toast, setToast] = createSignal<string | null>(null);
let timer: ReturnType<typeof setTimeout> | undefined;

export { toast };

export function showToast(msg: string): void {
  setToast(msg);
  clearTimeout(timer);
  timer = setTimeout(() => setToast(null), 2500);
}
