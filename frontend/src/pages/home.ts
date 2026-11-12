// Home page: continue an unfinished exam or practice, choose banks / sheets / range / count and start an exam or (v0.6.5)
// a practice, plus the latest result (exam or practice) and an overview of the question banks in a side column.
import '../styles/home.css';
import { bankName, modeName, pageHead, recordBankNames } from '../components/common.ts';
import { aiPlusNow, naTag, summaryButton } from '../components/ai-plus-controls.ts';
import { kindTag } from '../components/kind-tag.ts';
import { icon } from '../components/shell.ts';
import { controlState, recordSummaryExtra } from '../domain/ai-plus.ts';
import { kindOf } from '../domain/attempt.ts';
import { inNotebook, masterStreakOf, pickQuestions, poolOf } from '../domain/grading.ts';
import { serverText } from '../i18n/errors.ts';
import { t, tn } from '../i18n/index.ts';
import { routeHash, type Route } from '../router.ts';
import { navigate, render, resumeSession, startExam } from '../state/actions.ts';
import { clearSaved, loadSaved } from '../state/session-storage.ts';
import { S, bankIdsOf, canManageBanks, isAdmin, pool, questionSheetKey, sheetKey, usableSheetKeys } from '../state/store.ts';
import type { AttemptKind, BankSummary, Mode } from '../types/api.ts';
import { ask } from '../utils/dialog.ts';
import { h, type Child } from '../utils/dom.ts';
import { fmtTime } from '../utils/format.ts';
import { liftToastAbove } from '../utils/toast.ts';

/** Wrong-answer notebook over the active questions: total size and size per sheet key */
interface Notebook {
  total: number;
  byKey: Map<string, number>;
  /** Correct answers in a row needed to leave the notebook */
  streak: number;
}

function notebook(): Notebook {
  const streak = masterStreakOf(S.progress);
  const byKey = new Map<string, number>();
  let total = 0;
  for (const q of S.bank?.questions ?? []) {
    if (!inNotebook(S.progress?.questions?.[q.id], streak)) continue;
    total++;
    const k = questionSheetKey(q);
    byKey.set(k, (byKey.get(k) ?? 0) + 1);
  }
  return { total, byKey, streak };
}

/** An in-app link: a real href (open in a new tab works), plain clicks go through navigate() */
function link(route: Route, label: Child, cls = 'btn'): HTMLAnchorElement {
  return h('a', {
    class: cls,
    href: routeHash(route),
    onclick: (e: MouseEvent) => {
      if (e.ctrlKey || e.metaKey || e.shiftKey || e.altKey || e.button !== 0) return;
      e.preventDefault();
      navigate(route);
    },
  }, label);
}

type Fact = [label: string, value: Child, cls?: string];

/** A small label / value list */
function facts(rows: Array<Fact | null>): HTMLDListElement {
  return h('dl', { class: 'home-facts props' }, rows.map((r) => (r ? [h('dt', null, r[0]), h('dd', { class: r[2] }, r[1])] : null)));
}

/** HM-4: the sub line of a bank / sheet row; the review count joins it (last part, in the wrong colour) instead of a chip */
function subLine(base: string, review: number): HTMLElement {
  return h('span', { class: 'sub' }, base, review ? [' · ', h('span', { class: 'nbwrong', title: t('home.pick.notebookTitle') }, t('home.pick.notebook', { n: review }))] : null);
}

// ---------------------------------------------------------------- unfinished exam

/** The small clock at the left of the unfinished-exam banner */
const ICON_RESUME = [{ cx: 12, cy: 12, r: 8.5 }, { d: 'M12 7.5V12l3 2' }];

async function discardSaved(practice: boolean): Promise<void> {
  const yes = await ask({
    title: t(practice ? 'home.resume.discardTitlePractice' : 'home.resume.discardTitle'),
    body: t('home.resume.discardBody'),
    ok: t('home.resume.discardOk'),
    cancel: t('home.resume.discardCancel'),
    danger: true,
  });
  if (!yes) return;
  clearSaved();
  render();
}

function resumeCard(): HTMLElement | null {
  const sv = loadSaved(S.bank);
  if (!sv) return null;
  const practice = sv.kind === 'practice';
  // HM-6: one line, "Unfinished practice · Question 12/51", then Continue and Discard (what was saved stays in the title)
  const names = bankIdsOf(sv.qs).map(bankName).join(t('common.listSep'));
  const detail = [names, sv.mode !== 'all' ? modeName(sv.mode) : '', t('home.resume.lead')].filter(Boolean).join(' · ');
  return h('section', { class: 'resume home-resume', 'aria-labelledby': 'home-resume-h' },
    icon(ICON_RESUME, 'ico rs-ico'),
    h('div', { class: 'rs-text', title: detail },
      h('div', { class: 'rs-head' },
        h('h2', { id: 'home-resume-h' }, t(practice ? 'home.resume.titlePractice' : 'home.resume.title')),
        h('span', { class: 'rs-where num' }, t('home.resume.pos', { i: sv.idx + 1, n: sv.qs.length }))),
      sv.dropped ? h('p', { class: 'rs-meta' }, tn('home.resume.dropped', sv.dropped)) : null),
    h('div', { class: 'rs-acts' },
      h('button', { class: 'btn primary', type: 'button', onclick: () => resumeSession() }, t('home.resume.continue')),
      h('button', { class: 'btn quiet', type: 'button', onclick: () => void discardSaved(practice) }, t('home.resume.discard'))));
}

// ---------------------------------------------------------------- start an exam

/** "All questions": a stack of sheets. "Wrong-answer notebook": a notebook with a cross. */
const ICON_ALL = [{ d: 'M12 4 3.5 8.5 12 13l8.5-4.5L12 4Z' }, { d: 'M3.5 12.5 12 17l8.5-4.5' }, { d: 'M3.5 16.5 12 21l8.5-4.5' }];
const ICON_CHEVRON = [{ d: 'M6.5 9.5 12 15l5.5-5.5' }];
/** A chevron pointing right: the third action of the latest-result card ("Records ›") */
const ICON_FWD = [{ d: 'M9.5 6.5 15 12l-5.5 5.5' }];
/** Banks the user unfolded / folded during this visit (otherwise: unfolded when there are at most two banks) */
const openBanks = new Map<number, boolean>();
const ICON_WEAK = [{ d: 'M6.5 3.5h9l3 3v14h-12Z' }, { d: 'M9.8 10.3l4.4 4.4' }, { d: 'M14.2 10.3l-4.4 4.4' }];

interface BankPick {
  /** Keys of the bank's usable sheets */
  keys: string[];
  box: HTMLInputElement;
  row: HTMLElement;
  /** "1 of 2 sheets selected" while only part of the bank is selected (visible also when the bank is folded) */
  info: HTMLElement;
  sheets: Array<{ key: string; box: HTMLInputElement }>;
}

/**
 * No questions to practise. An administrator imports a workbook. A bank manager (v0.6.5) can import one too (it becomes
 * theirs) or be given access to one. A plain user who sees no bank at all has not been given any yet (since v0.6.5 an
 * administrator chooses each user's banks; GET /api/banks lists only those), so they are asked to tell an administrator.
 */
function emptyState(): HTMLElement {
  const some = S.banks.length > 0;
  if (isAdmin()) {
    return h('div', { class: 'emptybank' },
      h('p', null, t(some ? 'home.empty.noQuestions' : 'home.empty.noBanks')),
      link({ name: 'banks' }, t('home.empty.go'), 'btn primary'));
  }
  // A bank manager can import banks and update their own, so they get the way to Question banks too (v0.7.5)
  if (canManageBanks()) {
    return h('div', { class: 'emptybank' },
      h('p', null, t(some ? 'home.empty.noQuestionsManager' : 'home.empty.noBanksManager')),
      link({ name: 'banks' }, t('home.empty.go'), 'btn primary'));
  }
  return h('div', { class: 'emptybank' }, h('p', null, t(some ? 'home.empty.noQuestionsUser' : 'home.empty.noAccess')));
}

function startPanel(nb: Notebook): HTMLElement {
  const b = S.bank;
  const cfg = S.cfg;
  // The header row holds the title and, on its right, the "questions from" choice
  const head = h('div', { class: 'home-setup-head' }, h('h2', { id: 'home-setup-h' }, t('home.setup.title')));
  const panel = h('section', { class: 'panel home-setup', 'aria-labelledby': 'home-setup-h' },
    head,
    b?.error ? h('div', { class: 'err', role: 'alert' }, h('p', null, serverText(b.error))) : null);

  const listed = S.banks.filter((x) => x.total > 0);
  if (!b || !b.questions.length || !listed.length) {
    panel.append(emptyState());
    return panel;
  }

  const selected = cfg.selected ?? new Set<string>();
  cfg.selected = selected;
  const notebookInSelection = (): number => {
    let n = 0;
    for (const k of selected) n += nb.byKey.get(k) ?? 0;
    return n;
  };
  // Only "all questions" and the notebook can be chosen here ("retry" is started from a result or a record)
  if (cfg.mode !== 'weak' || !notebookInSelection()) cfg.mode = 'all';

  // Banks and sheets
  const picks: BankPick[] = [];
  const toggle = (keys: readonly string[], on: boolean): void => {
    for (const k of keys) {
      if (on) selected.add(k);
      else selected.delete(k);
    }
    refresh();
  };

  const bankGroup = (bank: BankSummary): HTMLElement => {
    const usable = bank.sheets.filter((s) => s.usable > 0);
    const keys = usable.map((s) => sheetKey(bank.id, s.name));
    const box = h('input', {
      type: 'checkbox', 'aria-controls': `home-sheets-${bank.id}`,
      onchange: () => toggle(keys, box.checked),
    });
    // Folded banks keep the list short without a scrolling box inside the page (nested scrolling fights the page on phones)
    const open = openBanks.get(bank.id) ?? listed.length <= 2;
    const info = h('span', { class: 'sub home-selinfo' });
    const label = h('label', { class: 'home-bankrow' }, box,
      h('span', { class: 'grow' },
        h('span', { class: 'name' }, bank.name),
        subLine(t('home.pick.bankSub', { questions: tn('common.questions', bank.total), sheets: tn('common.sheets', usable.length) }),
          keys.reduce((n, k) => n + (nb.byKey.get(k) ?? 0), 0)),
        info));
    const sheetBox = h('div', { class: 'home-sheets', id: `home-sheets-${bank.id}`, role: 'group', 'aria-label': bank.name, hidden: !open });
    const expand = h('button', {
      class: 'home-expand', type: 'button', 'aria-expanded': String(open), 'aria-controls': `home-sheets-${bank.id}`,
      'aria-label': t(open ? 'home.pick.hideSheets' : 'home.pick.showSheets', { name: bank.name }),
      title: t(open ? 'home.pick.hideSheets' : 'home.pick.showSheets', { name: bank.name }),
      onclick: () => {
        const now = sheetBox.hidden;
        sheetBox.hidden = !now;
        openBanks.set(bank.id, now);
        expand.setAttribute('aria-expanded', String(now));
        const tip = t(now ? 'home.pick.hideSheets' : 'home.pick.showSheets', { name: bank.name });
        expand.setAttribute('aria-label', tip);
        expand.title = tip;
      },
    }, icon(ICON_CHEVRON));
    const row = h('div', { class: 'home-bankhead' }, label, expand);
    const pick: BankPick = { keys, box, row, info, sheets: [] };
    picks.push(pick);

    const sheetRows = bank.sheets.map((s) => {
      if (s.usable <= 0) {
        return h('label', { class: 'home-sheetrow off' },
          h('input', { type: 'checkbox', disabled: true }),
          h('span', { class: 'grow' }, h('span', { class: 'name' }, s.name), h('span', { class: 'sub' }, t('home.pick.sheetRetired'))));
      }
      const key = sheetKey(bank.id, s.name);
      const sbox = h('input', { type: 'checkbox', onchange: () => toggle([key], sbox.checked) });
      pick.sheets.push({ key, box: sbox });
      return h('label', { class: 'home-sheetrow' }, sbox,
        h('span', { class: 'grow' },
          h('span', { class: 'name' }, s.name),
          subLine(s.inactive ? tn('home.pick.sheetSubRetired', s.usable, { retired: s.inactive }) : tn('home.pick.sheetSub', s.usable), nb.byKey.get(key) ?? 0)));
    });

    // One card per bank: the bank row is the card's header, its sheets are compact rows inside the same card
    sheetBox.append(...sheetRows);
    return h('div', { class: 'home-bank' }, row, sheetBox);
  };

  const groups = listed.map(bankGroup);
  const allKeys = picks.flatMap((p) => p.keys);
  const hidden = S.banks.length - listed.length;
  // The visible heading and "select all / clear" share one row; the legend names the group for screen readers
  const pickSet = h('fieldset', { class: 'home-pick' },
    h('legend', { class: 'sr-only' }, t('home.pick.legend')),
    h('div', { class: 'home-pick-head' },
      h('span', { class: 'home-pick-title', 'aria-hidden': 'true' }, t('home.pick.legend')),
      allKeys.length > 1 ? h('div', { class: 'home-pickacts' },
        h('button', { class: 'btn quiet', type: 'button', onclick: () => toggle(allKeys, true) }, t('home.pick.all')),
        h('button', { class: 'btn quiet', type: 'button', onclick: () => toggle(allKeys, false) }, t('home.pick.none'))) : null),
    h('div', { class: 'home-picklist' }, groups),
    hidden ? h('p', { class: 'dim home-note' }, tn('home.pick.hiddenBanks', hidden)) : null);

  // Range (mode)
  const modeBox = (val: Mode): HTMLInputElement => h('input', {
    type: 'radio', name: 'home-mode', value: val,
    onchange: () => {
      cfg.mode = val;
      refresh();
    },
  });
  const allBox = modeBox('all');
  const weakBox = modeBox('weak');
  // Two compact choices with an icon and a count; the explanation is one short line under them, only when it matters
  const allCount = h('span', { class: 'mseg-n num' });
  const weakCount = h('span', { class: 'mseg-n num' });
  const modeHint = h('p', { class: 'mseg-hint', id: 'home-mode-hint' });
  const weakRow = h('label', { class: 'mseg weak', title: t('mode.weak') }, weakBox, icon(ICON_WEAK, 'ico mseg-ico'), h('span', { class: 'mseg-label' }, h('span', { class: 'hm-full' }, t('home.mode.weakShort')), h('span', { class: 'hm-short' }, t('home.mode.weakPhone'))), weakCount);
  const modeSet = h('fieldset', { class: 'home-mode', 'aria-describedby': 'home-mode-hint' },
    h('legend', { class: 'sr-only' }, t('home.mode.legend')),
    h('div', { class: 'mseg-group' },
      h('label', { class: 'mseg', title: t('home.mode.allSub') }, allBox, icon(ICON_ALL, 'ico mseg-ico'), h('span', { class: 'mseg-label' }, t('home.mode.allShort')), allCount),
      weakRow));
  head.append(modeSet);
  head.after(modeHint);

  // Count, order, start
  const countInput = h('input', { type: 'number', min: '1', step: '1', inputmode: 'numeric', 'aria-label': t('home.count.label') });
  const shuffleBox = h('input', {
    type: 'checkbox', checked: cfg.shuffle,
    onchange: () => {
      cfg.shuffle = shuffleBox.checked;
    },
  });
  // Start exam stays the main action; Start practice (v0.6.5) draws from the same scope, next to it. v0.7 (owner): Start exam
  // is solid green and Start practice solid orange (home.css)
  const startBtn = h('button', { class: 'btn primary big home-go-exam', type: 'button', onclick: () => start('exam') }, t('home.start'));
  const practiceBtn = h('button', {
    class: 'btn big home-go-practice', type: 'button', title: t('home.startPractice.hint'), onclick: () => start('practice'),
  }, t('home.startPractice'));
  const scopeText = h('p', { class: 'home-scope', role: 'status' });

  /** The number typed into the count field (null = empty / not a positive number) */
  const typedCount = (): number | null => {
    const v = parseInt(countInput.value, 10);
    return Number.isFinite(v) && v >= 1 ? v : null;
  };
  const effectiveCount = (n: number): number => (cfg.count == null ? n : Math.min(Math.max(1, cfg.count), n));
  const readCount = (n: number): void => {
    const v = typedCount();
    cfg.count = v == null || v >= n ? null : v;
  };

  const updateScope = (n: number): void => {
    const k = effectiveCount(n);
    scopeText.textContent = n === 0
      ? t(selected.size ? 'home.scopeEmpty' : 'home.scopeNoSheet')
      : k < n ? t('home.scopePart', { n, count: k }) : tn('home.scope', n);
  };

  // Updates everything that depends on the selection in place (no re-render: keeps focus and scroll position)
  function refresh(): void {
    for (const p of picks) {
      const on = p.keys.filter((k) => selected.has(k)).length;
      p.box.checked = on > 0 && on === p.keys.length;
      p.box.indeterminate = on > 0 && on < p.keys.length;
      p.row.classList.toggle('part', p.box.indeterminate);
      p.info.textContent = p.box.indeterminate ? t('home.pick.partial', { on, n: p.keys.length }) : '';
      for (const s of p.sheets) s.box.checked = selected.has(s.key);
    }
    const inSel = notebookInSelection();
    weakBox.disabled = inSel === 0;
    weakRow.classList.toggle('off', inSel === 0);
    if (cfg.mode === 'weak' && !inSel) cfg.mode = 'all';
    allBox.checked = cfg.mode === 'all';
    weakBox.checked = cfg.mode === 'weak';
    allCount.textContent = String(S.bank ? poolOf(S.bank.questions, selected, 'all', S.progress, questionSheetKey).length : 0);
    weakCount.textContent = String(inSel);
    // Say why the notebook cannot be chosen, or how a question leaves it while it is chosen; nothing otherwise
    modeHint.textContent = !nb.total ? t('home.mode.weakEmpty')
      : !inSel ? tn('home.mode.weakNone', nb.total)
        : cfg.mode === 'weak' ? tn('home.mode.weakSub', inSel, { k: nb.streak }) : '';
    modeHint.hidden = !modeHint.textContent;

    const n = pool().length;
    countInput.max = String(Math.max(1, n));
    countInput.disabled = n === 0;
    countInput.value = n ? String(effectiveCount(n)) : '';
    startBtn.disabled = n === 0;
    practiceBtn.disabled = n === 0;
    updateScope(n);
  }

  // While typing only the scope text follows; the field itself is normalised when the edit is committed
  countInput.addEventListener('input', () => {
    const n = pool().length;
    readCount(n);
    updateScope(n);
  });
  countInput.addEventListener('change', () => {
    readCount(pool().length);
    refresh();
  });

  function start(kind: AttemptKind): void {
    const p = pool();
    if (!p.length) return;
    readCount(p.length);
    cfg.shuffle = shuffleBox.checked;
    const n = effectiveCount(p.length);
    void startExam(pickQuestions(p, n, cfg.shuffle), cfg.mode, kind);
  }

  // The bar pinned to the bottom: Start exam / Start practice, what will be asked, and the two options right next to it
  const goBar = h('div', { class: 'home-go' }, h('div', { class: 'home-go-btns' }, startBtn, practiceBtn), scopeText,
    h('div', { class: 'home-go-opts' },
      h('label', { class: 'home-count', title: t('home.count.label') }, h('span', null, t('home.count.short')), countInput),
      h('label', { class: 'home-shuffle', title: t('home.shuffle') }, shuffleBox, h('span', null, t('home.shuffle.short')))));
  // The mode comes first: "practise my wrong answers" is a frequent start and should not sit below every sheet row
  panel.append(pickSet, goBar);
  // v1.0 (SH-4): toasts sit above the start bar while it is on screen (this moves the toasts, never the bar: HM-1)
  liftToastAbove(goBar);
  refresh();
  return panel;
}

// ---------------------------------------------------------------- side column

/**
 * A greyed AI summary: in the narrow cell the 无法使用 tag replaces the button's label (HM-2). The button keeps its label
 * (wrapped, hidden by CSS at 480 px and below) and gets its own copy of the tag; the tag after the button shows on wide
 * layouts only. The reason stays the accessible description.
 */
function tagInsideButton(root: HTMLElement, state: ReturnType<typeof controlState>): void {
  const btn = root instanceof HTMLButtonElement ? root : root.querySelector<HTMLButtonElement>('button.aip-sum-btn');
  const tag = naTag(state);
  if (!btn || !tag) return;
  for (const n of Array.from(btn.childNodes)) {
    if (n.nodeType === Node.TEXT_NODE && n.textContent?.trim()) {
      const lbl = h('span', { class: 'hm-ai-lbl' }, n.textContent);
      btn.replaceChild(lbl, n);
    }
  }
  btn.append(h('span', { class: 'aip-na hm-ai-na', 'aria-hidden': 'true' }, tag.textContent));
}

function latestCard(): HTMLElement {
  const sec = h('section', { class: 'panel home-latest', 'aria-labelledby': 'home-latest-h' },
    h('h2', { id: 'home-latest-h' }, t('home.latest.title')));
  if (S.records == null) {
    sec.append(h('p', { class: 'dim' }, t('common.loading')));
    return sec;
  }
  const r = S.records[0];
  if (!r) {
    sec.append(h('p', { class: 'empty' }, t('home.latest.empty')));
    return sec;
  }
  const wrong = Math.max(0, r.total - r.correct);
  // v0.9: the AI summary of the latest record (only for users with the AI+ feature; greyed with its own tag when unusable)
  const view = aiPlusNow();
  const state = controlState(view, recordSummaryExtra(r));
  const summary = view.show
    ? summaryButton({ id: r.id, at: r.at, kind: kindOf(r) }, state, { anchorKey: 'home-ai', quiet: false, tag: true })
    : null;
  if (summary && state.show && !state.usable) tagInsideButton(summary, state);
  // The latest record of either kind, with its Exam / Practice tag
  sec.append(
    h('div', { class: 'stat' },
      h('b', { class: 'num' }, t('home.latest.percent', { percent: r.percent })),
      h('span', { class: 'dim' }, t('home.latest.correct', { correct: r.correct, total: r.total })),
      kindTag(kindOf(r))),
    facts([
      [t('home.fact.at'), h('span', { class: 'num' }, r.at)],
      [t('home.fact.banks'), recordBankNames(r.banks)],
      modeName(r.mode) ? [t('home.fact.mode'), modeName(r.mode)] : null,
      [t('home.fact.wrong'), tn('common.questions', wrong), wrong ? 'ng' : undefined],
      [t('home.fact.time'), h('span', { class: 'num' }, fmtTime(r.seconds))],
    ]),
    // One row of equal cells (two without AI+): details, AI summary, all records (text + chevron, a short name on phones)
    h('div', { class: 'acts home-latest-acts' },
      link({ name: 'record', id: r.id }, t('home.latest.view')),
      summary ? h('div', { class: 'home-cell' }, summary) : null,
      link({ name: 'records' }, [
        h('span', { class: 'hm-full' }, t('home.latest.all')),
        h('span', { class: 'hm-short' }, t('home.latest.allShort')),
        icon(ICON_FWD, 'ico hm-fwd'),
      ], 'btn quiet')));
  return sec;
}

/** HM-5: the wrong-notebook figure is a link to the wrong-answer review when there is something to review */
function notebookFig(total: number): HTMLElement {
  const a = link({ name: 'wrong' }, [h('b', { class: 'num' }, total), h('span', null, t('home.overview.notebook'))], 'home-fig ng');
  a.setAttribute('aria-label', t('home.overview.reviewN', { n: total }));
  return a;
}

function overviewCard(nb: Notebook): HTMLElement {
  const questions = S.banks.reduce((n, b) => n + b.total, 0);
  const retired = S.banks.reduce((n, b) => n + b.inactive, 0);
  const fig = (value: number, label: string, cls = ''): HTMLElement =>
    h('div', { class: 'home-fig' + cls }, h('b', { class: 'num' }, value), h('span', null, label));
  return h('section', { class: 'panel home-overview', 'aria-labelledby': 'home-overview-h' },
    h('h2', { id: 'home-overview-h' }, t('home.overview.title')),
    h('div', { class: 'home-figs' },
      fig(S.banks.length, t('home.overview.banks')),
      fig(usableSheetKeys().length, t('home.overview.sheets')),
      fig(questions, t('home.overview.questions')),
      nb.total ? notebookFig(nb.total) : fig(nb.total, t('home.overview.notebook'))),
    retired ? h('p', { class: 'dim home-note' }, tn('home.overview.retired', retired)) : null,
    // With wrong answers to review, that is the stronger action; the banks page (administrators and bank managers) is also
    // in the menu
    nb.total || canManageBanks() ? h('div', { class: 'acts' },
      nb.total ? link({ name: 'wrong' }, t('home.overview.review')) : null,
      canManageBanks() ? link({ name: 'banks' }, t('nav.banks'), nb.total ? 'btn quiet' : 'btn') : null) : null);
}

export function renderHome(): HTMLElement {
  // the start panel sets it again when it has its bar (no questions: no bar, toasts at the default height)
  liftToastAbove(null);
  const nb = notebook();
  return h('div', { class: 'page page-home' },
    pageHead(t('home.title')),
    h('div', { class: 'home-grid' },
      h('div', { class: 'home-main' }, resumeCard(), startPanel(nb)),
      h('div', { class: 'home-side' }, latestCard(), overviewCard(nb))));
}
