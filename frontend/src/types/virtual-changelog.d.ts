// Generated at build time by the xexam-changelog plugin in vite.config.ts from the three changelogs (docs/<lang>/changelog.md).
declare module 'virtual:changelog' {
  import type { ChangelogEntry } from '../domain/changelog.ts';

  const data: Record<'en' | 'zh' | 'ja', ChangelogEntry[]>;
  export default data;
}
