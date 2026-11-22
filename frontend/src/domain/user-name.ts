// v2.6: the name a person is shown by. The login (users.username) is an e-mail address; the display name (用户名) is chosen
// by the user at the first sign-in and may be changed on the Profile page. Until it is set, the login stands in.

/** The fields of a user that decide the shown name */
export interface NamedUser {
  username: string;
  displayName?: string | null;
}

/** The display name when set, else the login */
export function shownName(u: NamedUser): string {
  const d = (u.displayName ?? '').trim();
  return d || u.username;
}

/** Maximum length of a display name in characters (Unicode scalar values, as the server counts them) */
export const DISPLAY_NAME_MAX = 20;

/** The display name as the server stores it: trimmed, inner whitespace runs collapsed to one space */
export function cleanDisplayName(raw: string): string {
  return raw.replace(/\s+/g, ' ').trim();
}

/** Why a display name is refused (mirror of the server rule), or null when it is fine */
export function displayNameProblem(raw: string): 'empty' | 'bad' | null {
  const v = cleanDisplayName(raw);
  if (!v) return 'empty';
  // the invisible characters are looked for in the raw text too: JS trim() drops a leading / trailing U+FEFF silently
  // eslint-disable-next-line no-control-regex
  if (raw.includes('\ufeff') || [...v].length > DISPLAY_NAME_MAX || v.includes('@') || /[\u0000-\u001f\u007f-\u0084\u0086-\u009f\u00ad\u061c\u180e\u200b-\u200f\u202a-\u202e\u2060-\u2064\u2066-\u206f\ufeff\ufff9-\ufffb]/.test(v)) return 'bad';
  return null;
}

/** v2.6: the session must finish setup first (a new password and / or a display name) before anything else is reachable */
export function setupRequired(u: { mustChangePassword: boolean; needsName?: boolean } | null | undefined): boolean {
  return !!u && (u.mustChangePassword || u.needsName === true);
}

/**
 * What saving the Profile name field would do: refuse (`empty`, `bad`), do nothing because it is the name already set
 * (`same`, compared after cleaning, case-sensitively: a change of case is a change), or `ok`.
 */
export function nameSave(current: string | null | undefined, draft: string): 'empty' | 'bad' | 'same' | 'ok' {
  const p = displayNameProblem(draft);
  if (p) return p;
  return cleanDisplayName(draft) === (current ?? '') ? 'same' : 'ok';
}

/**
 * The page a freshly signed-in user must pass before the app: the new-password page first (it also asks for the username
 * while there is none), else the small "set a username" page, else null (the app itself).
 */
export function setupView(u: { mustChangePassword: boolean; needsName?: boolean } | null | undefined): 'password' | 'name' | null {
  if (!u) return null;
  return u.mustChangePassword ? 'password' : u.needsName === true ? 'name' : null;
}

/** A login that looks like an e-mail address (required for new or changed logins in User management) */
export function looksLikeEmail(v: string): boolean {
  return /^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(v.trim());
}

/** What the sign-in form accepts: an e-mail address, or the default administrator `admin` of a fresh install */
export function signInNameOk(v: string): boolean {
  const s = v.trim();
  return looksLikeEmail(s) || s.toLowerCase() === 'admin';
}
