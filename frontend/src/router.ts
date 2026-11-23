// Hash routing: #/ (dashboard), #/study (exam & practice setup), #/banks, #/users (+ #/users/<uid>/records[/<id>], v2.7), #/logs, #/records, #/records/<id>, #/records/wrong, #/settings, #/profile (v2.6), #/about, #/quiz, #/result,
// and (v0.8) #/ai (AI Chat; #/ai/<id> = that chat open, #/ai/new = a new chat not sent yet, so a reload keeps either) and
// #/ai-settings (AI settings, administrators).
// Hash routes need no server-side fallback and survive a page reload.

export type Route =
  | { name: 'home' }
  | { name: 'study' }
  | { name: 'banks' }
  | { name: 'users' }
  | { name: 'userRecords'; user: number }
  | { name: 'userRecord'; user: number; id: number }
  | { name: 'logs' }
  | { name: 'records' }
  | { name: 'record'; id: number }
  | { name: 'wrong' }
  | { name: 'settings' }
  | { name: 'profile' }
  | { name: 'about' }
  | { name: 'quiz' }
  | { name: 'result' }
  | { name: 'ai'; chatId?: number; fresh?: true }
  | { name: 'aiSettings' };

/** The menu entry a route belongs to */
export type NavSection = 'home' | 'study' | 'banks' | 'records' | 'users' | 'logs' | 'settings' | 'profile' | 'about' | 'ai' | 'aiSettings';

export function parseHash(hash: string): Route {
  const path = hash.replace(/^#\/?/, '').replace(/\/+$/, '').split('?')[0] ?? '';
  const parts = path.split('/').filter(Boolean);
  const [a, b] = parts;
  switch (a) {
    case undefined:
    case 'home':
      return { name: 'home' };
    case 'study':
      return { name: 'study' };
    case 'banks':
      return { name: 'banks' };
    case 'users': {
      // v2.7: #/users/<uid>/records and #/users/<uid>/records/<rid> (an administrator reads a user's study records)
      const [, uid, sub, rid] = parts;
      if (parts.length > 4) return { name: 'users' };
      if (uid && sub === 'records' && /^[0-9]{1,15}$/.test(uid)) {
        if (rid && /^[0-9]{1,15}$/.test(rid)) return { name: 'userRecord', user: Number(uid), id: Number(rid) };
        if (!rid) return { name: 'userRecords', user: Number(uid) };
      }
      return { name: 'users' };
    }
    case 'logs':
      return { name: 'logs' };
    case 'records':
      if (b === 'wrong') return { name: 'wrong' };
      if (b && /^[0-9]{1,15}$/.test(b)) return { name: 'record', id: Number(b) };
      return { name: 'records' };
    case 'settings':
      return { name: 'settings' };
    case 'profile':
      return { name: 'profile' };
    case 'about':
      return { name: 'about' };
    case 'quiz':
      return { name: 'quiz' };
    case 'result':
      return { name: 'result' };
    case 'ai':
      if (b && /^[0-9]{1,15}$/.test(b)) return { name: 'ai', chatId: Number(b) };
      if (b === 'new') return { name: 'ai', fresh: true };
      return { name: 'ai' };
    case 'ai-settings':
      return { name: 'aiSettings' };
    default:
      return { name: 'home' };
  }
}

export function routeHash(r: Route): string {
  switch (r.name) {
    case 'home':
      return '#/';
    case 'record':
      return `#/records/${r.id}`;
    case 'userRecords':
      return `#/users/${r.user}/records`;
    case 'userRecord':
      return `#/users/${r.user}/records/${r.id}`;
    case 'wrong':
      return '#/records/wrong';
    case 'aiSettings':
      return '#/ai-settings';
    case 'ai':
      return r.chatId != null ? `#/ai/${r.chatId}` : r.fresh ? '#/ai/new' : '#/ai';
    default:
      return `#/${r.name}`;
  }
}

export function sectionOf(r: Route): NavSection | null {
  switch (r.name) {
    case 'home':
      return 'home';
    case 'study':
      return 'study';
    case 'banks':
      return 'banks';
    case 'users':
    case 'userRecords':
    case 'userRecord':
      return 'users';
    case 'logs':
      return 'logs';
    case 'records':
    case 'record':
    case 'wrong':
    case 'result':
      return 'records';
    case 'settings':
      return 'settings';
    case 'profile':
      return 'profile';
    case 'about':
      return 'about';
    case 'ai':
      return 'ai';
    case 'aiSettings':
      return 'aiSettings';
    default:
      return null;
  }
}

export function sameRoute(a: Route, b: Route): boolean {
  return routeHash(a) === routeHash(b);
}
