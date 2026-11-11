// Centralized API client: all network requests go out from here, errors are normalized to ApiError, and pages only handle ApiError.
// Uses relative URLs (same origin). An expired login (the Cloudflare Access login page) shows up as "not JSON"; it is recognized here.
// No user-facing text lives here: pages turn an ApiError into text in the current UI language with errorText() (src/i18n/errors.ts).
// Sign-in (v0.6): a 401 from any route but the sign-in itself means the session is gone, and a 403 with the "choose a new
// password first" message means the password must be replaced; both are reported to the handler set with setAuthHandler()
// (state/actions.ts shows the matching page), and the request still fails with its ApiError as usual.
// Rate limit (v0.7.5): Nginx answers 429 itself when too many requests arrive (all users behind a tunnel share one address).
// That 429 has no JSON body with "retryAfter", which is how it differs from the sign-in lock (429 with JSON "retryAfter").
// A GET or HEAD refused that way is retried automatically (3 tries in all: wait Retry-After when given, else 1 s, then 2 s);
// other methods are never retried. Still refused, the request fails with kind 'tooMany', which pages show as "too many
// requests, wait a few seconds" instead of "cannot reach the service". The sign-in lock keeps its own handling ('http').
// Time limit (v0.7.5): a GET may give a time limit per try (the System log's poll reads do); a try that has not finished by
// then (no answer, or a body that stalls) is aborted and fails with kind 'network', like a lost connection.

export type ApiErrorKind =
  | 'http' // the service returned an error status code with a JSON body; `serverMessage` holds the backend's (Chinese) reason
  | 'status' // an error status code without a JSON body (proxy error page, login page, ...)
  | 'tooMany' // the proxy's own 429 (its rate limit, not the sign-in lock), still refused after the automatic retries
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

/** Tries in all for a GET / HEAD that the proxy's rate limit refuses (the first request and two retries) */
export const RATE_LIMIT_TRIES = 3;
/** Seconds to wait before the 1st and the 2nd retry when the 429 has no usable Retry-After */
const RETRY_WAITS = [1, 2] as const;
/** A longer Retry-After is cut to this many seconds, so a page never sits still for long */
const RETRY_MAX_WAIT = 10;

let pause = (ms: number): Promise<void> => new Promise((resolve) => setTimeout(resolve, ms));

/** For the unit tests: replaces the wait between automatic retries (milliseconds) */
export function setRetryPause(fn: (ms: number) => Promise<void>): void {
  pause = fn;
}

/** Whether an error is the proxy's rate limit (too many requests in a short time), after the automatic retries */
export function isTooManyRequests(e: unknown): boolean {
  return e instanceof ApiError && e.kind === 'tooMany';
}

/** Nginx's own 429: anything but the sign-in lock's JSON body with "retryAfter" */
async function isProxyLimit(r: Response): Promise<boolean> {
  if (r.status !== 429) return false;
  if (!isJson(r)) return true;
  try {
    const body: unknown = await r.clone().json();
    return !(typeof body === 'object' && body !== null && 'retryAfter' in body);
  } catch {
    return true;
  }
}

/** Seconds to wait before retry number `retry` (1 or 2): the 429's Retry-After (seconds form, at most RETRY_MAX_WAIT), else 1 s, then 2 s */
function retryWait(r: Response, retry: number): number {
  const header = r.headers.get('Retry-After')?.trim() ?? '';
  if (/^\d{1,9}$/.test(header)) return Math.min(Number(header), RETRY_MAX_WAIT);
  return RETRY_WAITS[Math.min(retry, RETRY_WAITS.length) - 1] ?? 1;
}

export interface GetOptions {
  /** Milliseconds one try may take, answer and body included; a try still running then fails with kind 'network' */
  timeoutMs?: number;
}

/** What one try gave: the parsed answer, or the wait in seconds before the next try (Nginx's 429 on a GET / HEAD) */
type Try<T> = { done: true; value: T } | { done: false; wait: number };

async function attemptOnce<T>(input: string, init: RequestInit | undefined, retry: number | null, signal: AbortSignal | undefined): Promise<Try<T>> {
  let r: Response;
  try {
    // same-origin: the session cookie (HttpOnly, SameSite=Strict) goes along; never to another origin
    r = await fetch(input, { credentials: 'same-origin', ...init, ...(signal ? { signal } : {}) });
  } catch {
    throw new ApiError('network');
  }
  if (await isProxyLimit(r)) {
    if (retry != null) return { done: false, wait: retryWait(r, retry) };
    throw new ApiError('tooMany', 429);
  }
  try {
    return { done: true, value: await parse<T>(r) };
  } catch (e) {
    // A body cut off by the time limit is a lost connection, not "unexpected" (which hints at an expired login)
    if (signal?.aborted) throw new ApiError('network');
    const problem = e instanceof ApiError ? authProblem(input.split('?')[0] ?? input, e) : null;
    if (problem) authHandler(problem);
    throw e;
  }
}

async function send<T>(input: string, init?: RequestInit, timeoutMs?: number): Promise<T> {
  const method = (init?.method ?? 'GET').toUpperCase();
  const retries = method === 'GET' || method === 'HEAD';
  for (let attempt = 1; ; attempt += 1) {
    const ctrl = timeoutMs != null && timeoutMs > 0 ? new AbortController() : null;
    const timer = ctrl ? setTimeout(() => ctrl.abort(), timeoutMs) : null;
    let res: Try<T>;
    try {
      res = await attemptOnce<T>(input, init, retries && attempt < RATE_LIMIT_TRIES ? attempt : null, ctrl?.signal);
    } finally {
      if (timer != null) clearTimeout(timer);
    }
    if (res.done) return res.value;
    // the wait between tries is outside the time limit of a try
    await pause(res.wait * 1000);
  }
}

export function getJson<T>(path: string, opts: GetOptions = {}): Promise<T> {
  return send<T>(path, { method: 'GET' }, opts.timeoutMs);
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
