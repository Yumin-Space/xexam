// The banner of a result that was not saved (v1.3, IMPROVE A1): its state machine. Pure; the actions drive it.

export type UnsavedState = 'unsaved' | 'saving' | 'saved';
export type UnsavedEvent = 'retry' | 'ok' | 'fail';

/** unsaved --retry--> saving --ok--> saved; saving --fail--> unsaved; any other event changes nothing (a double tap, a late answer) */
export function nextUnsaved(s: UnsavedState, ev: UnsavedEvent): UnsavedState {
  if (s === 'unsaved' && ev === 'retry') return 'saving';
  if (s === 'saving' && ev === 'ok') return 'saved';
  if (s === 'saving' && ev === 'fail') return 'unsaved';
  return s;
}
