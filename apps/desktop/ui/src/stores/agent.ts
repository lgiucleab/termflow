// Blocks attached to the (M1: empty) agent session. M2 turns these into context for the model.
import { createSignal } from "solid-js";
import type { Block } from "../bridge";

export interface Attachment {
  block: Block;
  markdown: string;
}

const [attachments, setAttachments] = createSignal<Attachment[]>([]);

export { attachments };

export function attach(a: Attachment): void {
  setAttachments((xs) => (xs.some((x) => x.block.id === a.block.id) ? xs : [...xs, a]));
}

export function detach(blockId: string): void {
  setAttachments((xs) => xs.filter((x) => x.block.id !== blockId));
}
