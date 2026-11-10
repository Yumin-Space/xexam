// Shared review rendering, used by the result page, the exam-record detail and the wrong-answer review:
//   · reviewItem(): one question with your answer and the correct answer (or only the correct answer), hotspot per-item verdicts,
//     explanation, mnemonic, note, the collapsible Chinese translation (unless the question is already shown in Chinese) and the per-question stats line;
//   · reviewBlock(): the correct / wrong strip, the "wrong only / all" switch and the list of review items.
// Question content (stems, options, items) is shown in the question language from the settings, with the lang attribute it returns;
// explanations, mnemonics and notes are shown as they are. Every label goes through the "result.*" catalog.
import '../styles/records.css';
import { inNotebook, masterStreakOf, stateOf, type Answer } from '../domain/grading.ts';
import { shown, type Shown } from '../domain/language.ts';
import { t, tn, type MsgKey } from '../i18n/index.ts';
import { S } from '../state/store.ts';
import type { Question } from '../types/api.ts';
import { h, paras, svgMark, type Child } from '../utils/dom.ts';
import { bankName, kindLabel } from './common.ts';

export interface ReviewEntry {
  q: Question;
  /** What was picked (letters, or one choice index per hotspot item); null / undefined = unanswered. Ignored when showMine is false. */
  ans: Answer;
  correct: boolean;
}

export interface ReviewOptions {
  /** Number shown in the heading ("Question 3") */
  no: number;
  /** false: only the correct answer is shown (wrong-answer review). Default true. */
  showMine?: boolean;
  /** The question was retired from its bank (shows a badge) */
  retired?: boolean;
  /** Replaces the default per-question stats line (null = no stats line) */
  stats?: HTMLElement | null;
}

export type ReviewFilter = 'wrong' | 'all';

/** One row of reviewBlock(): whether it was answered correctly, and how to draw it (drawn lazily, once). */
export interface ReviewRow {
  correct: boolean;
  render: () => HTMLElement;
}

const letters = (a: Answer): string[] => (Array.isArray(a) ? a.filter((x): x is string => typeof x === 'string') : []);

/** Puts an element where `{name}` stands in an already translated text (keeps word order right in every language). */
function withEl(text: string, name: string, el: HTMLElement): Child[] {
  const mark = `{${name}}`;
  const i = text.indexOf(mark);
  if (i < 0) return [text, ' ', el];
  return [text.slice(0, i), el, text.slice(i + mark.length)];
}

/** "Answered 5 times so far (2 wrong); still in the wrong-answer notebook." */
function statsLine(q: Question): HTMLElement | null {
  const st = S.progress?.questions?.[q.id];
  if (!st) return null;
  const key: MsgKey = inNotebook(st, masterStreakOf(S.progress)) ? 'result.stats.inNotebook' : st.wrong ? 'result.stats.left' : 'result.stats';
  return h('p', { class: 'stats' }, tn(key, st.attempts, { wrong: st.wrong }));
}

function choiceBody(q: Question, view: Shown, mine: string[] | null): HTMLElement {
  const right = q.answer as string[];
  return h('div', { class: 'opts' + (q.kind === 'multi' ? ' multi' : '') }, q.options.map((o, oi) => {
    const isAns = right.includes(o.key);
    const picked = !!mine && mine.includes(o.key);
    const tags = [
      isAns ? h('span', { class: 'tag g' }, t('result.q.correctTag')) : null,
      picked ? h('span', { class: 'tag ' + (isAns ? 'k' : 'r') }, t('result.q.mineTag')) : null,
    ].filter((x): x is HTMLSpanElement => !!x);
    return h('div', { class: 'opt review' + (isAns ? ' is-ans' : '') + (picked && !isAns ? ' is-bad' : '') },
      h('span', { class: 'bubble' }, o.key),
      h('div', { class: 'body' },
        tags.length ? h('div', { class: 'tags' }, tags) : null,
        h('span', { class: 't', lang: view.lang }, view.optionText(oi))));
  }));
}

function choiceVerdict(q: Question, mine: string[] | null, e: ReviewEntry): HTMLElement {
  const sep = t('common.listSep');
  const right = (q.answer as string[]).join(sep);
  const key = h('span', { class: 'vp' }, withEl(t('result.q.key'), 'a', h('b', { class: 'r' }, right)));
  if (!mine) return h('p', { class: 'verdict' }, key);
  const mineText = mine.length ? mine.join(sep) : t('result.q.unanswered');
  return h('p', { class: 'verdict' },
    h('span', { class: 'vp' }, withEl(t('result.q.mine'), 'a', h('b', { class: e.correct ? 'r' : 'w' }, mineText))),
    key);
}

function hotspotBody(q: Question, view: Shown, ans: Answer, showMine: boolean): { body: HTMLElement; okN: number } {
  const right = q.answer as number[];
  const mineAt = (k: number): number | null => {
    const v = Array.isArray(ans) ? ans[k] : null;
    return typeof v === 'number' ? v : null;
  };
  let okN = 0;
  const body = h('div', { class: 'hs' }, q.items.map((_, k) => {
    const want = right[k] ?? -1;
    const keyText = h('b', { class: 'g', lang: view.choiceLang(k, want) }, view.choiceText(k, want));
    if (!showMine) {
      return h('div', { class: 'hs-item' },
        h('span', { class: 'hs-no' }, k + 1),
        h('p', { lang: view.lang, class: 'pre' }, view.itemText(k)),
        h('div', { class: 'hsa' }, h('span', null, withEl(t('result.q.key'), 'a', keyText))));
    }
    const mine = mineAt(k);
    const ok = mine === want;
    if (ok) okN++;
    const mineText = mine == null ? t('result.q.unanswered') : view.choiceText(k, mine) || t('result.q.unanswered');
    return h('div', { class: 'hs-item ' + (ok ? 'rv-ok' : 'rv-ng') },
      h('span', { class: 'hs-no' }, k + 1),
      h('p', { lang: view.lang, class: 'pre' }, view.itemText(k)),
      h('div', { class: 'hsa' },
        h('span', { class: 'tag ' + (ok ? 'g' : 'r') }, ok ? t('result.q.correct') : t('result.q.wrong')),
        h('span', null, withEl(t('result.q.youChose'), 'a',
          h('b', { class: ok ? 'g' : 'r', lang: mine == null ? null : view.choiceLang(k, mine) }, mineText))),
        ok ? null : h('span', null, withEl(t('result.q.key'), 'a', keyText))));
  }));
  return { body, okN };
}

/** One question in review form. */
export function reviewItem(e: ReviewEntry, opts: ReviewOptions): HTMLElement {
  const { q } = e;
  const showMine = opts.showMine !== false;
  const unanswered = stateOf(q, e.ans) === 0;
  // Shown in the question language from the settings; a question without that language falls back to Japanese
  const view = shown(q, S.settings.questionLang);
  let body: HTMLElement;
  let verdict: HTMLElement;

  if (q.kind === 'hotspot') {
    const hs = hotspotBody(q, view, e.ans, showMine);
    body = hs.body;
    const n = q.items.length;
    verdict = showMine
      ? h('p', { class: 'verdict' }, withEl(tn('result.q.hsVerdict', n), 'ok', h('b', { class: e.correct ? 'r' : 'w' }, hs.okN)))
      : h('p', { class: 'verdict' }, tn('result.q.hsRule', n));
  } else {
    const mine = showMine ? letters(e.ans) : null;
    body = choiceBody(q, view, mine);
    verdict = choiceVerdict(q, mine, e);
  }

  const status = !showMine ? null : e.correct
    ? h('span', { class: 'badge ok' }, t('result.q.correct'))
    : h('span', { class: 'badge ng' }, unanswered ? t('result.q.unanswered') : t('result.q.wrong'));
  const source = S.banks.length > 1 ? t('result.q.source', { bank: bankName(q.bank), sheet: q.sheet }) : q.sheet;
  const stats = opts.stats === undefined ? statsLine(q) : opts.stats;

  return h('article', { class: 'rv' + (showMine && e.correct ? ' good' : ''), tabindex: '-1', 'aria-label': t('result.q.no', { n: opts.no }) },
    h('div', { class: 'paper' },
      h('div', { class: 'phead' },
        h('b', { class: 'num' }, t('result.q.no', { n: opts.no })),
        status,
        h('span', { class: 'badge' }, kindLabel(q)),
        opts.retired ? h('span', { class: 'badge retired' }, t('result.q.retired')) : null,
        h('span', { class: 'rv-src' }, source)),
      opts.retired ? h('p', { class: 'rv-retired' }, t('result.q.retiredHint')) : null,
      h('div', { class: 'stem' }, paras(view.stem, undefined, view.lang)),
      body,
      verdict,
      q.explanation ? h('div', { class: 'explain' }, h('h4', null, t('result.q.explain')), h('p', { class: 'pre' }, q.explanation)) : null,
      q.mnemonic ? h('div', { class: 'mnemo' }, h('h4', null, t('result.q.mnemonic')), h('p', null, q.mnemonic)) : null,
      q.note ? h('p', { class: 'note' }, t('result.q.note', { note: q.note })) : null,
      // Shown only when it adds something: in Chinese the stem and options above already are this translation
      q.translation && view.lang !== 'zh'
        ? h('details', { class: 'tr' }, h('summary', null, t('result.q.translation')), h('p', { class: 'pre', lang: 'zh' }, q.translation))
        : null,
      stats));
}

/**
 * The review part of a result: a strip with one button per question (jumps to it), a heading with the
 * "wrong only / all" switch, and the list. Rendered items are kept, so switching does not rebuild them.
 * Returns the three elements in display order.
 */
export function reviewBlock(rows: readonly ReviewRow[], filter: { get: () => ReviewFilter; set: (f: ReviewFilter) => void }): HTMLElement[] {
  const total = rows.length;
  const wrong = rows.filter((r) => !r.correct).length;
  const built = new Map<number, HTMLElement>();
  const itemEl = (i: number): HTMLElement => {
    let el = built.get(i);
    if (!el) {
      el = (rows[i] as ReviewRow).render();
      el.id = 'rv-' + i;
      built.set(i, el);
    }
    return el;
  };

  const list = h('div', { class: 'rvlist' });
  const segW = h('button', { type: 'button' }, t('result.review.onlyWrong', { n: wrong }));
  const segA = h('button', { type: 'button' }, t('result.review.all', { n: total }));
  const fill = (): void => {
    const f = filter.get();
    segW.setAttribute('aria-pressed', String(f === 'wrong'));
    segA.setAttribute('aria-pressed', String(f === 'all'));
    const shownRows = rows.map((r, i) => ({ r, i })).filter(({ r }) => f === 'all' || !r.correct);
    list.replaceChildren(...(shownRows.length
      ? shownRows.map(({ i }) => itemEl(i))
      : [h('div', { class: 'allgood' }, t('result.review.allGood'))]));
  };
  const setFilter = (f: ReviewFilter): void => {
    if (filter.get() === f) return;
    filter.set(f);
    fill();
  };
  segW.addEventListener('click', () => setFilter('wrong'));
  segA.addEventListener('click', () => setFilter('all'));
  fill();

  const jump = (i: number): void => {
    if (filter.get() === 'wrong' && rows[i]?.correct) setFilter('all');
    const el = itemEl(i);
    el.scrollIntoView({ block: 'start' });
    el.focus({ preventScroll: true });
  };
  const strip = h('div', { class: 'strip', role: 'group', 'aria-label': t('result.strip.label') }, rows.map((r, i) => h('button', {
    class: r.correct ? 'ok' : 'ng',
    type: 'button',
    'aria-label': t('result.strip.item', { n: i + 1, verdict: r.correct ? t('result.q.correct') : t('result.q.wrong') }),
    onclick: () => jump(i),
  }, h('span', { class: 'n' }, i + 1), svgMark(r.correct))));

  const head = h('div', { class: 'rvhead' },
    h('h2', null, t('result.review.heading')),
    h('div', { class: 'seg', role: 'group', 'aria-label': t('result.review.filter') }, segW, segA));

  return [strip, head, list];
}
