// 集中的 API 客户端：所有网络请求都从这里出去，错误统一成 ApiError，页面只处理 ApiError。
// 用相对地址（同源）。登录过期（Cloudflare Access 的登录页）会表现为"不是 JSON"，这里统一识别并给出提示。

export type ApiErrorKind =
  | 'http' // 服务返回了错误状态码（带后端给的中文原因）
  | 'network' // 连不上
  | 'unexpected'; // 返回的不是预期内容（常见原因：登录已过期，被重定向到登录页）

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
    // 非 JSON：HTML 登录页、代理的错误页……
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
