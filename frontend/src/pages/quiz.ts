// Quiz page: question stem + options / hotspot sub-items, Japanese/Chinese/English switch, answer sheet, timer, keyboard. Correctness is not shown while answering.
import '../styles/quiz.css';
import { stateOf, type Answer } from '../domain/grading.ts';
import { hasLang, shown, type Lang } from '../domain/language.ts';
import { kindLabel } from '../components/common.ts';
import { t, type MsgKey } from '../i18n/index.ts';
import { S } from '../state/store.ts';
import {
  elapsed, go, leaveQuiz, onRefresh, pick, pickItem, resetRefreshers, runRefreshers, setQuestionLang, submit, toggleFlag,
} from '../state/actions.ts';
import { h, paras } from '../utils/dom.ts';
import { fmtTime } from '../utils/format.ts';
import { toast } from '../utils/toast.ts';

/** Names of the question languages, in the current UI language */
const LANG_NAME: Record<Lang, MsgKey> = { en: 'quiz.lang.en', zh: 'quiz.lang.zh', ja: 'quiz.lang.ja' };
/** Why a language button is unavailable (the question lacks that language): its tooltip, and the toast shown when it is tapped */
const LANG_MISSING: Record<Exclude<Lang, 'ja'>, MsgKey> = { zh: 'quiz.lang.noZh', en: 'quiz.lang.noEn' };
/** Answer state of a question (stateOf: 0 = open, 1 = hotspot partly answered, 2 = answered) */
const TILE_STATE: readonly MsgKey[] = ['quiz.tile.open', 'quiz.tile.part', 'quiz.tile.done'];

/** The data-focus key of the focused control: after a redraw (next question, language switch) the same control gets the focus back */
function focusedKey(): string | null {
  const a = document.activeElement;
  return a instanceof HTMLElement ? a.dataset.focus ?? null : null;
}

function restoreFocus(root: HTMLElement, key: string | null): void {
  if (!key) return;
  queueMicrotask(() => {
    if (!root.isConnected) return;
    const el = root.querySelector<HTMLElement>(`[data-focus="${key}"]`);
    if (el && !(el instanceof HTMLButtonElement && el.disabled)) el.focus({ preventScroll: true });
  });
}

export function renderQuiz(): HTMLElement {
  const se = S.session;
  if (!se) return h('div', { class: 'wrap' });
  const q = se.qs[se.idx];
  if (!q) return h('div', { class: 'wrap' });
  const n = se.qs.length;
  const keep = focusedKey();
  resetRefreshers();

  const answerOf = (id: string): Answer => se.answers[id];
  const answeredNow = (): number => se.qs.filter((x) => stateOf(x, answerOf(x.id)) === 2).length;
  const meterFill = h('i');
  const railCount = h('span', { class: 'num' });
  onRefresh(() => {
    const a = answeredNow();
    meterFill.style.width = (100 * a) / n + '%';
    railCount.textContent = t('quiz.answered', { a, n });
  });

  // If the requested language is unavailable for this question (it does not match the Japanese structure), fall back to Japanese instead of showing misaligned content
  const view = shown(q, S.settings.questionLang);
  const lang = view.lang;
  const stemText = view.stem;

  let body: HTMLElement;
  if (q.kind === 'hotspot') {
    body = h('div', { class: 'hs' }, q.items.map((it, i) => h('div', { class: 'hs-item' },
      h('span', { class: 'hs-no' }, i + 1),
      h('p', { lang, class: 'pre' }, view.itemText(i)),
      h('div', { class: 'chips', role: 'group', 'aria-label': t('quiz.item', { i: i + 1 }) }, it.choices.map((_, j) => {
        const btn = h('button', { class: 'chip', type: 'button', 'aria-pressed': 'false', onclick: () => pickItem(q, i, j) },
          h('span', { class: 'dot' }), h('span', { lang: view.choiceLang(i, j) }, view.choiceText(i, j))); // choices use what the current-language column wrote (是 / 否, Yes / No); if nothing is written, use the Japanese one
        onRefresh(() => {
          const a = answerOf(q.id);
          btn.setAttribute('aria-pressed', String(Array.isArray(a) && a[i] === j));
        });
        return btn;
      })))));
  } else {
    body = h('div', { class: 'opts' + (q.kind === 'multi' ? ' multi' : ''), role: 'group', 'aria-label': t('quiz.options') }, q.options.map((o, i) => {
      const text = view.optionText(i);
      const btn = h('button', { class: 'opt', type: 'button', 'aria-pressed': 'false', onclick: () => pick(q, o.key) },
        h('span', { class: 'bubble' }, o.key), h('span', { class: 't', lang }, text));
      onRefresh(() => {
        const a = answerOf(q.id);
        btn.setAttribute('aria-pressed', String(Array.isArray(a) && a.includes(o.key)));
      });
      return btn;
    }));
  }

  // A language the question lacks stays focusable and tappable (aria-disabled, not disabled): a tap says why in a toast,
  // because phones never show the title tooltip and a disabled button would silently ignore the tap
  const langBtn = (code: Lang): HTMLElement => {
    const missing = code === 'ja' || hasLang(q, code) ? null : t(LANG_MISSING[code]);
    return h('button', {
      class: 'lang-btn', type: 'button', 'aria-pressed': String(lang === code), 'data-focus': `lang-${code}`,
      'aria-disabled': missing ? 'true' : null,
      title: missing,
      onclick: () => (missing ? toast(missing) : setQuestionLang(code)),
    }, t(LANG_NAME[code]));
  };
  const langSwitch = h('div', { class: 'lang', role: 'group', 'aria-label': t('quiz.lang.label') }, langBtn('en'), langBtn('zh'), langBtn('ja'));

  const flagBtn = h('button', {
    class: 'btn flagbtn', type: 'button', 'aria-pressed': 'false', 'data-focus': 'q-flag', title: t('quiz.flag.hint'), onclick: () => toggleFlag(q),
  }, t('quiz.flag'));
  onRefresh(() => flagBtn.setAttribute('aria-pressed', String(se.flags.has(q.id))));

  const last = se.idx === n - 1;
  const paper = h('article', { class: 'paper' },
    h('div', { class: 'phead' }, h('span', { class: 'badge' }, kindLabel(q)), h('span', null, q.sheet), langSwitch),
    h('div', { class: 'stem' }, paras(stemText, undefined, lang)),
    body,
    h('div', { class: 'pfoot' },
      h('div', { class: 'grp' },
        h('button', { class: 'btn', type: 'button', disabled: se.idx === 0, 'data-focus': 'q-prev', onclick: () => go(se.idx - 1) }, t('quiz.prev')),
        last
          ? h('button', { class: 'btn primary', type: 'button', 'data-focus': 'q-next', onclick: () => void submit() }, t('quiz.submit'))
          : h('button', { class: 'btn primary', type: 'button', 'data-focus': 'q-next', onclick: () => go(se.idx + 1) }, t('quiz.next'))),
      flagBtn));

  const tiles = h('div', { class: 'tiles' }, se.qs.map((x, i) => {
    const tile = h('button', { class: 'tile num', type: 'button', 'data-focus': `tile-${i}`, onclick: () => go(i) }, String(i + 1));
    onRefresh(() => {
      const st = stateOf(x, answerOf(x.id));
      const fl = se.flags.has(x.id);
      const cur = i === se.idx;
      tile.className = 'tile num' + (st === 2 ? ' done' : st === 1 ? ' part' : '') + (cur ? ' cur' : '') + (fl ? ' flag' : '');
      const parts = [t('quiz.tile.q', { i: i + 1 }), t(TILE_STATE[st] ?? 'quiz.tile.open')];
      if (fl) parts.push(t('quiz.tile.flagged'));
      if (cur) parts.push(t('quiz.tile.current'));
      tile.setAttribute('aria-label', parts.join(t('quiz.tile.sep')));
      if (cur) tile.setAttribute('aria-current', 'true');
      else tile.removeAttribute('aria-current');
    });
    return tile;
  }));

  const rail = h('aside', { class: 'rail', 'aria-labelledby': 'rail-h' },
    h('h2', { id: 'rail-h' }, t('quiz.sheet')),
    tiles,
    h('div', { class: 'legend' },
      h('span', null, h('i', { class: 'done' }), t('quiz.legend.done')),
      h('span', null, h('i', { class: 'part' }), t('quiz.legend.part')),
      h('span', null, h('i'), t('quiz.legend.open')),
      h('span', null, h('i', { class: 'flag' }), t('quiz.legend.flag'))),
    h('p', { class: 'rail-count' }, railCount),
    h('button', { class: 'btn primary', type: 'button', 'data-focus': 'rail-submit', onclick: () => void submit() }, t('quiz.submit')),
    h('p', { class: 'keys' }, t('quiz.keys.nav'), h('br'), t('quiz.keys.pick')));

  const timerEl = h('span', { class: 'timer num', id: 'timer' }, fmtTime(elapsed()));
  S.timer = window.setInterval(() => {
    timerEl.textContent = fmtTime(elapsed());
  }, 1000);

  // Auto-advance (a setting): a small reminder in the top bar, so a jump to the next question does not come as a surprise
  const autoTag = S.settings.autoAdvance
    ? h('span', { class: 'qauto', title: t('quiz.auto.hint') }, h('span', { class: 'qauto-dot', 'aria-hidden': 'true' }), t('quiz.auto.on'))
    : null;

  runRefreshers();

  const root = h('div', { class: 'wrap' },
    h('div', { class: 'qbar' },
      h('h1', { class: 'l' }, h('b', { class: 'num' }, t('quiz.position', { i: se.idx + 1 })), h('span', { class: 'dim num' }, t('quiz.of', { n }))),
      h('div', { class: 'r' },
        autoTag,
        h('span', { class: 'qtime', title: t('quiz.time') }, h('span', { class: 'sr-only' }, t('quiz.time')), timerEl),
        h('button', { class: 'btn quiet', type: 'button', title: t('quiz.saveExit.hint'), onclick: leaveQuiz }, t('quiz.saveExit')))),
    h('div', { class: 'meter', 'aria-hidden': 'true' }, meterFill),
    h('div', { class: 'quiz-grid' }, paper, rail));
  restoreFocus(root, keep);
  return root;
}

/** Keyboard: ← → switch questions, A–H pick an answer, M flags. Holding a key does not repeat; ignored while a dialog is open or an input is focused. */
export function installQuizKeys(): void {
  document.addEventListener('keydown', (e) => {
    const se = S.session;
    if (S.route.name !== 'quiz' || !se || e.ctrlKey || e.metaKey || e.altKey || e.repeat) return;
    if (document.querySelector('dialog[open]')) return;
    const target = e.target;
    if (target instanceof HTMLElement && /^(INPUT|TEXTAREA|SELECT)$/.test(target.tagName)) return;
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
