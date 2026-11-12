// Browser local storage key names. Since v0.2 everything uses the xexam.* prefix.
// Since v0.6 the unfinished exam and the settings cache belong to the signed-in user: their keys carry the user id, so
// another user signing in on the same browser never sees them. The owner is set when a user signs in (setStorageOwner).
export const paths = {
  /** The unfinished exam before v0.6 (one user); moved once to the first administrator who signs in on this browser */
  legacySession: 'xexam.session.v1',
  /** Before v0.5 the question language lived only here; now it is a server-side setting (read once for migration) */
  lang: 'xexam.lang.v1',
  /** First-paint cache of the server-side settings before v0.6 (one user); still read for the sign-in page */
  legacySettings: 'xexam.settings.v1',
  /** Id of the user who signed in last on this browser: their cached settings paint the page before the sign-in is known */
  lastUser: 'xexam.lastUser.v1',
  /**
   * sessionStorage: the language picked on the sign-in page, carried across the page reload that follows a sign-in after
   * someone else's sign-out (read and removed once at startup)
   */
  pendingLang: 'xexam.pendingLang.v1',
} as const;

/** The unfinished exam of one user */
export const sessionKey = (userId: number): string => `xexam.u${userId}.session.v1`;
/** Whether the answer sheet of the practice / exam page is expanded on phones (QZ-5): per user and browser */
export const sheetOpenKey = (userId: number): string => `xexam.u${userId}.sheetOpen.v1`;
/** First-paint cache of one user's settings */
export const settingsKey = (userId: number): string => `xexam.u${userId}.settings.v1`;

let owner: number | null = null;

/** The user whose browser data (unfinished exam, settings cache) is read and written; null = nobody signed in */
export function storageOwner(): number | null {
  return owner;
}

export function setStorageOwner(userId: number | null): void {
  owner = userId;
}
