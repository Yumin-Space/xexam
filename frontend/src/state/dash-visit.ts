// v2.5: what one visit of the Home dashboard reads: the dashboard data and, beside it, the study records. The records are
// read again on every visit because the greeting's "last studied" and the AI study assessment's newest-record time come
// from them: a record saved on another device or tab since the list was loaded would otherwise give a stale time, and the
// saved assessment (stored with the server's newest time) would never be found again. Pure, so it runs under node --test.

export interface VisitFetchers<D, R> {
  dashboard: () => Promise<D>;
  records: () => Promise<R>;
}

export interface VisitResult<D, R> {
  /** The dashboard data, null when its request failed */
  data: D | null;
  /** The fresh records, null when their request failed (the page keeps the list it has) */
  records: R | null;
}

/** Both requests at once; never rejects, and a failed records request never fails the dashboard */
export async function loadVisit<D, R>(f: VisitFetchers<D, R>): Promise<VisitResult<D, R>> {
  const [d, r] = await Promise.allSettled([f.dashboard(), f.records()]);
  return {
    data: d.status === 'fulfilled' ? d.value : null,
    records: r.status === 'fulfilled' ? r.value : null,
  };
}
