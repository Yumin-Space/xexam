// A parser for server-sent events (the text/event-stream format of AI Chat's streamed answers, v0.8), without the DOM's
// EventSource (which cannot POST). Pure: it gets the text as it arrives (already decoded from UTF-8 by the caller, with
// TextDecoder's stream mode, so a character split between two chunks is never broken) and calls onEvent per event.
// Rules (the WHATWG event-stream format): lines end in LF, CRLF or CR, also when the two characters of a CRLF arrive in two
// chunks; "data:" lines of one event are joined with LF; a line starting with ":" is a comment (the keep-alive); "event:"
// names the event (default "message"); a blank line dispatches it; an event without data is not dispatched; anything left
// at the end of the stream without its blank line is dropped. One event (or one unfinished line) may hold at most 1 MiB.

export interface SseEvent {
  event: string;
  data: string;
}

export const SSE_MAX_EVENT = 1024 * 1024;

/** An event (or a line still waiting for its end) grew beyond the limit */
export class SseTooLarge extends Error {
  constructor() {
    super('server-sent event too large');
    this.name = 'SseTooLarge';
  }
}

export interface SseParser {
  /** The next piece of the stream (any length, split anywhere) */
  push(chunk: string): void;
  /** The stream ended: an unfinished event is dropped */
  end(): void;
}

export function createSseParser(onEvent: (ev: SseEvent) => void, maxEvent = SSE_MAX_EVENT): SseParser {
  let buf = '';
  /** The previous chunk ended in CR: an LF at the start of the next one belongs to that line end */
  let afterCr = false;
  let data: string[] = [];
  let size = 0;
  let name = '';

  const dispatch = (): void => {
    const ev = data.length ? { event: name || 'message', data: data.join('\n') } : null;
    data = [];
    size = 0;
    name = '';
    if (ev) onEvent(ev);
  };

  const line = (l: string): void => {
    if (l === '') {
      dispatch();
      return;
    }
    if (l.charCodeAt(0) === 58) return; // ":" comment
    const colon = l.indexOf(':');
    const field = colon < 0 ? l : l.slice(0, colon);
    let value = colon < 0 ? '' : l.slice(colon + 1);
    if (value.charCodeAt(0) === 32) value = value.slice(1);
    if (field === 'data') {
      size += value.length + 1;
      if (size > maxEvent) throw new SseTooLarge();
      data.push(value);
    } else if (field === 'event') {
      name = value;
    }
    // "id" and "retry" are not used; unknown fields are ignored
  };

  return {
    push(chunk: string): void {
      let s = chunk;
      if (afterCr && s.charCodeAt(0) === 10) s = s.slice(1);
      afterCr = false;
      buf += s;
      let start = 0;
      for (let i = 0; i < buf.length; i++) {
        const c = buf.charCodeAt(i);
        if (c !== 10 && c !== 13) continue;
        line(buf.slice(start, i));
        if (c === 13) {
          if (i + 1 < buf.length) {
            if (buf.charCodeAt(i + 1) === 10) i++;
          } else {
            afterCr = true;
          }
        }
        start = i + 1;
      }
      buf = buf.slice(start);
      if (buf.length > maxEvent) throw new SseTooLarge();
    },
    end(): void {
      buf = '';
      data = [];
      size = 0;
      name = '';
    },
  };
}
