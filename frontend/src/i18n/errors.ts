// Turns any error into text in the current UI language.
//   · ApiError from the backend: the backend's (Chinese) message is translated by localizeServerMessage(); a 429 that says how
//     long to wait (retryAfter) becomes "Try again in N minutes / hours / 1 day";
//   · the proxy's rate limit ('tooMany', v0.7.5): "too many requests, wait a few seconds", without the sign-in hint;
//   · network / unexpected / bare status errors: generic texts from the catalog;
//   · LocalError: an error raised by the page itself (catalog key + params), translated lazily so a language switch also switches it.
//   · v0.8: an error of the AI routes carries a stable code: `ai.<x>` is the catalog text `ai.err.<x>` (provider problems
//     `ai.up.*` are `ai.err.up.*`), with the number of a reached limit as {n}; `auth.guard` (Nginx) is `ai.err.guard`;
//     `auth.signin` / `auth.password` carry the backend's own texts and are translated like any backend message.
import { ApiError } from '../api/client.ts';
import { DEFAULT_LIMITS } from '../domain/ai-settings.ts';
import { hasKey, localeTag, t, uiLang, type MsgKey, type Params } from './index.ts';
import { localizeServerMessage } from './server-messages.ts';
import { waitText } from './wait.ts';

/** The number an AI limit text states when the answer did not give it: the defaults of the safety limits */
const LIMIT_OF: Record<string, number> = {
  'ai.tooLong': DEFAULT_LIMITS.messageChars,
  'ai.chatFull': DEFAULT_LIMITS.messagesPerChat,
  'ai.messageLimit': DEFAULT_LIMITS.messagesPerChat,
  'ai.chatLimit': DEFAULT_LIMITS.chatsPerUser,
  'ai.memoryFull': DEFAULT_LIMITS.memoriesPerUser,
  'ai.memoryLimit': DEFAULT_LIMITS.memoriesPerUser,
  'ai.memoryTooLong': DEFAULT_LIMITS.memoryChars,
};

/** The catalog key of an AI error code, or null when the catalog has no text for it */
function aiKey(code: string): MsgKey | null {
  if (code === 'auth.guard') return 'ai.err.guard';
  if (!code.startsWith('ai.')) return null;
  const key = `ai.err.${code.slice(3)}`;
  return hasKey('en', key) ? (key as MsgKey) : null;
}

/** Whether an AI error code has a text of its own */
export const knownAiCode = (code: string): boolean => aiKey(code) != null;

/**
 * The text of an AI error code (a stream's error event, a failed answer, a test result). `limit`: the number the answer
 * gave for a limit that was reached. An unknown code falls back to the generic internal error of AI.
 */
export function aiCodeText(code: string, limit?: number): string {
  const key = aiKey(code) ?? 'ai.err.internal';
  const n = limit ?? LIMIT_OF[code];
  return t(key, n == null ? undefined : { n: n.toLocaleString(localeTag()) });
}

export class LocalError extends Error {
  readonly key: MsgKey;
  readonly params: Params | undefined;

  constructor(key: MsgKey, params?: Params) {
    super(key);
    this.name = 'LocalError';
    this.key = key;
    this.params = params;
  }
}

export function errorText(e: unknown, fallback: MsgKey = 'err.generic'): string {
  if (e instanceof LocalError) return t(e.key, e.params);
  if (e instanceof ApiError) {
    if (e.code && aiKey(e.code)) return aiCodeText(e.code, e.limit);
    switch (e.kind) {
      case 'http':
        // Too many failed sign-ins / password checks: say how long the lock still runs
        if (e.status === 429 && e.retryAfter != null) return t('auth.err.tooMany', { wait: waitText(e.retryAfter) });
        return localizeServerMessage(e.serverMessage, uiLang());
      case 'tooMany':
        // The proxy's rate limit, still refusing after the automatic retries: not a lost connection or sign-in
        return t('err.tooMany');
      case 'network':
        return t('err.network');
      case 'unexpected':
        return t('err.unexpected');
      case 'status':
        return t('err.status', { status: e.status });
    }
  }
  return t(fallback);
}

/** A backend text that is not an error (for example the reason a row of an uploaded workbook was skipped) */
export function serverText(msg: string): string {
  return localizeServerMessage(msg, uiLang());
}
