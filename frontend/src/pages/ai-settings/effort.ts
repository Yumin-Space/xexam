// AI settings: what a model row's strength list shows (v2.1) and the master-switch state of a row, as plain data.
import type { AiEffortFamily, AiSettings, AiSlot } from '../../types/api.ts';

export interface EffortView { family: AiEffortFamily; level: string; levels: string[] }

/** The selected value of the dropdown that means "Default" (no strength parameter sent) */
export const EFFORT_DEFAULT = '';

/**
 * v2.2: the family of the connection the row uses, the stored level when it belongs to that family (else '' = Default,
 * nothing sent: an untouched row is Default) and the family's levels; null when the row does not resolve or no lists came.
 * The dropdown shows Default first, then `levels`.
 */
export function effortView(
  s: Pick<AiSettings, 'slots' | 'effortLevels' | 'pool' | 'connections'>, slot: AiSlot,
): EffortView | null {
  const lists = s.effortLevels;
  // v2.2: the server names the family itself (also for a row whose connection is switched off); older answers fall back
  // to using -> pool -> connection
  let family: AiEffortFamily | null | undefined = s.slots[slot].effortFamily;
  if (!family) {
    const using = s.slots[slot].using;
    const pooled = using ? s.pool.find((m) => m.id === using.modelId) : undefined;
    family = pooled ? s.connections.find((c) => c.id === pooled.connectionId)?.effortFamily : undefined;
  }
  const levels = family ? lists?.[family] : undefined;
  if (!family || !levels) return null;
  const stored = s.slots[slot].effort;
  const level = stored && stored.family === family && levels.includes(stored.level) ? stored.level : EFFORT_DEFAULT;
  return { family, level, levels };
}

/** The level to post for a dropdown value: '' (Default) posts null */
export const effortPost = (value: string): string | null => (value === EFFORT_DEFAULT ? null : value);

/** v2.1 master switch: while AI+ is off every row is greyed and shows its feature switch off (stored values stay) */
export const rowPaused = (s: Pick<AiSettings, 'enabled'>): boolean => !s.enabled;
