// Shared review rendering, used by the result page, the exam-record detail, the wrong-answer review and (its parts) the
// feedback of the practice page:
//   · reviewItem(): one question with your answer and the correct answer (or only the correct answer), hotspot per-item
//     verdicts, explanation, memory aid, note and the per-question stats line, plus (v0.6.5) its own English / 中文 / 日本語
//     switch. The switch starts at the question language from the settings and changes only that card: its question text,
//     options, explanation and memory aid. Languages the question lacks are greyed out. (It replaces the old separate
//     "Chinese translation" fold.)
//   · reviewBlock(): the correct / wrong strip, the "wrong only / all" switch and the list of review items.
//   · answerReview() / explainBlocks(): the answer part and the explanation part of a card, for the practice page.
// Question content is shown with the lang attribute of the language it is in; every label goes through the "result.*" catalog.
import '../styles/records.css';
import { aiControls } from '../domain/ai-plus.ts';
import { inNotebook, masterStreakOf, stateOf, type Answer } from '../domain/grading.ts';
import { explainOf, htmlLang, shown, type Lang, type LangText, type Shown } from '../domain/language.ts';
import { t, tn, type MsgKey } from '../i18n/index.ts';
import { onRouteChange } from '../state/actions.ts';
import { S } from '../state/store.ts';
import type { Question } from '../types/api.ts';
import { h, paras, svgMark, type Child } from '../utils/dom.ts';
import { reanchorAiPop } from './ai-pop.ts';
import { aiPlusNow, naTag, pheadRight, studyButton, studyIcon, studyState, type StudyTarget } from './ai-plus-controls.ts';
import { bankName, kindLabel } from './common.ts';
import { langSwitch } from './lang-switch.ts';

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
  /**
   * v0.9, AI+ only: where the AI analysis of each card belongs (result page or record detail). Without it, or when the
   * account has no AI+ feature, a card shows no AI control at all.
   */
  ai?: {
    page: 'result' | 'detail';
    mode: 'practice' | 'exam';
    key: string | null;
    startedAt: number | null;
    recordId: number | null;
  };
}

export type ReviewFilter = 'wrong' | 'all';

/** One row of reviewBlock(): whether it was answered correctly, and how to draw it (drawn lazily, once). */
export interface ReviewRow {
  correct: boolean;
  render: () => HTMLElement;
}

/**
 * The language a card was switched to, by question id. Kept while the page stays (a redraw of the page, for example when the
 * record id of a fresh result arrives, keeps every card's choice) and forgotten when another page opens: cards then start
 * again at the question language from the settings.
 */
const cardLang = new Map<string, Lang>();
onRouteChange(() => cardLang.clear());

const letters = (a: Answer): string[] => (Array.isArray(a) ? a.filter((x): x is string => typeof x === 'string') : []);
const langAttr = (l: Lang | null): string | null => (l ? htmlLang(l) : null);

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
      picked ? h('span', { class: 'tag ' + (isAns ? 'k' : 'ng') }, t('result.q.mineTag')) : null,
    ].filter((x): x is HTMLSpanElement => !!x);
    return h('div', { class: 'opt review' + (isAns ? ' is-ans' : '') + (picked && !isAns ? ' is-bad' : '') },
      h('span', { class: 'bubble' }, o.key),
      h('div', { class: 'body' },
        tags.length ? h('div', { class: 'tags' }, tags) : null,
        h('span', { class: 't', lang: htmlLang(view.lang) }, view.optionText(oi))));
  }));
}

function choiceVerdict(q: Question, mine: string[] | null, correct: boolean): HTMLElement {
  const sep = t('common.listSep');
  const right = (q.answer as string[]).join(sep);
  const key = h('span', { class: 'vp' }, withEl(t('result.q.key'), 'a', h('b', { class: 'r' }, right)));
  if (!mine) return h('p', { class: 'verdict' }, key);
  // Each chosen letter follows its option (v0.7.5): green when it is one of the correct answers, orange when it is not, so a
  // partly right multiple choice shows both colours. "Not answered" is one orange word.
  const rightKeys = q.answer as string[];
  const mineEl = mine.length
    ? h('span', null, mine.flatMap((k, i) => [i ? sep : null, h('b', { class: rightKeys.includes(k) ? 'r' : 'w' }, k)]))
    : h('b', { class: correct ? 'r' : 'w' }, t('result.q.unanswered'));
  return h('p', { class: 'verdict' },
    h('span', { class: 'vp' }, withEl(t('result.q.mine'), 'a', mineEl)),
    key);
}

function hotspotBody(q: Question, view: Shown, ans: Answer, showMine: boolean): { body: HTMLElement; okN: number } {
  const right = q.answer as number[];
  const mineAt = (k: number): number | null => {
    const v = Array.isArray(ans) ? ans[k] : null;
    return typeof v === 'number' ? v : null;
  };
  const itemLang = htmlLang(view.lang);
  let okN = 0;
  const body = h('div', { class: 'hs' }, q.items.map((_, k) => {
    const want = right[k] ?? -1;
    const keyText = h('b', { class: 'g', lang: htmlLang(view.choiceLang(k, want)) }, view.choiceText(k, want));
    if (!showMine) {
      return h('div', { class: 'hs-item' },
        h('span', { class: 'hs-no' }, k + 1),
        h('p', { lang: itemLang, class: 'pre' }, view.itemText(k)),
        h('div', { class: 'hsa' }, h('span', null, withEl(t('result.q.key'), 'a', keyText))));
    }
    const mine = mineAt(k);
    const ok = mine === want;
    if (ok) okN++;
    const mineText = mine == null ? t('result.q.unanswered') : view.choiceText(k, mine) || t('result.q.unanswered');
    return h('div', { class: 'hs-item ' + (ok ? 'rv-ok' : 'rv-ng') },
      h('span', { class: 'hs-no' }, k + 1),
      h('p', { lang: itemLang, class: 'pre' }, view.itemText(k)),
      h('div', { class: 'hsa' },
        h('span', { class: 'tag ' + (ok ? 'g' : 'ng') }, ok ? t('result.q.correct') : t('result.q.wrong')),
        h('span', null, withEl(t('result.q.youChose'), 'a',
          // the chosen choice as a solid green (right) or orange (wrong) key; "not answered" is plain text (nothing was chosen)
          h('b', { class: mine == null ? 'none' : ok ? 'g' : 'r', lang: mine == null ? null : htmlLang(view.choiceLang(k, mine)) }, mineText))),
        ok ? null : h('span', null, withEl(t('result.q.key'), 'a', keyText))));
  }));
  return { body, okN };
}

/**
 * The answer part of a card in the language of `view`: the options (or hotspot items) marked right / wrong / chosen, and
 * the verdict line. showMine false: only the correct answer (wrong-answer review).
 */
export function answerReview(q: Question, view: Shown, ans: Answer, correct: boolean, showMine = true): { body: HTMLElement; verdict: HTMLElement } {
  if (q.kind === 'hotspot') {
    const hs = hotspotBody(q, view, ans, showMine);
    const n = q.items.length;
    const verdict = showMine
      ? h('p', { class: 'verdict' }, withEl(tn('result.q.hsVerdict', n), 'ok', h('b', { class: correct ? 'r' : 'w' }, hs.okN)))
      : h('p', { class: 'verdict' }, tn('result.q.hsRule', n));
    return { body: hs.body, verdict };
  }
  const mine = showMine ? letters(ans) : null;
  return { body: choiceBody(q, view, mine), verdict: choiceVerdict(q, mine, correct) };
}

/**
 * The explanation, memory aid and answer note of a question in a language (six-column questions have their own per
 * language; older ones show their single explanation in every language). Empty parts are left out.
 */
export function explainBlocks(q: Question, lang: Lang): Array<HTMLElement | null> {
  const ex = explainOf(q, lang);
  const text = (x: LangText, tag: 'p' | 'span', cls?: string): HTMLElement => h(tag, { class: cls, lang: langAttr(x.lang) }, x.text);
  return [
    ex.explanation.text ? h('div', { class: 'explain' }, h('h4', null, t('result.q.explain')), text(ex.explanation, 'p', 'pre')) : null,
    ex.mnemonic.text ? h('div', { class: 'mnemo' }, h('h4', null, t('result.q.mnemonic')), text(ex.mnemonic, 'p')) : null,
    ex.note.text ? h('p', { class: 'note' }, withEl(t('result.q.note'), 'note', text(ex.note, 'span'))) : null,
  ];
}

/** One question in review form, with its own language switch. */
export function reviewItem(e: ReviewEntry, opts: ReviewOptions): HTMLElement {
  const { q } = e;
  const showMine = opts.showMine !== false;
  const unanswered = stateOf(q, e.ans) === 0;
  const status = !showMine ? null : e.correct
    ? h('span', { class: 'badge ok' }, t('result.q.correct'))
    : h('span', { class: 'badge ng' }, unanswered ? t('result.q.unanswered') : t('result.q.wrong'));
  const source = S.banks.length > 1 ? t('result.q.source', { bank: bankName(q.bank), sheet: q.sheet }) : q.sheet;
  const stats = opts.stats === undefined ? statsLine(q) : opts.stats;
  const paper = h('div', { class: 'paper' });

  // Draws the card's content in a language (the effective one: a language the question lacks falls back to its base)
  const draw = (want: Lang, focusSwitch: boolean): void => {
    const view = shown(q, want);
    const { body, verdict } = answerReview(q, view, e.ans, e.correct, showMine);
    const sw = langSwitch(q, view.lang, (l) => {
      cardLang.set(q.id, l);
      draw(l, true);
    });
    // v0.9: the AI analysis control of the card (AI+ accounts only): the book icon in the header, the AI 解析 button at the
    // right of the answer line. Its question language is the one the card shows now.
    let head: HTMLElement = sw;
    let answerLine: HTMLElement = verdict;
    if (opts.ai && aiPlusNow().show) {
      const target: StudyTarget = {
        page: opts.ai.page, mode: opts.ai.mode, key: opts.ai.key, startedAt: opts.ai.startedAt, recordId: opts.ai.recordId,
        questionId: q.id, no: opts.no, qlang: () => shown(q, cardLang.get(q.id) ?? S.settings.questionLang).lang,
      };
      const spec = aiControls(opts.ai.page)[0];
      if (spec) {
        const state = studyState(target);
        head = pheadRight([studyIcon(target, spec, state)], naTag(state), sw);
        answerLine = h('div', { class: 'verdict-row' }, verdict, studyButton(target, spec, state, { bar: false }));
      }
    }
    paper.replaceChildren(
      h('div', { class: 'phead' },
        h('b', { class: 'num' }, t('result.q.no', { n: opts.no })),
        status,
        h('span', { class: 'badge' }, kindLabel(q)),
        opts.retired ? h('span', { class: 'badge retired' }, t('result.q.retired')) : null,
        h('span', { class: 'rv-src' }, source),
        head),
      ...[
        opts.retired ? h('p', { class: 'rv-retired' }, t('result.q.retiredHint')) : null,
        h('div', { class: 'stem' }, paras(view.stem, undefined, htmlLang(view.lang))),
        body,
        answerLine,
        ...explainBlocks(q, view.lang),
        stats,
      ].filter((x): x is HTMLElement => !!x));
    // Switching redraws the card: the focus goes back to the pressed button instead of being lost
    if (focusSwitch) sw.querySelector<HTMLElement>('[aria-pressed="true"]')?.focus({ preventScroll: true });
    // The redraw replaced the opener of an open popover: it follows the new one
    if (opts.ai) reanchorAiPop();
  };
  draw(cardLang.get(q.id) ?? S.settings.questionLang, false);

  return h('article', { class: 'rv' + (showMine && e.correct ? ' good' : ''), tabindex: '-1', 'aria-label': t('result.q.no', { n: opts.no }) }, paper);
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
