// AI settings: the thinking-strength family of the connection form (v2.1). A new connection's family follows the format and
// the address until it is chosen by hand; a saved connection starts on its stored family, following the format again unless it differs from the inferred one.
import type { AiEffortFamily, AiFormat, AiProvider } from '../../types/api.ts';

/** What the family should be for a provider, format and address (anthropic and responses follow the format, a Google address wins over openai) */
export function inferFamily(provider: AiProvider, format: AiFormat, baseUrl: string): AiEffortFamily {
  if (provider === 'anthropic') return 'anthropic';
  if (provider === 'gateway') {
    if (format === 'anthropic') return 'anthropic';
    if (format === 'responses') return 'responses';
  }
  return /generativelanguage|google|gemini/i.test(baseUrl) ? 'google' : 'openai';
}

export interface FamilyChoice { value: AiEffortFamily; byHand: boolean }

/** The starting choice of a saved connection: a family equal to what the form would infer keeps following it; a different
 *  one (the upgrade read the model names, or an admin chose it) counts as chosen by hand */
export function savedFamily(stored: AiEffortFamily, provider: AiProvider, format: AiFormat, baseUrl: string): FamilyChoice {
  return { value: stored, byHand: stored !== inferFamily(provider, format, baseUrl) };
}

/** The family after the form changed: kept when chosen by hand, else re-inferred */
export function followFamily(cur: FamilyChoice, provider: AiProvider, format: AiFormat, baseUrl: string): FamilyChoice {
  return cur.byHand ? cur : { value: inferFamily(provider, format, baseUrl), byHand: false };
}
