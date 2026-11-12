// Result page (#/result) of an exam or (v0.6.5) a practice: the score first, then per-question correctness. The review
// cards (your answer, the correct answer, explanation, memory aid, each with its own English / 中文 / 日本語 switch) come
// from components/review.ts. Rendered inside the shell.
import '../styles/review.css';
import '../styles/records.css';
import { modeName, pageHead } from '../components/common.ts';
import { iconButton } from '../components/icon-button.ts';
import { reviewBlock, reviewItem } from '../components/review.ts';
import { percentOf, shuffle } from '../domain/grading.ts';
import { t, tn } from '../i18n/index.ts';
import { navigate, startExam } from '../state/actions.ts';
import { S } from '../state/store.ts';
import { h } from '../utils/dom.ts';
import { fmtTime } from '../utils/format.ts';

export function renderResult(): HTMLElement {
  const R = S.result;
  if (!R) {
    // Normally unreachable (the router sends #/result without a result to the records page)
    return h('div', { class: 'page page-result' },
      pageHead(t('result.title')),
      h('div', { class: 'panel rec-empty' },
        h('p', { class: 'empty' }, t('result.none')),
        h('div', { class: 'acts' }, h('button', { class: 'btn primary', type: 'button', onclick: () => navigate({ name: 'records' }) }, t('result.toRecords')))));
  }

  const n = R.results.length;
  const good = R.results.filter((r) => r.correct).length;
  const wrong = n - good;
  const pct = percentOf(good, n);

  // startExam asks first when an unsubmitted exam is saved (for example when saving this result failed and the answers were kept)
  // Again / retry start the same kind as this result: a practice continues as a practice
  const again = (): void => void startExam(shuffle(R.results.map((r) => r.q)), R.mode, R.kind);
  const retryWrong = (): void => void startExam(shuffle(R.results.filter((r) => !r.correct).map((r) => r.q)), 'retry', R.kind);
  const practice = R.kind === 'practice';
  const home = (): void => navigate({ name: 'home' });
  const recordId = R.recordId;

  const meta = { mode: modeName(R.mode), time: fmtTime(R.seconds) };
  const metaText = R.prev ? tn('result.metaPrev', n, { ...meta, prev: R.prev.percent }) : tn('result.meta', n, meta);

  const rows = R.results.map((r, i) => ({
    correct: r.correct,
    render: () => reviewItem({ q: r.q, ans: r.ans, correct: r.correct }, {
      no: i + 1,
      ai: { page: 'result', mode: R.kind, key: R.key, startedAt: R.startedAt, recordId: R.recordId },
    }),
  }));

  // Icon actions (RS-1): the same DOM at every width; the text label shows beside the icon except on phones (CSS, `.lbl`)
  const action = (icon: 'retake' | 'list' | 'home', label: string, onClick: () => void): HTMLElement => {
    const b = iconButton({ icon, label, onClick });
    b.classList.add('res-act');
    b.append(h('span', { class: 'lbl', 'aria-hidden': 'true' }, label));
    return b;
  };

  return h('div', { class: 'page page-result' },
    pageHead(t(practice ? 'result.titlePractice' : 'result.title')),
    h('section', { class: 'score' },
      h('div', { class: 'big num', role: 'img', 'aria-label': t('result.scoreLabel', { pct }) }, String(pct), h('small', null, '%')),
      h('div', { class: 'says' },
        h('p', null, wrong === 0 ? t('result.allCorrect') : t('result.summary', { good, wrong })),
        h('p', { class: 'dim' }, metaText),
        // Questions deleted during the exam (v0.6.5): left out of the score, the record and the review below
        R.notCounted ? h('div', { class: 'callout warn res-gone', role: 'note' }, h('p', null, tn('result.notCounted', R.notCounted))) : null),
      h('div', { class: 'acts' },
        wrong ? h('button', { class: 'btn primary', type: 'button', onclick: retryWrong }, t('result.retryWrong')) : null,
        action('retake', t(practice ? 'result.againPractice' : 'result.again'), again),
        recordId != null ? action('list', t('result.viewRecord'), () => navigate({ name: 'record', id: recordId })) : null,
        action('home', t('result.home'), home))),
    reviewBlock(rows, { get: () => S.rvFilter, set: (f) => { S.rvFilter = f; } }));
}
