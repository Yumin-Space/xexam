// Thinking strength (v1.6): the level a model uses when nothing is sent (only values verified in the official docs,
// 2026-11-16) and the level kept when the family button changes.

const TABLE: Record<string, string> = {
  'claude-opus-5-5': 'medium', 'claude-haiku-5-5': 'medium',
  'claude-sonnet-5-5': 'high', 'claude-opus-5': 'high', 'claude-sonnet-5': 'high', 'claude-opus-4-8': 'high',
  'claude-opus-4-7': 'high', 'claude-opus-4-6': 'high', 'claude-sonnet-4-6': 'high',
  'gpt-6.1-sol': 'medium', 'gpt-6.1-terra': 'medium', 'gpt-6-sol': 'medium', 'gpt-6-luna': 'medium', 'gpt-5.6': 'medium',
  'gpt-5.5': 'medium',
  'gpt-5.4': 'none', 'gpt-5.2': 'none', 'gpt-5.1': 'none',
  'gemini-4-hydrogen': 'HIGH', 'gemini-3.1-pro-preview': 'HIGH', 'gemini-3-flash-preview': 'HIGH', 'gemini-3-pro-preview': 'HIGH',
  'gemini-4-carbon': 'MEDIUM', 'gemini-4-argon': 'MEDIUM', 'gemini-3.8-flash': 'MEDIUM', 'gemini-3.6-flash': 'MEDIUM',
  'gemini-3.5-flash-lite': 'MINIMAL', 'gemini-3.1-flash-lite': 'MINIMAL',
  // Gemini 2.5 Flash-Lite does not think unless asked: shown as "none" (not a Google button level)
  'gemini-2.5-flash-lite': 'none',
};

// deepseek model ids (Responses API family): effort high unless set (https://api-docs.deepseek.com/guides/thinking_mode/)
const DEEPSEEK_DEFAULT = 'high';

// Gemma thinks by default (observed live, 2026-11-16): thinking_level HIGH
const GEMMA_DEFAULT = 'HIGH';

const ROUTING = /^(us|eu|apac|global|anthropic|google|openai)\./;

/** The model id without routing prefixes, version suffixes and date stamps */
function normalize(model: string): string {
  let m = model.toLowerCase();
  m = m.slice(m.lastIndexOf('/') + 1);
  while (ROUTING.test(m)) m = m.replace(ROUTING, '');
  m = m.split(/[@:]/)[0] ?? '';
  m = m.replace(/-v\d+$/, '').replace(/-\d{4}-\d{2}-\d{2}$/, '').replace(/-\d{8}$/, '');
  if (m.startsWith('claude-')) m = m.replace(/\./g, '-');
  return m;
}

/** The level a model uses when nothing is sent, or null when it is not known (exact lookup, no prefix matching) */
export function knownDefault(model: string | null | undefined): string | null {
  if (!model) return null;
  const k = normalize(model);
  if (k.includes('deepseek')) return DEEPSEEK_DEFAULT;
  if (k.includes('gemma')) return GEMMA_DEFAULT;
  return Object.prototype.hasOwnProperty.call(TABLE, k) ? TABLE[k] ?? null : null;
}

/**
 * The level after a family button: the current one when the new family has it (case-insensitive: "high" and "HIGH" are the
 * same level), else the model's known default when the family has it (a hint such as "none" is never picked when the family
 * does not offer it), else the family's middle level ("medium" / "MEDIUM"), or "high" / "HIGH" when that is missing
 * (Responses API, Gemma), or the first level
 */
export function levelForFamily(
  current: string | null | undefined, levels: readonly string[], modelDefault?: string | null,
): string {
  const find = (x: string): string | undefined => levels.find((l) => l.toLowerCase() === x.toLowerCase());
  return (current != null ? find(current) : undefined)
    ?? (modelDefault != null ? find(modelDefault) : undefined)
    ?? find('medium') ?? find('high') ?? levels[0] ?? 'high';
}
