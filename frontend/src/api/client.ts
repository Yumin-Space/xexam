// Centralized API client: all network requests go out from here, errors are normalized to ApiError, and pages only handle ApiError.
// Uses relative URLs (same origin). An expired login (the Cloudflare Access login page) shows up as "not JSON"; it is recognized here and a hint is given.

export type ApiErrorKind =
  | 'http' // the service returned an error status code (with the Chinese-language reason supplied by the backend)
  | 'network' // cannot connect
  | 'unexpected'; // the response is not what was expected (common cause: login expired, redirected to the login page)

export class ApiError extends Error {
  readonly kind: ApiErrorKind;
  readonly status: number;

  constructor(kind: ApiErrorKind, message: string, status = 0) {
    super(message);
    this.name = 'ApiError';
    this.kind = kind;
    this.status = status;
  }
}

export const MSG_NETWORK = '连不上服务。请确认程序（或 Docker 容器）还在运行；如果你是通过域名访问的，也可能是登录已过期，请刷新页面重新登录。';
export const MSG_UNEXPECTED = '服务返回的不是预期的内容。如果你是通过域名访问的，可能是登录已过期，请刷新页面重新登录后再试。';

function isJson(r: Response): boolean {
  return /json/i.test(r.headers.get('Content-Type') ?? '');
}

async function parse<T>(r: Response): Promise<T> {
  if (!isJson(r)) {
    // Not JSON: an HTML login page, a proxy error page...
    if (!r.ok) throw new ApiError('http', `请求失败（HTTP ${r.status}）。如果你是通过域名访问的，可能是登录已过期，请刷新页面重新登录。`, r.status);
    throw new ApiError('unexpected', MSG_UNEXPECTED, r.status);
  }
  let body: unknown;
  try {
    body = await r.json();
  } catch {
    throw new ApiError('unexpected', MSG_UNEXPECTED, r.status);
  }
  if (!r.ok) {
    const msg = typeof body === 'object' && body !== null && 'error' in body && typeof (body as { error: unknown }).error === 'string'
      ? (body as { error: string }).error
      : `请求失败（HTTP ${r.status}）。`;
    throw new ApiError('http', msg, r.status);
  }
  return body as T;
}

async function send<T>(input: string, init?: RequestInit): Promise<T> {
  let r: Response;
  try {
    r = await fetch(input, init);
  } catch {
    throw new ApiError('network', MSG_NETWORK);
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

export function postFile<T>(path: string, file: File): Promise<T> {
  return send<T>(path, {
    method: 'POST',
    body: file,
    headers: { 'Content-Type': 'application/octet-stream', 'X-Filename': encodeURIComponent(file.name) },
  });
}
