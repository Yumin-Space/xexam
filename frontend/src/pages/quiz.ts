// Quiz page: question stem + options / hotspot sub-items, the English / 中文 / 日本語 switch, answer sheet, timer, keyboard.
// Two kinds (v0.6.5):
//   · exam: correctness is not shown while answering; Submit grades everything at the end;
//   · practice: immediate feedback. A single-choice question is checked as soon as an option is chosen, multiple-choice and
//     hotspot questions have a "Check answer" button. A checked question is locked and shows right / wrong, the correct
//     answer, the explanation and the memory aid (in the language of the page's switch); the answer sheet marks it right or
//     wrong. "Finish practice" saves it like an exam (kind "practice"). Auto-advance does not apply.
import '../styles/quiz.css';
import { canCheck, checksOnPick, practiceCounts, practiceMark, type PracticeMark } from '../domain/practice.ts';
import { aiControls } from '../domain/ai-plus.ts';
import { grade, stateOf, type Answer } from '../domain/grading.ts';
import { htmlLang, shown } from '../domain/language.ts';
import { aiPlusNow, naTag, pheadRight, studyButton, studyIcon, studyState, type StudyTarget } from '../components/ai-plus-controls.ts';
import { kindLabel } from '../components/common.ts';
import { langSwitch } from '../components/lang-switch.ts';
import { answerReview, explainBlocks } from '../components/review.ts';
import { t, type MsgKey } from '../i18n/index.ts';
import { S, type Session } from '../state/store.ts';
import {
  checkAnswer, elapsed, go, leaveQuiz, onRefresh, pick, pickItem, resetRefreshers, runRefreshers, setQuestionLang, submit, toggleFlag,
} from '../state/actions.ts';
import type { Question } from '../types/api.ts';
import { h, paras, svgMark } from '../utils/dom.ts';
import { fmtTime } from '../utils/format.ts';

/** Answer state of a question in an exam (stateOf: 0 = open, 1 = hotspot partly answered, 2 = answered) */
const TILE_STATE: readonly MsgKey[] = ['quiz.tile.open', 'quiz.tile.part', 'quiz.tile.done'];
/** State of a question in a practice */
const PRACTICE_TILE: Record<PracticeMark, MsgKey> = { open: 'quiz.tile.unchecked', right: 'quiz.tile.right', wrong: 'quiz.tile.wrong' };

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

/**
 * Practice: the checked questions whose feedback was already shown. A question checked since the last drawing gets the focus
 * on its verdict, so a screen reader reads "Correct" / "Wrong" at once (the redraw would otherwise drop the focus). A
 * continued practice starts with every checked question as already shown.
 */
let announcedFor: Session | null = null;
const announced = new Set<string>();

function freshlyChecked(se: Session, id: string): boolean {
  if (announcedFor !== se) {
    announcedFor = se;
    announced.clear();
    for (const x of se.checked) announced.add(x);
  }
  if (!se.checked.has(id) || announced.has(id)) return false;
  announced.add(id);
  return true;
}

/** Where the AI hint / explanation of a practice question belongs: this session's chat (v0.9) */
function practiceTarget(se: Session, q: Question): StudyTarget {
  const target: StudyTarget = {
    page: 'practice', mode: 'practice', key: se.key, startedAt: se.startedAt, recordId: null, questionId: q.id, no: se.idx + 1,
    // read when the ask starts: the card's language switch may have changed it since the popover opened
    qlang: () => shown(q, S.settings.questionLang).lang,
    // [查看] on a busy note: only the answer of the question on the page can be shown here
    resolve: (r) => (r.questionId === q.id && r.chatKind === 'practice' ? target : null),
  };
  return target;
}

export function renderQuiz(): HTMLElement {
  const se = S.session;
  if (!se) return h('div', { class: 'wrap' });
  const q = se.qs[se.idx];
  if (!q) return h('div', { class: 'wrap' });
  const n = se.qs.length;
  const practice = se.kind === 'practice';
  const checked = practice && se.checked.has(q.id);
  const fresh = practice && freshlyChecked(se, q.id);
  const keep = focusedKey();
  resetRefreshers();

  const answerOf = (id: string): Answer => se.answers[id];
  const meterFill = h('i');
  const railCount = h('span', { class: 'num' });
  onRefresh(() => {
    if (practice) {
      const c = practiceCounts(se.qs, se.answers, se.checked);
      meterFill.style.width = (100 * c.checked) / n + '%';
      railCount.textContent = t('quiz.practice.count', { c: c.checked, n, r: c.right });
    } else {
      const a = se.qs.filter((x) => stateOf(x, answerOf(x.id)) === 2).length;
      meterFill.style.width = (100 * a) / n + '%';
      railCount.textContent = t('quiz.answered', { a, n });
    }
  });

  // If the requested language is unavailable for this question (it does not line up with the base text), fall back to the
  // question's base language instead of showing misaligned content
  const view = shown(q, S.settings.questionLang);
  const lang = htmlLang(view.lang);

  let body: HTMLElement;
  let feedback: HTMLElement | null = null;
  let verdictEl: HTMLElement | null = null;
  if (checked) {
    // Practice, checked: the options marked right / wrong (like the review cards), then the verdict and the explanation
    const ans = answerOf(q.id);
    const ok = grade(q, ans);
    const parts = answerReview(q, view, ans, ok);
    body = parts.body;
    verdictEl = h('p', { class: 'pverdict ' + (ok ? 'ok' : 'ng'), tabindex: '-1' }, svgMark(ok), ok ? t('quiz.practice.right') : t('quiz.practice.wrong'));
    feedback = h('div', { class: 'pfeed' }, verdictEl, parts.verdict, explainBlocks(q, view.lang));
  } else if (q.kind === 'hotspot') {
    body = h('div', { class: 'hs' }, q.items.map((it, i) => h('div', { class: 'hs-item' },
      h('span', { class: 'hs-no' }, i + 1),
      h('p', { lang, class: 'pre' }, view.itemText(i)),
      h('div', { class: 'chips', role: 'group', 'aria-label': t('quiz.item', { i: i + 1 }) }, it.choices.map((_, j) => {
        const btn = h('button', { class: 'chip', type: 'button', 'aria-pressed': 'false', onclick: () => pickItem(q, i, j) },
          // choices use what the current-language column wrote (是 / 否, Yes / No); if nothing is written, the base language's
          h('span', { class: 'dot' }), h('span', { lang: htmlLang(view.choiceLang(i, j)) }, view.choiceText(i, j)));
        onRefresh(() => {
          const a = answerOf(q.id);
          btn.setAttribute('aria-pressed', String(Array.isArray(a) && a[i] === j));
        });
        return btn;
      })))));
  } else {
    body = h('div', { class: 'opts' + (q.kind === 'multi' ? ' multi' : ''), role: 'group', 'aria-label': t('quiz.options') }, q.options.map((o, i) => {
      const btn = h('button', { class: 'opt', type: 'button', 'aria-pressed': 'false', onclick: () => pick(q, o.key) },
        h('span', { class: 'bubble' }, o.key), h('span', { class: 't', lang }, view.optionText(i)));
      onRefresh(() => {
        const a = answerOf(q.id);
        btn.setAttribute('aria-pressed', String(Array.isArray(a) && a.includes(o.key)));
      });
      return btn;
    }));
  }

  const flagBtn = h('button', {
    class: 'btn flagbtn', type: 'button', 'aria-pressed': 'false', 'data-focus': 'q-flag', title: t('quiz.flag.hint'), onclick: () => toggleFlag(q),
  }, t('quiz.flag'));
  onRefresh(() => flagBtn.setAttribute('aria-pressed', String(se.flags.has(q.id))));

  // Practice, multiple choice / hotspot, not checked yet: "Check answer", usable once something is chosen. It is the main
  // action of the question until it is used; Next / Finish then take over.
  let checkBtn: HTMLButtonElement | null = null;
  if (practice && !checked && !checksOnPick(q)) {
    const btn = h('button', { class: 'btn primary', type: 'button', 'data-focus': 'q-check', onclick: () => checkAnswer(q) }, t('quiz.practice.check'));
    onRefresh(() => {
      btn.disabled = !canCheck(q, answerOf(q.id), se.checked.has(q.id));
    });
    checkBtn = btn;
  }
  // v0.9: the AI hint / explanation controls of a practice (the exam has none), for users with the AI+ feature only
  let headRight: HTMLElement = langSwitch(q, view.lang, (l) => setQuestionLang(l), { focusKey: 'lang' });
  let aiBar: HTMLElement | null = null;
  const [hintSpec, explainSpec] = aiControls('practice');
  if (practice && hintSpec && explainSpec && aiPlusNow().show) {
    const target = practiceTarget(se, q);
    const state = studyState(target);
    headRight = pheadRight([studyIcon(target, hintSpec, state), studyIcon(target, explainSpec, state)], naTag(state), headRight);
    aiBar = h('div', { class: 'grp aip-grp' },
      studyButton(target, hintSpec, state, { bar: true }), studyButton(target, explainSpec, state, { bar: true }), naTag(state));
  }
  const last = se.idx === n - 1;
  const finishLabel = practice ? t('quiz.practice.finish') : t('quiz.submit');
  const nextCls = checkBtn ? 'btn' : 'btn primary';
  const paper = h('article', { class: 'paper' },
    h('div', { class: 'phead' },
      h('span', { class: 'badge' }, kindLabel(q)),
      h('span', null, q.sheet),
      headRight),
    h('div', { class: 'stem' }, paras(view.stem, undefined, lang)),
    body,
    feedback,
    h('div', { class: 'pfoot' },
      h('div', { class: 'grp' },
        h('button', { class: 'btn', type: 'button', disabled: se.idx === 0, 'data-focus': 'q-prev', onclick: () => go(se.idx - 1) }, t('quiz.prev')),
        checkBtn,
        last
          ? h('button', { class: nextCls, type: 'button', 'data-focus': 'q-next', onclick: () => void submit() }, finishLabel)
          : h('button', { class: nextCls, type: 'button', 'data-focus': 'q-next', onclick: () => go(se.idx + 1) }, t('quiz.next'))),
      aiBar,
      flagBtn));

  const tiles = h('div', { class: 'tiles' }, se.qs.map((x, i) => {
    const tile = h('button', { class: 'tile num', type: 'button', 'data-focus': `tile-${i}`, onclick: () => go(i) }, String(i + 1));
    onRefresh(() => {
      const a = answerOf(x.id);
      const fl = se.flags.has(x.id);
      const cur = i === se.idx;
      let cls: string;
      let state: MsgKey;
      if (practice) {
        const mark = practiceMark(x, a, se.checked.has(x.id));
        cls = mark === 'right' ? ' ok' : mark === 'wrong' ? ' ng' : '';
        state = PRACTICE_TILE[mark];
      } else {
        const st = stateOf(x, a);
        cls = st === 2 ? ' done' : st === 1 ? ' part' : '';
        state = TILE_STATE[st] ?? 'quiz.tile.open';
      }
      tile.className = 'tile num' + cls + (cur ? ' cur' : '') + (fl ? ' flag' : '');
      const parts = [t('quiz.tile.q', { i: i + 1 }), t(state)];
      if (fl) parts.push(t('quiz.tile.flagged'));
      if (cur) parts.push(t('quiz.tile.current'));
      tile.setAttribute('aria-label', parts.join(t('quiz.tile.sep')));
      if (cur) tile.setAttribute('aria-current', 'true');
      else tile.removeAttribute('aria-current');
    });
    return tile;
  }));

  const legend = practice
    ? h('div', { class: 'legend' },
      h('span', null, h('i', { class: 'ok' }), t('quiz.legend.right')),
      h('span', null, h('i', { class: 'ng' }), t('quiz.legend.wrong')),
      h('span', null, h('i'), t('quiz.legend.unchecked')),
      h('span', null, h('i', { class: 'flag' }), t('quiz.legend.flag')))
    : h('div', { class: 'legend' },
      h('span', null, h('i', { class: 'done' }), t('quiz.legend.done')),
      h('span', null, h('i', { class: 'part' }), t('quiz.legend.part')),
      h('span', null, h('i'), t('quiz.legend.open')),
      h('span', null, h('i', { class: 'flag' }), t('quiz.legend.flag')));

  const rail = h('aside', { class: 'rail', 'aria-labelledby': 'rail-h' },
    h('h2', { id: 'rail-h' }, t('quiz.sheet')),
    tiles,
    legend,
    h('p', { class: 'rail-count' }, railCount),
    h('button', { class: 'btn primary', type: 'button', 'data-focus': 'rail-submit', onclick: () => void submit() }, finishLabel),
    h('p', { class: 'keys' }, t('quiz.keys.nav'), h('br'), t('quiz.keys.pick')));

  const timerEl = h('span', { class: 'timer num', id: 'timer' }, fmtTime(elapsed()));
  S.timer = window.setInterval(() => {
    timerEl.textContent = fmtTime(elapsed());
  }, 1000);

  // Auto-advance (a setting, exams only): a small reminder in the top bar, so a jump to the next question does not come as a surprise
  const autoTag = !practice && S.settings.autoAdvance
    ? h('span', { class: 'qauto', title: t('quiz.auto.hint') }, h('span', { class: 'qauto-dot', 'aria-hidden': 'true' }), t('quiz.auto.on'))
    : null;
  // Practice: a tag next to the position, so it is clear that answers are checked one by one
  const kindTag = practice ? h('span', { class: 'tag b qkind', title: t('quiz.practice.hint') }, t('quiz.practice.tag')) : null;

  runRefreshers();

  const root = h('div', { class: 'wrap' + (practice ? ' practice' : '') },
    h('div', { class: 'qbar' },
      h('h1', { class: 'l' }, h('b', { class: 'num' }, t('quiz.position', { i: se.idx + 1 })), h('span', { class: 'dim num' }, t('quiz.of', { n })), kindTag),
      h('div', { class: 'r' },
        autoTag,
        h('span', { class: 'qtime', title: t('quiz.time') }, h('span', { class: 'sr-only' }, t('quiz.time')), timerEl),
        h('button', { class: 'btn quiet', type: 'button', title: t(practice ? 'quiz.saveExit.hintPractice' : 'quiz.saveExit.hint'), onclick: leaveQuiz }, t('quiz.saveExit')))),
    h('div', { class: 'meter', 'aria-hidden': 'true' }, meterFill),
    h('div', { class: 'quiz-grid' }, paper, rail));
  if (fresh && verdictEl) {
    const v = verdictEl;
    queueMicrotask(() => {
      if (v.isConnected) v.focus({ preventScroll: true });
    });
  } else {
    restoreFocus(root, keep);
  }
  return root;
}

/** Keyboard: ← → switch questions, A–H pick an answer, M flags. Holding a key does not repeat; ignored while a dialog is open or an input is focused. */
export function installQuizKeys(): void {
  document.addEventListener('keydown', (e) => {
    const se = S.session;
    if (S.route.name !== 'quiz' || !se || S.authView || e.ctrlKey || e.metaKey || e.altKey || e.repeat) return;
    if (document.querySelector('dialog[open]')) return;
    const target = e.target;
    if (target instanceof HTMLElement && /^(INPUT|TEXTAREA|SELECT)$/.test(target.tagName)) return;
    // v0.9: keys typed inside the AI popover (its buttons, the answer text) are not quiz shortcuts
    if (target instanceof Element && target.closest('.aip-pop')) return;
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
      // pick() ignores a checked (locked) practice question; in a practice it checks a single-choice question at once
      const k = e.key.toUpperCase();
      if (q.options.some((o) => o.key === k)) pick(q, k);
    }
  });
}
