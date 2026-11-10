// 答题页：题干 + 选项 / 热点小项、日语/中文切换、答题卡、计时、键盘。答题时不显示对错。
import { stateOf, type Answer } from '../domain/grading.ts';
import { kindLabel } from '../components/common.ts';
import { S } from '../state/store.ts';
import {
  elapsed, go, leaveQuiz, onRefresh, pick, pickItem, resetRefreshers, runRefreshers, setLang, submit, toggleFlag,
} from '../state/actions.ts';
import type { Lang } from '../state/store.ts';
import { h, paras } from '../utils/dom.ts';
import { fmtTime } from '../utils/format.ts';

export function renderQuiz(): HTMLElement {
  const se = S.session;
  if (!se) return h('div', { class: 'wrap' });
  const q = se.qs[se.idx];
  if (!q) return h('div', { class: 'wrap' });
  const n = se.qs.length;
  resetRefreshers();

  const answerOf = (id: string): Answer => se.answers[id];
  const answeredNow = (): number => se.qs.filter((x) => stateOf(x, answerOf(x.id)) === 2).length;
  const meterFill = h('i');
  const railCount = h('span', { class: 'num' });
  onRefresh(() => {
    const a = answeredNow();
    meterFill.style.width = (100 * a) / n + '%';
    railCount.textContent = `已答 ${a} / ${n} 题`;
  });

  // 这道题没有可用的中文翻译时，一律显示日语
  const zh = q.zh;
  const hasZh = !!zh;
  const lang: Lang = S.lang === 'zh' && zh ? 'zh' : 'ja';
  const stemText = lang === 'zh' && zh ? zh.stem : q.stem;

  let body: HTMLElement;
  if (q.kind === 'hotspot') {
    body = h('div', { class: 'hs' }, q.items.map((it, i) => h('div', { class: 'hs-item' },
      h('span', { class: 'hs-no' }, i + 1),
      h('p', { lang, class: 'pre' }, lang === 'zh' && zh?.items ? (zh.items[i] ?? it.text) : it.text),
      h('div', { class: 'chips', role: 'group', 'aria-label': `第 ${i + 1} 小项` }, it.choices.map((ch, j) => {
        const btn = h('button', { class: 'chip', type: 'button', 'aria-pressed': 'false', onclick: () => pickItem(q, i, j) },
          h('span', { class: 'dot' }), h('span', { lang: 'ja' }, ch)); // 选项本身（はい / いいえ 等）是答案原文，不翻译
        onRefresh(() => {
          const a = answerOf(q.id);
          btn.setAttribute('aria-pressed', String(Array.isArray(a) && a[i] === j));
        });
        return btn;
      })))));
  } else {
    body = h('div', { class: 'opts' + (q.kind === 'multi' ? ' multi' : ''), role: 'group', 'aria-label': '选项' }, q.options.map((o, i) => {
      const text = lang === 'zh' && zh?.options ? (zh.options[i] ?? o.text) : o.text;
      const btn = h('button', { class: 'opt', type: 'button', 'aria-pressed': 'false', onclick: () => pick(q, o.key) },
        h('span', { class: 'bubble' }, o.key), h('span', { class: 't', lang }, text));
      onRefresh(() => {
        const a = answerOf(q.id);
        btn.setAttribute('aria-pressed', String(Array.isArray(a) && a.includes(o.key)));
      });
      return btn;
    }));
  }

  const langBtn = (code: Lang, label: string): HTMLElement => h('button', {
    class: 'lang-btn', type: 'button', lang: code, 'aria-pressed': String(lang === code),
    disabled: code === 'zh' && !hasZh,
    title: code === 'zh' && !hasZh ? '这道题的 B 列没有中文翻译' : null,
    onclick: () => setLang(code),
  }, label);
  const langSwitch = h('div', { class: 'lang', role: 'group', 'aria-label': '题目语言' }, langBtn('ja', '日语'), langBtn('zh', '中文'));

  const flagBtn = h('button', { class: 'btn flagbtn', type: 'button', 'aria-pressed': 'false', onclick: () => toggleFlag(q) }, '标记这题');
  onRefresh(() => flagBtn.setAttribute('aria-pressed', String(se.flags.has(q.id))));

  const last = se.idx === n - 1;
  const paper = h('article', { class: 'paper' },
    h('div', { class: 'phead' }, h('span', { class: 'badge' }, kindLabel(q)), h('span', null, q.sheet), langSwitch),
    h('div', { class: 'stem' }, paras(stemText, undefined, lang)),
    body,
    h('div', { class: 'pfoot' },
      h('div', { class: 'grp' },
        h('button', { class: 'btn', type: 'button', disabled: se.idx === 0, onclick: () => go(se.idx - 1) }, '上一题'),
        last
          ? h('button', { class: 'btn primary', type: 'button', onclick: () => void submit() }, '交卷')
          : h('button', { class: 'btn primary', type: 'button', onclick: () => go(se.idx + 1) }, '下一题')),
      flagBtn));

  const tiles = h('div', { class: 'tiles' }, se.qs.map((x, i) => {
    const t = h('button', { class: 'tile num', type: 'button', onclick: () => go(i) }, String(i + 1));
    onRefresh(() => {
      const st = stateOf(x, answerOf(x.id));
      const fl = se.flags.has(x.id);
      const cur = i === se.idx;
      t.className = 'tile num' + (st === 2 ? ' done' : st === 1 ? ' part' : '') + (cur ? ' cur' : '') + (fl ? ' flag' : '');
      t.setAttribute('aria-label', `第 ${i + 1} 题，${['未答', '只答了一部分', '已答'][st]}${fl ? '，已标记' : ''}${cur ? '，当前题' : ''}`);
      if (cur) t.setAttribute('aria-current', 'true');
      else t.removeAttribute('aria-current');
    });
    return t;
  }));

  const rail = h('aside', { class: 'rail', 'aria-label': '答题卡' },
    h('h2', null, '答题卡'),
    tiles,
    h('div', { class: 'legend' },
      h('span', null, h('i', { class: 'done' }), '已答'),
      h('span', null, h('i', { class: 'part' }), '热点题只答了一部分'),
      h('span', null, h('i'), '未答'),
      h('span', null, h('i', { class: 'flag' }), '已标记')),
    h('p', { style: 'margin-bottom:12px' }, railCount),
    h('button', { class: 'btn primary', type: 'button', onclick: () => void submit() }, '交卷'),
    h('p', { class: 'keys' }, '← → 切换题目', h('br'), 'A–E 选答案　M 标记'));

  const timerEl = h('span', { class: 'timer num', id: 'timer' }, fmtTime(elapsed()));
  S.timer = window.setInterval(() => {
    timerEl.textContent = fmtTime(elapsed());
  }, 1000);

  runRefreshers();

  return h('div', { class: 'wrap' },
    h('div', { class: 'qbar' },
      h('div', { class: 'l' }, h('b', { class: 'num' }, `第 ${se.idx + 1} 题`), h('span', { class: 'dim num' }, `共 ${n} 题`)),
      h('div', { class: 'r' }, timerEl,
        h('button', { class: 'btn quiet', type: 'button', onclick: leaveQuiz }, '暂存退出'))),
    h('div', { class: 'meter', 'aria-hidden': 'true' }, meterFill),
    h('div', { class: 'quiz-grid' }, paper, rail));
}

/** 键盘：← → 切题，A–H 选答案，M 标记。长按不重复触发，弹窗打开或在输入框里时不响应。 */
export function installQuizKeys(): void {
  document.addEventListener('keydown', (e) => {
    const se = S.session;
    if (S.view !== 'quiz' || !se || e.ctrlKey || e.metaKey || e.altKey || e.repeat) return;
    if (document.querySelector('dialog[open]')) return;
    const t = e.target;
    if (t instanceof HTMLElement && /^(INPUT|TEXTAREA|SELECT)$/.test(t.tagName)) return;
    const q = se.qs[se.idx];
    if (!q) return;
    if (e.key === 'ArrowRight' && se.idx < se.qs.length - 1) {
      e.preventDefault();
      go(se.idx + 1);
    } else if (e.key === 'ArrowLeft' && se.idx > 0) {
      e.preventDefault();
      go(se.idx - 1);
    } else if (e.key === 'm' || e.key === 'M') {
      toggleFlag(q);
    } else if (/^[a-hA-H]$/.test(e.key) && q.kind !== 'hotspot') {
      const k = e.key.toUpperCase();
      if (q.options.some((o) => o.key === k)) pick(q, k);
    }
  });
}
