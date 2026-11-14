// Quiz page: question stem + options / hotspot sub-items, the English / 中文 / 日本語 switch, answer sheet, timer, keyboard.
// Two kinds (v0.6.5):
//   · exam: correctness is not shown while answering; Submit grades everything at the end;
//   · practice: immediate feedback. Every question (single, multiple choice, hotspot) has a "Check answer" button (v1.1: a
//     pick alone never checks). A checked question is locked and shows right / wrong, the correct
//     answer, the explanation and the memory aid (in the language of the page's switch); the answer sheet marks it right or
//     wrong. "Finish practice" saves it like an exam (kind "practice"). Auto-advance does not apply.
import '../styles/quiz.css';
import { githubLine } from '../components/github-line.ts';
import { barSlots, practiceCounts, practiceMark, sheetMatches, type BarAct, type PracticeMark, type SheetFilter } from '../domain/practice.ts';
import { aiControls } from '../domain/ai-plus.ts';
import { countdownText, remaining, timerTone } from '../domain/exam-timer.ts';
import { grade, stateOf, type Answer } from '../domain/grading.ts';
import { htmlLang, shown } from '../domain/language.ts';
import { studyButton, studyDock, studyState, type StudyTarget } from '../components/ai-plus-controls.ts';
import { aiSetMark, kindLabel } from '../components/common.ts';
import { shortLabel } from '../components/icon-button.ts';
import { langSwitch } from '../components/lang-switch.ts';
import { miniIcon, type MiniIcon } from '../components/mini-icons.ts';
import { answerReview, explainBlocks } from '../components/review.ts';
import { holdTip } from '../components/tooltip.ts';
import { t, type MsgKey } from '../i18n/index.ts';
import { S, type Session } from '../state/store.ts';
import { sheetOpenKey } from '../state/storage-keys.ts';
import {
  checkAnswer, elapsed, examTick, go, leaveQuiz, onRefresh, pick, pickItem, resetRefreshers, runRefreshers, setQuestionLang, submit, toggleFlag,
} from '../state/actions.ts';
import type { Question } from '../types/api.ts';
import { h, paras, svgMark } from '../utils/dom.ts';
import { fmtTime } from '../utils/format.ts';
import { BAR_MQ, PHONE_MQ, matches } from '../utils/media.ts';
import { liftToastAbove } from '../utils/toast.ts';

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
  // A-OWNER-24b / 25: up to the rail line (BAR_MQ) the top row shows the answered count ("已完成 1/50"; up to 360 px only "1/50")
  const topCountFull = h('span', { class: 'qcount-l' });
  const topCountShort = h('span', { class: 'qcount-s num', 'aria-hidden': 'true' });
  const topCount = h('span', { class: 'qcount num', 'data-focus': 'q-count' }, topCountFull, topCountShort);
  const flagsPart = (): string => (se.flags.size > 0 ? t('quiz.sum.flags', { f: se.flags.size }) : '');
  onRefresh(() => {
    const answered = se.qs.filter((x) => stateOf(x, answerOf(x.id)) === 2).length;
    topCountFull.textContent = t('quiz.top.answered', { a: answered, n });
    topCountShort.textContent = `${answered}/${n}`;
    if (practice) {
      const c = practiceCounts(se.qs, se.answers, se.checked);
      meterFill.style.width = (100 * c.checked) / n + '%';
      railCount.textContent = t('quiz.sum.practice', { c: c.checked, n, r: c.right }) + flagsPart();
    } else {
      const a = answered;
      meterFill.style.width = (100 * a) / n + '%';
      railCount.textContent = t('quiz.sum.exam', { a, n }) + flagsPart();
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
        const btn = h('button', { class: 'sheet-chip', type: 'button', 'aria-pressed': 'false', onclick: () => pickItem(q, i, j) },
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

  // ---- the action bar (QZ-3): one DOM at every width, the slot logic is domain/practice.ts barSlots ----
  const [hintSpec, explainSpec] = aiControls('practice');
  const target = practice ? practiceTarget(se, q) : null;
  const aiState = target && hintSpec && explainSpec ? studyState(target) : null;
  const aiOn = !!aiState && aiState.show;
  const slotsOf = (): ReturnType<typeof barSlots> => barSlots({
    kind: se.kind, qType: q.kind, answered: stateOf(q, answerOf(q.id)) > 0, checked, first: se.idx === 0, last: se.idx === n - 1,
    ai: !aiOn ? 'none' : aiState.usable ? 'usable' : 'greyed',
  });
  const plan = slotsOf();
  const finishLabel = practice ? t('quiz.practice.finish') : t('quiz.top.finishExam');

  // the main area's controls: Check answer and Next (Finish practice / Submit on the last question) are two separate buttons,
  // each with its own fixed place; Check answer is greyed (never hidden) when there is nothing to check
  const mainItem = (act: BarAct, enabled: boolean): HTMLElement => {
    if (act === 'check') {
      const btn = h('button', { class: 'btn primary', type: 'button', 'data-act': 'check', 'data-slot': 'check', 'data-focus': 'q-check', onclick: () => checkAnswer(q) },
        shortLabel('quiz.practice.check', 'quiz.practice.checkShort'));
      btn.disabled = !enabled;
      onRefresh(() => {
        btn.disabled = !(slotsOf().main.find((x) => x.act === 'check')?.enabled ?? false);
      });
      return btn;
    }
    if (act === 'next') {
      return h('button', {
        class: 'btn', type: 'button', 'data-act': 'next', 'data-slot': 'next', 'data-focus': 'q-next', disabled: !enabled, onclick: () => go(se.idx + 1),
      }, nextOrFinish(false, !practice));
    }
    return h('button', {
      class: 'btn primary', type: 'button', 'data-act': act, 'data-slot': 'next', 'data-focus': 'q-next', onclick: () => void submit(),
    }, nextOrFinish(true, act !== 'finish'));
  };

  const pfoot = h('div', { class: 'pfoot' });
  for (const sl of plan.slots) {
    if (sl.slot === 'prev') {
      pfoot.append(barButton({
        slot: 'prev', icon: 'back', label: t('quiz.prev'), focus: 'q-prev', disabled: !sl.enabled, onClick: () => go(se.idx - 1),
      }));
    } else if (sl.slot === 'main') {
      pfoot.append(h('div', { class: 'bar-main', 'data-slot': 'main' }, plan.main.map((it) => mainItem(it.act, it.enabled))));
    } else if (sl.slot === 'flag') {
      const flagBtn = barButton({
        slot: 'flag', icon: 'flag', label: t('quiz.flag'), tip: t('quiz.flag.tip', { hint: t('quiz.flag.hint') }), focus: 'q-flag', cls: 'flagbtn', pressed: false,
        onClick: () => toggleFlag(q),
      });
      onRefresh(() => flagBtn.setAttribute('aria-pressed', String(se.flags.has(q.id))));
      pfoot.append(flagBtn);
    } else if (target && aiState && hintSpec && explainSpec) {
      // practice, AI+: the two study buttons (components/ai-plus-controls.ts builds them: icon + AI mark + label, CSS decides what shows)
      const btn = studyButton(target, sl.slot === 'hint' ? hintSpec : explainSpec, aiState, { bar: true });
      if (btn) {
        if (!btn.dataset.slot) btn.dataset.slot = sl.slot;
        pfoot.append(btn);
      }
    }
  }

  const paper = h('article', { class: 'paper' },
    h('div', { class: 'phead' },
      h('span', { class: 'badge' }, kindLabel(q)),
      h('span', { class: 'phead-sheet', title: q.sheet }, q.sheet),
      aiSetMark(q.bank),
      langSwitch(q, view.lang, (l) => setQuestionLang(l), { focusKey: 'lang' })),
    h('div', { class: 'stem' }, paras(view.stem, undefined, lang)),
    body,
    feedback);
  // The AI+ dock of a practice sits right under the card (the panel itself is built by components/ai-panel.ts)
  const dock = target && hintSpec && aiState ? studyDock(target, hintSpec, aiState) : null;
  // Up to 880 px (or a phone turned sideways) the bar sits under the panel and stays pinned while the card or the panel is
  // on screen; wider, it is the foot of the card
  const barPinned = matches(BAR_MQ);
  if (!barPinned) {
    paper.append(pfoot);
    fitWideBar(pfoot);
  }

  // QZ-5 filter chips: all / open / (practice) wrong / marked, over the tiles
  let filter: SheetFilter = 'all';
  const mark0 = (x: Question, a: Answer): boolean => (practice ? !se.checked.has(x.id) : stateOf(x, a) < 2);
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
      tile.hidden = !sheetMatches(filter, { open: mark0(x, a), wrong: cls === ' ng', flagged: fl });
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

  const chipKinds: SheetFilter[] = practice ? ['all', 'wrong', 'open', 'flag'] : ['all', 'open', 'flag'];
  const chipBtns = chipKinds.map((f) => {
    const b = h('button', { class: 'sheet-chip', type: 'button', 'aria-pressed': 'false', 'data-focus': `chip-${f}` });
    b.addEventListener('click', () => {
      filter = f;
      runRefreshers();
    });
    return b;
  });
  onRefresh(() => {
    chipKinds.forEach((f, i) => {
      const n = se.qs.filter((x) => {
        const a = answerOf(x.id);
        return sheetMatches(f, {
          open: mark0(x, a), wrong: practice && se.checked.has(x.id) && practiceMark(x, a, true) === 'wrong', flagged: se.flags.has(x.id),
        });
      }).length;
      const b = chipBtns[i]!;
      b.textContent = t(`quiz.chip.${f}`, { n });
      b.setAttribute('aria-pressed', String(f === filter));
      b.disabled = n === 0 && f !== 'all';
      if (b.disabled && f === filter) filter = 'all';
    });
  });
  const chips = h('div', { class: 'sheet-chips', role: 'group', 'aria-label': t('quiz.chip.group') }, chipBtns);

  // The answer sheet (QZ-5): a summary row that is always there (title, counts, Submit / Finish, and on phones the chevron)
  // above the body (tiles, legend, key hints). Phones fold the body (remembered per user and browser); 601–880 px and
  // desktop always show it (CSS only: the same DOM at every width).
  const sheetBody = h('div', { class: 'rail-body', id: 'rail-body' }, chips, tiles, legend, h('p', { class: 'keys' }, t('quiz.keys.nav'), h('br'), t('quiz.keys.pick')));
  const toggle = h('button', {
    class: 'rail-toggle', type: 'button', 'aria-expanded': 'false', 'aria-controls': 'rail-body', 'aria-label': t('quiz.sheet'), 'data-focus': 'rail-toggle',
  }, miniIcon('chevDown', 'mi'));
  holdTip(toggle, t('quiz.sheet'));
  // A-OWNER-25: the summary row's finish button belongs to the side rail (> 880 px); under BAR_MQ the CSS hides it and the
  // top row's 交卷 / 完成 button takes its place
  const finishBtn = h('button', { class: 'btn primary', type: 'button', 'data-focus': 'rail-submit', onclick: () => void submit() }, finishLabel);
  const rail = h('aside', { class: 'rail', 'aria-labelledby': 'rail-h' },
    h('div', { class: 'rail-sum' },
      h('div', { class: 'rail-t' }, h('h2', { id: 'rail-h' }, t('quiz.sheet')), h('p', { class: 'rail-count' }, railCount)),
      finishBtn,
      toggle),
    sheetBody);
  const setSheetOpen = (open: boolean): void => {
    rail.classList.toggle('open', open);
    toggle.setAttribute('aria-expanded', String(open));
  };
  setSheetOpen(readSheetOpen());
  toggle.addEventListener('click', () => {
    const open = !rail.classList.contains('open');
    setSheetOpen(open);
    writeSheetOpen(open);
  });

  // TL-3: a timed exam shows the time left (countdown); an untimed exam and a practice keep the count-up
  const limit = !practice ? se.limit : null;
  const timerEl = h('span', { class: 'timer num', id: 'timer' });
  const paintTimer = (): void => {
    if (limit == null) {
      timerEl.textContent = fmtTime(elapsed());
      return;
    }
    const left = remaining(limit, elapsed());
    timerEl.textContent = countdownText(left);
    timerEl.className = 'timer num' + (timerTone(limit, left) === 'normal' ? '' : ' ' + timerTone(limit, left));
  };
  paintTimer();
  S.timer = window.setInterval(() => {
    paintTimer();
    if (limit != null) examTick();
  }, 1000);

  // Auto-advance (a setting, exams only): a reminder in the top row (a text tag; on phones a small dot, the text stays for
  // screen readers), so a jump to the next question does not come as a surprise
  const autoTag = !practice && S.settings.autoAdvance
    ? h('span', { class: 'qauto', title: t('quiz.auto.hint') }, h('span', { class: 'qauto-dot', 'aria-hidden': 'true' }), h('span', { class: 'qauto-l' }, t('quiz.auto.on')))
    : null;
  // Practice: a tag next to the position, so it is clear that answers are checked one by one
  const kindTag = practice ? h('span', { class: 'tag b qkind', title: t('quiz.practice.hint') }, t('quiz.practice.tag')) : null;

  // Save and exit: the text button on wide screens, an icon on phones (QZ-1); the hint is its tip
  const exit = h('button', { class: 'btn quiet qexit', type: 'button', 'aria-label': t('quiz.saveExit'), 'data-focus': 'q-exit', onclick: leaveQuiz },
    miniIcon('exit', 'mi'), h('span', { class: 'lbl' }, t('quiz.saveExit')));
  holdTip(exit, t('quiz.saveExit.tip', { hint: t(practice ? 'quiz.saveExit.hintPractice' : 'quiz.saveExit.hint') }));

  // A-OWNER-22 / 25 / 81: under BAR_MQ one small finish button left of the clock (icon + label, never hidden; when the row does
  // not fit, .tight swaps the full label for the short one: en "Finish", ja 「終了」, zh stays 4 characters); it opens the same confirm dialog as the rail button and the last question's bar button
  const topSubmit = h('button', {
    class: `btn qsubmit ${practice ? 'qsubmit-practice' : 'qsubmit-exam'}`, type: 'button', 'aria-label': finishLabel, 'data-focus': 'q-submit', onclick: () => void submit(),
  }, miniIcon('submit', 'mi'), h('span', { class: 'lbl', 'aria-hidden': 'true' }, t(practice ? 'quiz.top.finish' : 'quiz.top.submit')),
    h('span', { class: 'lbl-s', 'aria-hidden': 'true' }, t(practice ? 'quiz.top.finishS' : 'quiz.top.submitS')));
  holdTip(topSubmit, finishLabel);

  runRefreshers();

  const qbar = h('div', { class: 'qbar' },
    h('div', { class: 'ql' },
      h('h1', { class: 'l', 'aria-label': `${t('quiz.position', { i: se.idx + 1 })} ${t('quiz.of', { n })}` },
        h('span', { class: 'pos-full' }, h('b', { class: 'num' }, t('quiz.position', { i: se.idx + 1 })), h('span', { class: 'dim num' }, t('quiz.of', { n }))),
        h('span', { class: 'pos-short', 'aria-hidden': 'true' }, h('b', { class: 'num' }, se.idx + 1), h('span', { class: 'dim num' }, ` / ${n}`))),
      kindTag),
    h('div', { class: 'r' },
      autoTag,
      topCount,
      topSubmit,
      h('span', { class: 'qtime' + (limit != null ? ' timed' : ''), title: limit != null ? t('quiz.timeLeftTip', { min: Math.round(limit / 60) }) : t('quiz.time') },
        miniIcon(limit != null ? 'hourglass' : 'clock', 'mi'), h('span', { class: 'sr-only' }, t(limit != null ? 'quiz.timeLeft' : 'quiz.time')), timerEl),
      exit),
    h('div', { class: 'meter', 'aria-hidden': 'true' }, meterFill));

  const root = h('div', { class: 'wrap qz' + (practice ? ' practice' : '') },
    qbar,
    h('div', { class: 'quiz-grid' }, h('div', { class: 'quiz-main' }, paper, dock, barPinned ? pfoot : null), rail),
    githubLine(),
    // TL-4: the 30-minute notice (assertive) and the 5 / 1 minute lines (polite) for screen readers; examTick() writes them
    limit != null ? h('div', { id: 'quiz-sr-alert', class: 'sr-only', role: 'alert', 'aria-live': 'assertive' }) : null,
    limit != null ? h('div', { id: 'quiz-sr', class: 'sr-only', 'aria-live': 'polite' }) : null,
    se.timeUp ? timeUpOverlay() : null);
  if (fresh && verdictEl) {
    const v = verdictEl;
    queueMicrotask(() => {
      if (v.isConnected) v.focus({ preventScroll: true });
    });
  } else {
    restoreFocus(root, keep);
  }
  // the toasts sit above the pinned bar (QZ-6); --top-h tells the AI+ panel's scroll how much is pinned at the top (2.12 e)
  if (barPinned) liftToastAbove(pfoot);
  topRow = qbar;
  syncTopH();
  window.requestAnimationFrame(syncTopH);
  return root;
}

/** TL-5 step 3: the locked page - a full-page overlay with the title focused and no buttons while the answers are saved */
function timeUpOverlay(): HTMLElement {
  const title = h('h2', { id: 'timeup-title', tabindex: '-1' }, t('quiz.timeUp.title'));
  const box = h('div', { class: 'timeup', role: 'alertdialog', 'aria-modal': 'true', 'aria-labelledby': 'timeup-title', 'aria-describedby': 'timeup-body' },
    h('div', { class: 'timeup-card' }, title, h('p', { id: 'timeup-body' }, t('quiz.timeUp.saving'))));
  queueMicrotask(() => {
    if (title.isConnected) title.focus({ preventScroll: true });
  });
  return box;
}

/**
 * The wide bar (QZ-3, > 880 px) is one row: when its labels do not fit (a long language, a narrow window), Previous and
 * Mark to revisit become icons first (pf-c1), then the two AI buttons too (pf-c2). Each keeps its name (aria-label) and
 * its tip. Measured, not guessed per language: the bar is re-fitted whenever its width changes. The result does not depend
 * on the question (the Next place has a fixed width), so the places stay the same on every question.
 */
function fitWideBar(bar: HTMLElement): void {
  const oneRow = (): boolean => {
    const mids = [...bar.children].filter((e) => e.getClientRects().length > 0).map((e) => {
      const r = e.getBoundingClientRect();
      return r.top + r.height / 2;
    });
    return mids.length === 0 || Math.max(...mids) - Math.min(...mids) < 4;
  };
  const fit = (): void => {
    if (!bar.isConnected) return;
    bar.classList.remove('pf-c1', 'pf-c2');
    if (oneRow()) return;
    bar.classList.add('pf-c1');
    if (oneRow()) return;
    bar.classList.add('pf-c2');
  };
  window.requestAnimationFrame(fit);
  if (typeof ResizeObserver === 'undefined') return;
  let lastW = -1;
  const ro = new ResizeObserver(() => {
    if (!bar.isConnected) {
      ro.disconnect();
      return;
    }
    const w = Math.round(bar.getBoundingClientRect().width);
    if (w === lastW) return;
    lastW = w;
    fit();
  });
  ro.observe(bar);
  void document.fonts?.ready.then(fit);
}

/**
 * The label of the Next place (Finish practice in a practice, Submit in an exam, on the last question): the shown label
 * plus both labels again, hidden, aria-hidden and in the primary weight, all in one grid cell. The place is as wide as the
 * wider label in either weight, so it never changes width on the last question and nothing after it moves (QZ-3).
 */
function nextOrFinish(finish: boolean, exam: boolean): HTMLElement {
  const next = h('span', {}, shortLabel('quiz.next', 'quiz.nextShort'), miniIcon('fwd', 'mi'));
  const fin = exam ? h('span', {}, t('quiz.submit')) : h('span', {}, shortLabel('quiz.practice.finish', 'quiz.practice.finishShort'));
  const ghosts = [next, fin].map((el) => {
    const g = el.cloneNode(true) as HTMLElement;
    g.className = 'off';
    g.setAttribute('aria-hidden', 'true');
    return g;
  });
  const cur = finish ? fin : next;
  cur.className = 'on';
  return h('span', { class: 'nx-two' }, cur, ...ghosts);
}

/** One button of the action bar: icon + span.lbl; CSS shows the label from the width the spec names; the label is always the name */
function barButton(o: {
  slot: 'prev' | 'flag'; icon: MiniIcon; label: string; tip?: string; focus: string; cls?: string; disabled?: boolean; pressed?: boolean; onClick: () => void;
}): HTMLButtonElement {
  const btn = h('button', {
    class: 'btn bbtn' + (o.cls ? ' ' + o.cls : ''), type: 'button', 'aria-label': o.label, 'data-slot': o.slot, 'data-focus': o.focus,
    disabled: o.disabled, 'aria-pressed': o.pressed === undefined ? null : String(o.pressed), onclick: o.onClick,
  }, miniIcon(o.icon, 'mi'), h('span', { class: 'lbl', 'aria-hidden': 'true' }, o.label));
  holdTip(btn, o.tip ?? o.label);
  return btn;
}

// ---- the answer sheet's open state on phones (QZ-5): per user and browser, every access guarded, works without storage ----
function readSheetOpen(): boolean {
  try {
    const id = S.user?.id;
    return id != null && window.localStorage.getItem(sheetOpenKey(id)) === '1';
  } catch {
    return false;
  }
}

function writeSheetOpen(open: boolean): void {
  try {
    const id = S.user?.id;
    if (id != null) window.localStorage.setItem(sheetOpenKey(id), open ? '1' : '0');
  } catch {
    // storage blocked: the sheet still opens and closes, it is just not remembered
  }
}

// ---- --top-h (2.12 e): the height of the pinned top row under PHONE, else 0; set on every render and when PHONE flips ----
let topRow: HTMLElement | null = null;

function syncTopH(): void {
  const row = topRow;
  // A-OWNER-81: when the row's right group does not fit on one line, the top-row finish pill switches to its short label (never drops it)
  const right = row && row.isConnected ? row.querySelector<HTMLElement>('.r') : null;
  const left = row && row.isConnected ? row.querySelector<HTMLElement>('.ql') : null;
  if (row && right && left) {
    row.classList.remove('tight');
    const over = left.scrollWidth > left.clientWidth + 1 || right.offsetTop > left.offsetTop + 4 || row.scrollWidth > row.clientWidth + 1;
    row.classList.toggle('tight', over);
  }
  const px = row && row.isConnected && matches(PHONE_MQ) ? row.offsetHeight : 0;
  document.documentElement.style.setProperty('--top-h', `${px}px`);
}

/** Keyboard (and the PHONE watcher of --top-h): ← → switch questions, A–H pick an answer, M flags. Holding a key does not repeat; ignored while a dialog is open or an input is focused. */
export function installQuizKeys(): void {
  // A phone turned sideways (or a window resized across the PHONE line) changes the pinned row's height: --top-h follows
  if (typeof window.matchMedia === 'function') {
    window.matchMedia(PHONE_MQ).addEventListener('change', () => {
      if (S.route.name === 'quiz') syncTopH();
    });
    window.addEventListener('resize', () => {
      if (S.route.name === 'quiz') syncTopH();
    });
  }
  document.addEventListener('keydown', (e) => {
    const se = S.session;
    if (S.route.name !== 'quiz' || !se || S.authView || e.ctrlKey || e.metaKey || e.altKey || e.repeat) return;
    if (document.querySelector('dialog[open]')) return;
    const target = e.target;
    if (target instanceof HTMLElement && /^(INPUT|TEXTAREA|SELECT)$/.test(target.tagName)) return;
    // keys typed inside the AI+ panel (its buttons, the answer text) are not quiz shortcuts
    if (target instanceof Element && target.closest('.aip-panel')) return;
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
      // pick() ignores a checked (locked) practice question; in a practice a pick only chooses, 核对答案 checks
      const k = e.key.toUpperCase();
      if (q.options.some((o) => o.key === k)) pick(q, k);
    }
  });
}
