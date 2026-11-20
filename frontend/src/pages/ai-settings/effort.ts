// AI settings: what a model row's strength list shows (v2.1) and the master-switch state of a row, as plain data.
import type { AiEffortFamily, AiSettings, AiSlot } from '../../types/api.ts';

export interface EffortView { family: AiEffortFamily; level: string; levels: string[] }

/** The family, the selected level (what is sent now) and the family's levels; null when the slot does not resolve or no lists came */
export function effortView(s: Pick<AiSettings, 'slots' | 'effortLevels'>, slot: AiSlot): EffortView | null {
  const lists = s.effortLevels;
  const eff = s.slots[slot].effortEffective ?? null;
  if (!lists || !eff || !lists[eff.family]) return null;
  return { family: eff.family, level: eff.level, levels: lists[eff.family] };
}

/** v2.1 master switch: while AI+ is off every row is greyed and shows its feature switch off (stored values stay) */
export const rowPaused = (s: Pick<AiSettings, 'enabled'>): boolean => !s.enabled;
