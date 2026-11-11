// Hash routing: #/ (home), #/banks, #/users, #/logs, #/records, #/records/<id>, #/records/wrong, #/settings, #/about, #/quiz, #/result,
// and (v0.8) #/ai (AI Chat; #/ai/<id> = that chat open, #/ai/new = a new chat not sent yet, so a reload keeps either) and
// #/ai-settings (AI settings, administrators).
// Hash routes need no server-side fallback and survive a page reload.

export type Route =
  | { name: 'home' }
  | { name: 'banks' }
  | { name: 'users' }
  | { name: 'logs' }
  | { name: 'records' }
  | { name: 'record'; id: number }
  | { name: 'wrong' }
  | { name: 'settings' }
  | { name: 'about' }
  | { name: 'quiz' }
  | { name: 'result' }
  | { name: 'ai'; chatId?: number; fresh?: true }
  | { name: 'aiSettings' };

/** The menu entry a route belongs to */
export type NavSection = 'home' | 'banks' | 'records' | 'users' | 'logs' | 'settings' | 'about' | 'ai' | 'aiSettings';

export function parseHash(hash: string): Route {
  const path = hash.replace(/^#\/?/, '').replace(/\/+$/, '').split('?')[0] ?? '';
  const parts = path.split('/').filter(Boolean);
  const [a, b] = parts;
  switch (a) {
    case undefined:
    case 'home':
      return { name: 'home' };
    case 'banks':
      return { name: 'banks' };
    case 'users':
      return { name: 'users' };
    case 'logs':
      return { name: 'logs' };
    case 'records':
      if (b === 'wrong') return { name: 'wrong' };
      if (b && /^[0-9]{1,15}$/.test(b)) return { name: 'record', id: Number(b) };
      return { name: 'records' };
    case 'settings':
      return { name: 'settings' };
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
    case 'banks':
      return 'banks';
    case 'users':
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

/**
 * Pages only administrators may open; everyone else is sent home (the backend refuses their requests anyway). Question banks
 * is not one of them since v0.6.5: bank managers use it too, and anyone else who opens #/banks sees the "Not allowed" page
 * instead of being redirected silently (main.ts). The system log (#/logs, v0.7) is not one either: it is for administrators
 * only, but others who open it see the "Not allowed" page too. Neither are AI Chat and AI settings (v0.8): main.ts decides,
 * from the AI status and the role, between the page and the "Not allowed" page.
 */
export function adminOnly(r: Route): boolean {
  return r.name === 'users';
}
