// Hash routing: #/ (home), #/banks, #/users, #/records, #/records/<id>, #/records/wrong, #/settings, #/about, #/quiz, #/result.
// Hash routes need no server-side fallback and survive a page reload.

export type Route =
  | { name: 'home' }
  | { name: 'banks' }
  | { name: 'users' }
  | { name: 'records' }
  | { name: 'record'; id: number }
  | { name: 'wrong' }
  | { name: 'settings' }
  | { name: 'about' }
  | { name: 'quiz' }
  | { name: 'result' };

/** The menu entry a route belongs to */
export type NavSection = 'home' | 'banks' | 'records' | 'users' | 'settings' | 'about';

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
    case 'records':
    case 'record':
    case 'wrong':
    case 'result':
      return 'records';
    case 'settings':
      return 'settings';
    case 'about':
      return 'about';
    default:
      return null;
  }
}

export function sameRoute(a: Route, b: Route): boolean {
  return routeHash(a) === routeHash(b);
}

/** Pages only administrators may open (the backend refuses their requests to everyone else) */
export function adminOnly(r: Route): boolean {
  return r.name === 'banks' || r.name === 'users';
}
