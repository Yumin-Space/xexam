// Thinking strength (v1.6): the level a model uses when nothing is sent (only values verified in the official docs,
// 2026-11-16) and the level kept when the family button changes.

const TABLE: Record<string, string> = {
  'claude-opus-5-5': 'medium', 'claude-haiku-5-5': 'medium',
  'claude-sonnet-5-5': 'high', 'claude-opus-5': 'high', 'claude-sonnet-5': 'high', 'claude-opus-4-8': 'high',
  'claude-opus-4-7': 'high', 'claude-opus-4-6': 'high', 'claude-sonnet-4-6': 'high',
  'gpt-6.1-sol': 'medium', 'gpt-6.1-terra': 'medium', 'gpt-6-sol': 'medium', 'gpt-6-luna': 'medium', 'gpt-5.6': 'medium',
  'gpt-5.5': 'medium',
  'gpt-5.4': 'none', 'gpt-5.2': 'none', 'gpt-5.1': 'none',
  'gemini-4-hydrogen': 'high', 'gemini-3.1-pro-preview': 'high', 'gemini-3-flash-preview': 'high', 'gemini-3-pro-preview': 'high',
  'gemini-4-carbon': 'medium', 'gemini-4-argon': 'medium', 'gemini-3.8-flash': 'medium', 'gemini-3.6-flash': 'medium',
  'gemini-3.5-flash-lite': 'minimal', 'gemini-3.1-flash-lite': 'minimal',
  'gemini-2.5-flash-lite': 'none',
};

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
  return Object.prototype.hasOwnProperty.call(TABLE, k) ? TABLE[k] ?? null : null;
}

/** The level after a family button: the current one when the new family has it, else "medium" (in all three lists) */
export function levelForFamily(current: string | null | undefined, levels: readonly string[]): string {
  return current != null && levels.includes(current) ? current : 'medium';
}
