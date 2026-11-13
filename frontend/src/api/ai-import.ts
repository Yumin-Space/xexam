// The AI file import routes (v1.1, IM-5, API-U6 / U7): the upload is a raw `application/octet-stream` body (like
// /api/upload) sent with XMLHttpRequest for its progress; a refusal becomes exactly the ApiError the fetch client builds.
import { replyLang, type UiLangCode } from '../domain/ai-plus.ts';
import { genStatusOf, jobOf } from '../state/ai.ts';
import type { GenJob, GenStatus } from '../types/api.ts';
import { ApiError, errorFromResponse, postJson, reportAuthError } from './client.ts';

export const UPLOAD_PATH = '/api/ai/import/upload';

/** The part of XMLHttpRequest the upload uses (so a test can pass a fake) */
export interface XhrLike {
  open(method: string, url: string): void;
  setRequestHeader(name: string, value: string): void;
  send(body: unknown): void;
  abort(): void;
  getResponseHeader(name: string): string | null;
  upload: { onprogress: ((e: { loaded: number; total: number; lengthComputable: boolean }) => void) | null };
  onload: (() => void) | null;
  onerror: (() => void) | null;
  onabort: (() => void) | null;
  ontimeout: (() => void) | null;
  status: number;
  responseText: string;
}

export interface UploadAnswer {
  job: GenJob;
  gen: GenStatus;
}

export function abortError(): Error {
  return Object.assign(new Error('The request was aborted.'), { name: 'AbortError' });
}

/**
 * `POST /api/ai/import/upload`: the file bytes as the body, `X-Filename: encodeURIComponent(name)`, `X-Ui-Lang`. `onProgress`
 * gets the bytes sent. Aborting `signal` aborts the request and rejects with an AbortError (nothing exists on the server
 * then). A non-2xx answer rejects with the ApiError of `errorFromResponse` (a 401 also runs the sign-in handling); a lost
 * connection is kind `network`.
 */
export function uploadImport(
  file: File, lang: UiLangCode, onProgress: (loaded: number, total: number) => void, signal: AbortSignal,
  make: () => XhrLike = () => new XMLHttpRequest() as unknown as XhrLike,
): Promise<UploadAnswer> {
  return new Promise<UploadAnswer>((resolve, reject) => {
    if (signal.aborted) {
      reject(abortError());
      return;
    }
    const xhr = make();
    let done = false;
    const finish = (fn: () => void): void => {
      if (done) return;
      done = true;
      signal.removeEventListener('abort', onAbort);
      fn();
    };
    const onAbort = (): void => {
      xhr.abort();
      finish(() => reject(abortError()));
    };
    signal.addEventListener('abort', onAbort);
    xhr.open('POST', UPLOAD_PATH);
    xhr.setRequestHeader('Content-Type', 'application/octet-stream');
    xhr.setRequestHeader('X-Filename', encodeURIComponent(file.name));
    xhr.setRequestHeader('X-Ui-Lang', replyLang(lang));
    xhr.upload.onprogress = (e): void => {
      if (e.lengthComputable) onProgress(e.loaded, e.total);
    };
    xhr.onload = (): void => finish(() => {
      if (xhr.status >= 200 && xhr.status < 300) {
        let body: unknown;
        try {
          body = JSON.parse(xhr.responseText);
        } catch {
          reject(new ApiError('unexpected', xhr.status));
          return;
        }
        const o = typeof body === 'object' && body !== null ? (body as Record<string, unknown>) : {};
        const job = jobOf(o['job']);
        const gen = genStatusOf(o['gen']);
        if (!job || !gen) reject(new ApiError('unexpected', xhr.status));
        else resolve({ job, gen });
        return;
      }
      const err = errorFromResponse(xhr.status, xhr.responseText, xhr.getResponseHeader('Retry-After'), UPLOAD_PATH);
      reportAuthError(UPLOAD_PATH, err);
      reject(err);
    });
    xhr.onerror = (): void => finish(() => reject(new ApiError('network')));
    xhr.ontimeout = (): void => finish(() => reject(new ApiError('network')));
    xhr.onabort = (): void => finish(() => reject(abortError()));
    xhr.send(file);
  });
}

/** `POST /api/ai/import/confirm {id, name}`: the job goes on writing under that bank name (API-U7, A-OWNER-57) */
export async function confirmImport(id: number, name: string): Promise<UploadAnswer> {
  const body = await postJson<Record<string, unknown>>('/api/ai/import/confirm', { id, name });
  const job = jobOf(body['job']);
  const gen = genStatusOf(body['gen']);
  if (!job || !gen) throw new ApiError('unexpected', 200);
  return { job, gen };
}
