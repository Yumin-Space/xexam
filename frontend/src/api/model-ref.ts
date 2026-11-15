// A model of one connection as text and back (v1.3): `<connectionId>:<model>`. Model names may contain colons, so the text
// splits at the FIRST colon only. Shared by AI Chat (the picker) and AI settings.
import type { ModelRef } from '../types/api.ts';

/** `{connectionId: 2, model: 'demo:small'}` -> `"2:demo:small"` */
export const modelRefText = (ref: ModelRef): string => `${ref.connectionId}:${ref.model}`;

/** The inverse of modelRefText; null when the text is not `<positive integer>:<non-empty model>` */
export function parseModelRef(text: string): ModelRef | null {
  const i = text.indexOf(':');
  if (i <= 0) return null;
  const id = text.slice(0, i);
  const model = text.slice(i + 1);
  if (!/^[1-9]\d{0,14}$/.test(id) || model === '') return null;
  return { connectionId: Number(id), model };
}
