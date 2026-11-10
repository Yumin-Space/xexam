// Browser local storage key names. Since v0.2 everything uses the xexam.* prefix.
export const paths = {
  session: 'xexam.session.v1',
  /** Before v0.5 the question language lived only here; now it is a server-side setting (read once for migration) */
  lang: 'xexam.lang.v1',
  /** First-paint cache of the server-side settings (v0.5+) */
  settings: 'xexam.settings.v1',
} as const;
