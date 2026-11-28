// Time zones for Profile > Time zone (v3.1): the device's zone, the list of IANA zones and their UTC offsets. Pure apart from
// reading `Intl`; every use of a newer `Intl` feature is feature-checked (the main tsconfig lib is ES2020).

/** The device's own IANA zone, or null when the browser does not say */
export function deviceZone(): string | null {
  try {
    const z = new Intl.DateTimeFormat().resolvedOptions().timeZone;
    return typeof z === 'string' && z !== '' ? z : null;
  } catch {
    return null;
  }
}

/** Every IANA zone the browser knows (`Intl.supportedValuesOf`), else just UTC and the device's zone */
export function allZones(): string[] {
  const sv = (Intl as unknown as { supportedValuesOf?: (key: string) => string[] }).supportedValuesOf;
  if (typeof sv === 'function') {
    try {
      const names = sv.call(Intl, 'timeZone');
      if (Array.isArray(names) && names.length > 0) return [...names];
    } catch {
      // fall through to the short list
    }
  }
  const own = deviceZone();
  return own && own !== 'UTC' ? ['UTC', own] : ['UTC'];
}

/** The offset of `zone` from UTC in minutes at `at` (default now); null when the browser does not know the zone */
export function zoneOffsetMinutes(zone: string, at: Date = new Date()): number | null {
  try {
    const parts = new Intl.DateTimeFormat('en-US', {
      timeZone: zone, hourCycle: 'h23', year: 'numeric', month: 'numeric', day: 'numeric', hour: 'numeric', minute: 'numeric', second: 'numeric',
    }).formatToParts(at);
    const get = (type: string): number => Number(parts.find((p) => p.type === type)?.value);
    const hour = get('hour') % 24; // some engines write midnight as 24
    const wall = Date.UTC(get('year'), get('month') - 1, get('day'), hour, get('minute'), get('second'));
    if (!Number.isFinite(wall)) return null;
    const real = Math.floor(at.getTime() / 1000) * 1000;
    return Math.round((wall - real) / 60000);
  } catch {
    return null;
  }
}

/** `UTC+05:30`, `UTC-03:00`, `UTC+00:00` */
export function offsetLabel(minutes: number): string {
  const sign = minutes < 0 ? '-' : '+';
  const abs = Math.abs(minutes);
  const hh = String(Math.floor(abs / 60)).padStart(2, '0');
  const mm = String(abs % 60).padStart(2, '0');
  return `UTC${sign}${hh}:${mm}`;
}

/**
 * The options of the zone picker: `(UTC+01:00) Europe/Berlin` style labels, sorted by offset and then by name. `UTC` is always
 * there (offset 0); a name the browser cannot give an offset for is dropped; a repeated name appears once.
 */
export function zoneOptions(names: readonly string[], offsetMinutes: (zone: string) => number | null): Array<{ value: string; label: string }> {
  const seen = new Set<string>();
  const rows: Array<{ value: string; offset: number }> = [];
  for (const name of ['UTC', ...names]) {
    if (seen.has(name)) continue;
    seen.add(name);
    const off = name === 'UTC' ? 0 : offsetMinutes(name);
    if (off == null || !Number.isFinite(off)) continue;
    rows.push({ value: name, offset: off });
  }
  rows.sort((a, b) => a.offset - b.offset || (a.value < b.value ? -1 : a.value > b.value ? 1 : 0));
  return rows.map((r) => ({ value: r.value, label: `(${offsetLabel(r.offset)}) ${r.value.replace(/_/g, ' ')}` }));
}
