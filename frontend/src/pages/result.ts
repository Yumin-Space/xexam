// 成绩页：先看分数，再看每题对错；错题显示你的答案、正确答案、解析、口诀、中文翻译。
import { inNotebook, masterStreakOf, percentOf, shuffle, stateOf, type Answer } from '../domain/grading.ts';
import { brand, kindLabel } from '../components/common.ts';
import { S, type ResultRow } from '../state/store.ts';
import { render, startSession } from '../state/actions.ts';
import { h, paras, svgMark } from '../utils/dom.ts';
import { fmtTime } from '../utils/format.ts';

const asLetters = (a: Answer): string[] => (Array.isArray(a) ? a.filter((x): x is string => typeof x === 'string') : []);

function reviewItem(r: ResultRow, i: number): HTMLElement {
  const q = r.q;
  const st = S.progress?.questions?.[q.id];
  const unanswered = stateOf(q, r.ans) === 0;
  let body: HTMLElement;
  let verdict: HTMLElement;

  if (q.kind === 'hotspot') {
    const right = q.answer as number[];
    const mineAt = (k: number): number | null => {
      const v = Array.isArray(r.ans) ? r.ans[k] : null;
      return typeof v === 'number' ? v : null;
    };
    const okN = q.items.filter((_, k) => mineAt(k) === right[k]).length;
    body = h('div', { class: 'hs' }, q.items.map((it, k) => {
      const mine = mineAt(k);
      const want = right[k] ?? -1;
      const ok = mine === want;
      return h('div', { class: 'hs-item ' + (ok ? 'rv-ok' : 'rv-ng') },
        h('span', { class: 'hs-no' }, k + 1),
        h('p', { lang: 'ja', class: 'pre' }, it.text),
        h('div', { class: 'hsa' },
          h('span', { class: 'tag ' + (ok ? 'g' : 'r') }, ok ? '答对' : '答错'),
          h('span', null, '你选了 ', h('b', { class: ok ? 'g' : 'r', lang: 'ja' }, mine == null ? '未作答' : (it.choices[mine] ?? '未作答'))),
          ok ? null : h('span', null, '正确答案 ', h('b', { class: 'g', lang: 'ja' }, it.choices[want] ?? ''))));
    }));
    verdict = h('p', { class: 'verdict' }, `这题共 ${q.items.length} 个小项，答对 `, h('b', { class: r.correct ? 'r' : 'w' }, `${okN} 个`), '；每个小项都对才算这题答对。');
  } else {
    const right = q.answer as string[];
    const mine = asLetters(r.ans);
    body = h('div', { class: 'opts' + (q.kind === 'multi' ? ' multi' : '') }, q.options.map((o) => {
      const isAns = right.includes(o.key);
      const picked = mine.includes(o.key);
      const tags = [isAns ? h('span', { class: 'tag g' }, '正确答案') : null, picked ? h('span', { class: 'tag ' + (isAns ? 'k' : 'r') }, '你的选择') : null].filter((x): x is HTMLElement => !!x);
      return h('div', { class: 'opt review' + (isAns ? ' is-ans' : '') + (picked && !isAns ? ' is-bad' : '') },
        h('span', { class: 'bubble' }, o.key),
        h('div', { class: 'body' }, tags.length ? h('div', { class: 'tags' }, tags) : null, h('span', { class: 't', lang: 'ja' }, o.text)));
    }));
    verdict = h('p', { class: 'verdict' }, '你的答案 ', h('b', { class: r.correct ? 'r' : 'w' }, unanswered ? '未作答' : mine.join('、')),
      '　正确答案 ', h('b', { class: 'r' }, right.join('、')));
  }

  const ms = masterStreakOf(S.progress);
  return h('article', { class: 'rv' + (r.correct ? ' good' : ''), id: 'rv-' + i },
    h('div', { class: 'paper' },
      h('div', { class: 'phead' },
        h('b', { class: 'num' }, `第 ${i + 1} 题`),
        h('span', { class: 'badge ' + (r.correct ? 'ok' : 'ng') }, r.correct ? '答对' : unanswered ? '未作答' : '答错'),
        h('span', { class: 'badge' }, kindLabel(q)),
        h('span', null, q.sheet)),
      h('div', { class: 'stem' }, paras(q.stem)),
      body,
      verdict,
      q.explanation ? h('div', { class: 'explain' }, h('h4', null, '解析：怎么选'), h('p', { class: 'pre' }, q.explanation)) : null,
      q.mnemonic ? h('div', { class: 'mnemo' }, h('h4', null, '记忆口诀'), h('p', null, q.mnemonic)) : null,
      q.note ? h('p', { class: 'note' }, '备注：' + q.note) : null,
      q.translation ? h('details', { class: 'tr' }, h('summary', null, '中文翻译'), h('p', { class: 'pre' }, q.translation)) : null,
      st ? h('p', { class: 'stats' },
        `这道题累计答过 ${st.attempts} 次，答错 ${st.wrong} 次` +
        (inNotebook(st, ms) ? '，还在错题本里' : st.wrong ? '，已经移出错题本' : '')) : null));
}

export function renderResult(): HTMLElement {
  const R = S.result;
  if (!R) return h('div', { class: 'wrap' });
  const n = R.results.length;
  const good = R.results.filter((r) => r.correct).length;
  const wrong = n - good;
  const pct = percentOf(good, n);

  const list = h('div', { class: 'rvlist' });
  const segW = h('button', { type: 'button', 'aria-pressed': 'true' }, `只看错题（${wrong}）`);
  const segA = h('button', { type: 'button', 'aria-pressed': 'false' }, `全部题目（${n}）`);
  const fill = (): void => {
    segW.setAttribute('aria-pressed', String(S.rvFilter === 'wrong'));
    segA.setAttribute('aria-pressed', String(S.rvFilter === 'all'));
    const items = R.results.map((r, i) => ({ r, i })).filter(({ r }) => S.rvFilter === 'all' || !r.correct);
    list.replaceChildren(...(items.length
      ? items.map(({ r, i }) => reviewItem(r, i))
      : [h('div', { class: 'allgood' }, '这一场全部答对，没有错题。想复习的话，切到「全部题目」。')]));
  };
  segW.addEventListener('click', () => {
    S.rvFilter = 'wrong';
    fill();
  });
  segA.addEventListener('click', () => {
    S.rvFilter = 'all';
    fill();
  });
  fill();

  const strip = h('div', { class: 'strip' }, R.results.map((r, i) => h('button', {
    class: r.correct ? 'ok' : 'ng', type: 'button', 'aria-label': `第 ${i + 1} 题，${r.correct ? '答对' : '答错'}，查看解析`,
    onclick: () => {
      if (S.rvFilter === 'wrong' && r.correct) {
        S.rvFilter = 'all';
        fill();
      }
      document.getElementById('rv-' + i)?.scrollIntoView({ block: 'start' });
    },
  }, h('span', { class: 'n' }, i + 1), svgMark(r.correct))));

  const again = (): void => startSession(shuffle(R.results.map((r) => r.q)), R.mode);
  const retryWrong = (): void => startSession(shuffle(R.results.filter((r) => !r.correct).map((r) => r.q)), 'retry');
  const home = (): void => {
    S.view = 'home';
    render();
    window.scrollTo(0, 0);
  };

  return h('div', { class: 'wrap' },
    h('header', { class: 'top' }, brand(), h('button', { class: 'btn quiet', type: 'button', onclick: home }, '回首页')),
    h('section', { class: 'score' },
      h('div', { class: 'big num', role: 'img', 'aria-label': `得分 ${pct}%` }, String(pct), h('small', null, '%')),
      h('div', { class: 'says' },
        h('p', null, wrong === 0 ? '全部答对。' : `答对 ${good} 题，答错 ${wrong} 题。`),
        h('p', { class: 'dim' }, `共 ${n} 题，用时 ${fmtTime(R.seconds)}` + (R.prev ? `。上一次 ${R.prev.percent}%` : '')),
        h('div', { class: 'acts' },
          wrong ? h('button', { class: 'btn primary', type: 'button', onclick: retryWrong }, '只重做错题') : null,
          h('button', { class: wrong ? 'btn' : 'btn primary', type: 'button', onclick: again }, '再考一次'),
          h('button', { class: 'btn', type: 'button', onclick: home }, '回首页')))),
    strip,
    h('div', { class: 'rvhead' }, h('h2', null, '答案与解析'), h('div', { class: 'seg' }, segW, segA)),
    list);
}
