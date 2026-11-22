// v2.5: the Home dashboard's data. The last result is kept in memory, so the page paints at once and refreshes in the
// background on each visit; it never polls and is forgotten on sign-out. Nothing is written to the browser's storage.
// Each visit also reads the study records again (state/dash-visit.ts), so the newest-record time is the server's.
import { fetchDashboard, type Dashboard } from '../api/dashboard.ts';
import { fetchRecords } from '../api/progress.ts';
import { onRouteChange, render } from './actions.ts';
import { loadVisit } from './dash-visit.ts';
import { onClearUserData, S } from './store.ts';

let last: Dashboard | null = null;
let failed = false;
let inflight = false;
/** The current visit already asked (a re-render of the page must not ask again) */
let asked = false;
/** Bumped on sign-out: an answer that arrives after it belongs to the previous user and is dropped */
let epoch = 0;

export interface DashView {
  data: Dashboard | null;
  /** Nothing to show yet and the request failed */
  error: boolean;
  /** Nothing to show yet and the request is running */
  loading: boolean;
}

export const dashView = (): DashView => ({ data: last, error: failed && !last, loading: !last && !failed });

function load(): void {
  if (inflight) return;
  inflight = true;
  asked = true;
  const mine = epoch;
  void loadVisit({ dashboard: fetchDashboard, records: fetchRecords })
    .then(({ data, records }) => {
      if (mine !== epoch) return;
      if (records) {
        S.records = records.records;
        S.recordsRetention = records.retention;
      }
      // A failed refresh keeps what is already on screen
      if (data) last = data;
      failed = !data;
    })
    .finally(() => {
      inflight = false;
      if (mine === epoch && S.route.name === 'home') render();
    });
}

/** Called by every paint of the page: one request per visit */
export function ensureDashboard(): void {
  if (!asked) load();
}

/** The quiet retry button of a failed block */
export function retryDashboard(): void {
  failed = false;
  load();
  render();
}

onRouteChange((from) => {
  if (from.name === 'home') asked = false;
});
onClearUserData(() => {
  epoch++;
  last = null;
  failed = false;
  inflight = false;
  asked = false;
});
