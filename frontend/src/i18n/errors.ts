// Turns any error into text in the current UI language.
//   · ApiError from the backend: the backend's (Chinese) message is translated by localizeServerMessage();
//   · network / unexpected / bare status errors: generic texts from the catalog;
//   · LocalError: an error raised by the page itself (catalog key + params), translated lazily so a language switch also switches it.
import { ApiError } from '../api/client.ts';
import { t, uiLang, type MsgKey, type Params } from './index.ts';
import { localizeServerMessage } from './server-messages.ts';

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
    switch (e.kind) {
      case 'http':
        return localizeServerMessage(e.serverMessage, uiLang());
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
