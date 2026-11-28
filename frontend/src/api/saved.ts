// v3.1: the server-side marker of an unfinished practice / exam (drives the reminders). Both calls are best effort.
import type { SavedReport } from '../types/api.ts';
import { postJson } from './client.ts';

/** Report a saved session. `keepalive` is for page hide: a plain fetch that survives the page, errors swallowed. */
export async function reportSaved(b: SavedReport, keepalive = false): Promise<void> {
  try {
    if (keepalive) {
      await fetch('/api/study/saved', {
        method: 'POST',
        credentials: 'same-origin',
        keepalive: true,
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(b),
      });
      return;
    }
    await postJson<unknown>('/api/study/saved', b);
  } catch {
    // reminders are a convenience; never disturb the quiz
  }
}

export async function discardSaved(key: string): Promise<void> {
  try {
    await postJson<unknown>('/api/study/saved/discard', { key });
  } catch {
    // the server drops the marker later anyway
  }
}
