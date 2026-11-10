// Centralized API client: all network requests go out from here, errors are normalized to ApiError, and pages only handle ApiError.
// Uses relative URLs (same origin). An expired login (the Cloudflare Access login page) shows up as "not JSON"; it is recognized here.
// No user-facing text lives here: pages turn an ApiError into text in the current UI language with errorText() (src/i18n/errors.ts).

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

  constructor(kind: ApiErrorKind, status = 0, serverMessage = '') {
    super(serverMessage || `${kind} ${status}`);
    this.name = 'ApiError';
    this.kind = kind;
    this.status = status;
    this.serverMessage = serverMessage;
  }
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
    throw msg ? new ApiError('http', r.status, msg) : new ApiError('status', r.status);
  }
  return body as T;
}

async function send<T>(input: string, init?: RequestInit): Promise<T> {
  let r: Response;
  try {
    r = await fetch(input, init);
  } catch {
    throw new ApiError('network');
  }
  return parse<T>(r);
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
