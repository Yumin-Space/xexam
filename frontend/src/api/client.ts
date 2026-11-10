// Centralized API client: all network requests go out from here, errors are normalized to ApiError, and pages only handle ApiError.
// Uses relative URLs (same origin). An expired login (the Cloudflare Access login page) shows up as "not JSON"; it is recognized here.
// No user-facing text lives here: pages turn an ApiError into text in the current UI language with errorText() (src/i18n/errors.ts).
// Sign-in (v0.6): a 401 from any route but the sign-in itself means the session is gone, and a 403 with the "choose a new
// password first" message means the password must be replaced; both are reported to the handler set with setAuthHandler()
// (state/actions.ts shows the matching page), and the request still fails with its ApiError as usual.

export type ApiErrorKind =
  | 'http' // the service returned an error status code with a JSON body; `serverMessage` holds the backend's (Chinese) reason
  | 'status' // an error status code without a JSON body (proxy error page, login page, ...)
  | 'network' // cannot connect
  | 'unexpected'; // the response is not what was expected (common cause: login expired, redirected to the login page)

export class ApiError extends Error {
  readonly kind: ApiErrorKind;
  readonly status: number;
  /** The `error` text from the backend's JSON body (kind 'http' only) */
  readonly serverMessage: string;
  /** Seconds to wait before trying again (a 429 of the login throttle: `"retryAfter"` in the body, else `Retry-After`) */
  readonly retryAfter: number | null;

  constructor(kind: ApiErrorKind, status = 0, serverMessage = '', retryAfter: number | null = null) {
    super(serverMessage || `${kind} ${status}`);
    this.name = 'ApiError';
    this.kind = kind;
    this.status = status;
    this.serverMessage = serverMessage;
    this.retryAfter = retryAfter;
  }
}

/** Whole seconds from the body's `retryAfter` or the `Retry-After` header (seconds form only); null when neither is usable */
function retryAfterOf(r: Response, body: unknown): number | null {
  const fromBody = typeof body === 'object' && body !== null ? (body as { retryAfter?: unknown }).retryAfter : undefined;
  if (typeof fromBody === 'number' && Number.isFinite(fromBody) && fromBody >= 0) return Math.ceil(fromBody);
  const header = r.headers.get('Retry-After')?.trim() ?? '';
  return /^\d{1,9}$/.test(header) ? Number(header) : null;
}

function isJson(r: Response): boolean {
  return /json/i.test(r.headers.get('Content-Type') ?? '');
}

async function parse<T>(r: Response): Promise<T> {
  if (!isJson(r)) {
    // Not JSON: an HTML login page, a proxy error page...
    if (!r.ok) throw new ApiError('status', r.status);
    throw new ApiError('unexpected', r.status);
  }
  let body: unknown;
  try {
    body = await r.json();
  } catch {
    throw new ApiError('unexpected', r.status);
  }
  if (!r.ok) {
    const msg = typeof body === 'object' && body !== null && 'error' in body && typeof (body as { error: unknown }).error === 'string'
      ? (body as { error: string }).error
      : '';
    throw msg ? new ApiError('http', r.status, msg, retryAfterOf(r, body)) : new ApiError('status', r.status);
  }
  return body as T;
}

/** The backend's MSG_PASSWORD_CHANGE_REQUIRED (backend/src/api/response.rs), sent with 403 */
export const MSG_PASSWORD_CHANGE_REQUIRED = '请先设置新密码。';

/** The route of the sign-in itself: its 401 is a wrong username or password, not a lost session */
const LOGIN_PATH = '/api/auth/login';

export type AuthProblem = 'signin' | 'password';

let authHandler: (problem: AuthProblem) => void = () => {};

/** Registers what happens when a request shows that the user must sign in (again) or must choose a new password first. */
export function setAuthHandler(fn: (problem: AuthProblem) => void): void {
  authHandler = fn;
}

/** Whether an error means "not signed in" (the session expired, was deleted or the account was disabled) */
export function isSignInError(e: unknown): boolean {
  return e instanceof ApiError && e.status === 401;
}

function authProblem(path: string, e: ApiError): AuthProblem | null {
  if (e.status === 401 && path !== LOGIN_PATH) return 'signin';
  if (e.status === 403 && e.kind === 'http' && e.serverMessage === MSG_PASSWORD_CHANGE_REQUIRED) return 'password';
  return null;
}

async function send<T>(input: string, init?: RequestInit): Promise<T> {
  let r: Response;
  try {
    // same-origin: the session cookie (HttpOnly, SameSite=Strict) goes along; never to another origin
    r = await fetch(input, { credentials: 'same-origin', ...init });
  } catch {
    throw new ApiError('network');
  }
  try {
    return await parse<T>(r);
  } catch (e) {
    const problem = e instanceof ApiError ? authProblem(input.split('?')[0] ?? input, e) : null;
    if (problem) authHandler(problem);
    throw e;
  }
}

export function getJson<T>(path: string): Promise<T> {
  return send<T>(path, { method: 'GET' });
}

export function postJson<T>(path: string, body?: unknown): Promise<T> {
  return send<T>(path, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(body ?? {}),
  });
}

export function postFile<T>(path: string, file: File, extraHeaders: Record<string, string> = {}): Promise<T> {
  return send<T>(path, {
    method: 'POST',
    body: file,
    headers: { 'Content-Type': 'application/octet-stream', 'X-Filename': encodeURIComponent(file.name), ...extraHeaders },
  });
}
