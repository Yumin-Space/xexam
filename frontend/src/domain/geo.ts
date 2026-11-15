// Sign-in location text (v1.3, v1.4: country may be missing, shortPlace for log rows): "city, region, country". The country name comes from Intl.DisplayNames in the UI language,
// with the 2-letter code as the fallback; missing parts are left out. Pure (the name lookup is injectable for tests).
import type { GeoLocation } from '../types/api.ts';

export type CountryName = (code: string) => string | null;

/** The country name of a 2-letter code in `lang`, or null when the runtime has none */
export function countryNameIn(lang: string): CountryName {
  let names: Intl.DisplayNames | null = null;
  try {
    names = new Intl.DisplayNames([lang], { type: 'region' });
  } catch {
    names = null;
  }
  return (code) => {
    try {
      const n = names?.of(code);
      return n && n !== code ? n : null;
    } catch {
      return null;
    }
  };
}

/** `city, region, country`; null when there is no location at all (the page then shows "Location unknown") */
export function placeText(loc: GeoLocation | null | undefined, name: CountryName): string | null {
  if (!loc) return null;
  const country = loc.country ? (name(loc.country) ?? loc.country) : null;
  const parts = [loc.city, loc.region, country].map((p) => (p ?? '').trim()).filter((p) => p !== '');
  return parts.length ? parts.join(', ') : null;
}

/** The short place of a log row (v1.4): the city, else the region, else the country name (a raw non-code country text is shown as it is); null when none */
export function shortPlace(loc: GeoLocation | null | undefined, name: CountryName): string | null {
  if (!loc) return null;
  const city = (loc.city ?? '').trim();
  if (city) return city;
  const region = (loc.region ?? '').trim();
  if (region) return region;
  const country = (loc.country ?? '').trim();
  if (!country) return null;
  return /^[A-Za-z]{2}$/.test(country) ? (name(country.toUpperCase()) ?? country) : country;
}
