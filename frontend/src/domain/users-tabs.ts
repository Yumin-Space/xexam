// The two tabs of User management (v1.4), built like the AI settings category tabs: the hash keeps the tab
// (#/users?tab=users | signin); a missing or unknown value is the first tab. No DOM, so it runs under `node --test`.

/** The tabs, in the order of the row: the user list, then the sign-in IP and location panel */
export const USER_TABS = ['users', 'signin'] as const;
export type UserTab = (typeof USER_TABS)[number];

/** The tab a location hash names (`#/users?tab=signin`); a missing or unknown value is the first tab */
export function userTabOfHash(hash: string): UserTab {
  const q = hash.indexOf('?');
  if (q < 0) return 'users';
  const v = new URLSearchParams(hash.slice(q + 1)).get('tab');
  return (USER_TABS as readonly string[]).includes(v ?? '') ? (v as UserTab) : 'users';
}

/** The hash of a tab: always `#/users?tab=<key>` */
export const hashOfUserTab = (tab: UserTab): string => `#/users?tab=${tab}`;
