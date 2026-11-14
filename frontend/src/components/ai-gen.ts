// AI-generated banks, the UI (v1.1, GB-1 .. GB-8): the Home button, the Bank management row action, the name / confirm
// dialogs and the progress cards (also of file imports, IM-6, drawn by components/ai-import.ts). The decisions are the pure
// functions of domain/ai-gen.ts; the state (status, upload, polling) is state/ai-gen.ts. Nothing here is drawn on #/quiz.
import '../styles/ai-gen.css';
import { aiControlView, type ControlContext, type ControlView } from '../domain/ai-features.ts';
import {
  checkName, cardJobs, genCount, genView, NAME_MAX_HOME, runningLine, stampText, type GenMode, type GenViewState,
} from '../domain/ai-gen.ts';
import { masterStreakOf, inNotebook } from '../domain/grading.ts';
import { jobReasonText } from '../i18n/errors.ts';
import { localeTag, t, tn, uiLang, type MsgKey } from '../i18n/index.ts';
import { navigate } from '../state/actions.ts';
import { aiGenStatus, aiStatus } from '../state/ai.ts';
import { cancelJob, currentUpload, dismissJob, startGeneration, subscribeGen } from '../state/ai-gen.ts';
import { canManageBanks, S, sheetKey } from '../state/store.ts';
import type { GenJob, GenStatus } from '../types/api.ts';
import { ask, formDialog } from '../utils/dialog.ts';
import { h } from '../utils/dom.ts';
import { toast } from '../utils/toast.ts';
import { fileCard } from './ai-import.ts';
import { tokenFigures } from '../domain/usage-bars.ts';
import { iconButton, shortLabel, withReason } from './icon-button.ts';
import { tipEl } from './tooltip.ts';
import { miniIcon } from './mini-icons.ts';

const msg = (k: string): MsgKey => k as MsgKey;

// ---------------------------------------------------------------- the decision of every AI bank control

/** The context every control decision (M-7) is made in */
export function controlContext(): ControlContext {
  const u = S.user;
  return { role: u?.role ?? 'user', service: u != null && u.aiAllowed, canManageBanks: canManageBanks(), status: aiStatus() };
}

/** The M-7 view of one feature for the signed-in user; the bank controls need the bank-management permission */
export const viewOf = (feature: 'generate' | 'import'): ControlView => aiControlView(feature, true, controlContext());

/** The reason text of a greyed state: the catalog text with its parameters, then the "AI banks are skipped" sentence */
export function reasonText(v: Extract<GenViewState, { usable: false }>): string {
  const base = t(msg(v.key), v.params);
  return v.skippedNote ? base + (uiLang() === 'en' ? ' ' : '') + t('aigen.why.aiSkipped') : base;
}

// ---------------------------------------------------------------- counts

/** Questions of the checked sheets of NON-AI banks (Home) */
function scopeQuestions(scope: ReadonlyArray<{ bank: number; sheet: string }>): { have: number; haveWrong: number; checked: number; onlyAi: boolean; skippedAi: boolean; hasAi: boolean } {
  const aiBanks = new Set(S.banks.filter((b) => b.ai).map((b) => b.id));
  const keys = new Set<string>();
  let aiSheets = 0;
  for (const s of scope) {
    if (aiBanks.has(s.bank)) aiSheets += 1;
    else keys.add(sheetKey(s.bank, s.sheet));
  }
  const ms = masterStreakOf(S.progress);
  let have = 0;
  let haveWrong = 0;
  for (const q of S.bank?.questions ?? []) {
    if (q.kind === 'hotspot' || !keys.has(sheetKey(q.bank, q.sheet))) continue;
    have += 1;
    if (inNotebook(S.progress?.questions?.[q.id], ms)) haveWrong += 1;
  }
  return { have, haveWrong, checked: keys.size, onlyAi: keys.size === 0 && aiSheets > 0, skippedAi: aiSheets > 0 && keys.size > 0, hasAi: aiSheets > 0 };
}

/** The AI sheets of the checked ones are never sent (the server skips them too) */
export const nonAiScope = (scope: ReadonlyArray<{ bank: number; sheet: string }>): Array<{ bank: number; sheet: string }> => {
  const aiBanks = new Set(S.banks.filter((b) => b.ai).map((b) => b.id));
  return scope.filter((s) => !aiBanks.has(s.bank)).map((s) => ({ bank: s.bank, sheet: s.sheet }));
};

// ---------------------------------------------------------------- the quota line and the dialogs

export function quotaLine(gen: GenStatus, key: 'aigen.dlg.quota' | 'aigen.imp.dlg.quota' = 'aigen.dlg.quota'): string {
  if (gen.unlimited) return t('aigen.dlg.unlimited');
  return t(key, { limit: gen.perDay ?? 0, left: gen.left ?? 0 });
}

/** Home: the name dialog, then start (GB-3) */
export async function startScopeGeneration(mode: 'scope' | 'wrong', scope: ReadonlyArray<{ bank: number; sheet: string }>): Promise<void> {
  const gen = aiGenStatus();
  if (!gen) return;
  const sc = scopeQuestions(scope);
  const have = mode === 'wrong' ? sc.haveWrong : sc.have;
  const input = h('input', {
    type: 'text', id: 'gen-name', class: 'gen-name', maxlength: String(NAME_MAX_HOME), autocomplete: 'off', spellcheck: false,
    value: t(mode === 'wrong' ? 'aigen.name.wrong' : 'aigen.name.scope', { time: stampText(new Date()) }), 'aria-describedby': 'gen-name-hint',
  });
  const hint = h('p', { class: 'dim gen-hint', id: 'gen-name-hint' }, t('aigen.dlg.nameHint', { max: NAME_MAX_HOME }));
  const content = h('div', { class: 'gen-form' },
    h('label', { for: 'gen-name', class: 'gen-label' }, t('aigen.dlg.name')), input, hint,
    h('p', { class: 'gen-quota' }, quotaLine(gen)));
  await formDialog({
    title: t(mode === 'wrong' ? 'aigen.dlg.wrongTitle' : 'aigen.dlg.scopeTitle'),
    body: t(mode === 'wrong' ? 'aigen.dlg.wrongBody' : 'aigen.dlg.scopeBody', { n: genCount(gen, mode), have }),
    content, ok: t('aigen.dlg.start'), busy: t('aigen.dlg.start'), cancel: t('aigen.dlg.cancel'), focus: input,
    submit: async () => {
      const c = checkName(input.value);
      if (!c.ok) return t(c.error === 'empty' ? 'aigen.dlg.nameEmpty' : c.error === 'long' ? 'aigen.dlg.nameLong' : 'aigen.dlg.nameBad', { max: NAME_MAX_HOME });
      const sheets = nonAiScope(scope);
      if (sheets.length === 0) return t('aigen.why.noScope');
      return startGeneration({ source: mode, sheets, name: c.name });
    },
  });
}

// ---------------------------------------------------------------- the buttons

/** One AI control in its three looks: usable, greyed with / without the 无法使用 tag (SH-17) */
function control(o: {
  state: GenViewState; icon: 'aiGen' | 'aiImport'; label: string; short?: [MsgKey, MsgKey]; aria?: string; tip?: string; cls: string;
  onClick: () => void; showLabel: boolean;
}): HTMLElement | null {
  const s = o.state;
  if (!s.show) return null;
  const btn = iconButton({ icon: o.icon, label: o.aria ?? o.label, tone: 'ai', showLabel: o.showLabel, ...(s.usable ? { onClick: o.onClick } : {}) });
  btn.classList.add(...o.cls.split(' ').filter(Boolean));
  if (o.showLabel && o.short) {
    btn.querySelector('.ibtn-lbl')?.replaceChildren(...shortLabel(o.short[0], o.short[1]));
    if (o.aria) btn.setAttribute('aria-label', o.aria);
  }
  if (s.usable) {
    if (o.tip) btn.title = o.tip;
    return btn;
  }
  if (s.tag) btn.append(h('span', { class: 'gen-na', 'aria-hidden': 'true' }, t('aiplus.na')));
  return withReason(btn, reasonText(s), { tapTip: true });
}

/**
 * Home's header button (C-F1): `mode` follows the 全部 / 错题本 switch, `scope` = the checked sheets. Null when the control is
 * hidden (no AI+ or no bank management, M-7). It redraws itself when the job state changes.
 */
export function homeGenButton(mode: 'all' | 'wrong', scope: Array<{ bank: number; sheet: string }>): HTMLElement | null {
  const host = h('span', { class: 'gen-host' });
  const draw = (): void => {
    const gen = aiGenStatus();
    const sc = scopeQuestions(scope);
    const m: GenMode = mode === 'wrong' ? 'wrong' : 'scope';
    const st = genView(viewOf('generate'), gen, {
      mode: m, checkedSheets: sc.checked, onlyAi: sc.onlyAi, skippedAi: sc.skippedAi, hasAi: sc.hasAi, have: mode === 'wrong' ? sc.haveWrong : sc.have,
    });
    const n = gen ? genCount(gen, mode === 'wrong' ? 'wrong' : 'bank') : 0;
    const el = control({
      state: st, icon: 'aiGen', label: t(mode === 'wrong' ? 'aigen.home.wrong' : 'aigen.home.scope'),
      short: mode === 'wrong' ? ['aigen.home.wrong', 'aigen.home.wrongShort'] : ['aigen.home.scope', 'aigen.home.scopeShort'],
      tip: t(mode === 'wrong' ? 'aigen.home.wrongTip' : 'aigen.home.scopeTip', { n }), cls: 'gen-btn gen-home',
      onClick: () => void startScopeGeneration(m === 'wrong' ? 'wrong' : 'scope', scope), showLabel: true,
    });
    host.replaceChildren(...(el ? [el] : []));
    host.hidden = el == null;
  };
  draw();
  subscribeGen(host, draw);
  return host.hidden ? null : host;
}

/**
 * Generation over the checked sheets of Bank management (BL-3): same decision as Home's (scope mode). `sheets` are the
 * checked sheets (AI banks cannot be checked there). Returns the button, or null while hidden.
 */
export function selectedGenButton(sheets: Array<{ bank: number; sheet: string }>): HTMLElement | null {
  const host = h('span', { class: 'gen-host' });
  const draw = (): void => {
    const sc = scopeQuestions(sheets);
    const st = genView(viewOf('generate'), aiGenStatus(), { mode: 'scope', checkedSheets: sc.checked, onlyAi: sc.onlyAi, skippedAi: sc.skippedAi, hasAi: sc.hasAi, have: sc.have });
    const el = control({
      state: st, icon: 'aiGen', label: t('aigen.sel.action'), cls: 'gen-btn gen-sel', showLabel: true,
      onClick: () => void startScopeGeneration('scope', sheets),
    });
    host.replaceChildren(...(el ? [el] : []));
    host.hidden = el == null;
  };
  draw();
  subscribeGen(host, draw);
  return host.hidden ? null : host;
}

// ---------------------------------------------------------------- the progress cards (GB-7)

function bar(pct: number | null, label: string): HTMLElement {
  return h('div', {
    class: 'gen-bar' + (pct == null ? ' ind' : ''), role: 'progressbar', 'aria-label': label,
    'aria-valuemin': '0', 'aria-valuemax': '100', 'aria-valuenow': pct == null ? null : String(pct),
  }, h('div', { class: 'gen-fill', style: pct == null ? '' : `width:${pct}%` }));
}

async function askCancel(job: GenJob, btn: HTMLButtonElement): Promise<void> {
  const free = job.calls === 0;
  const ok = await ask({
    title: t('aigen.card.cancelAsk'), body: t(free ? 'aigen.card.cancelBodyFree' : 'aigen.card.cancelBody'),
    ok: t('aigen.card.cancelOk'), cancel: t('aigen.card.cancelKeep'), danger: true,
  });
  if (!ok) return;
  await whileSending(btn, () => cancelJob(job));
}

/**
 * A-OWNER-67: the job's live figures (calls, input and output tokens, exact, with separators). A plain note, no live region
 * (the progress line above stays the only polite one); kept on the end states. Empty while the job has made no call.
 */
export function usageNote(job: GenJob): HTMLElement[] {
  const u = job.usage;
  if (!u || u.calls <= 0 || job.state === 'queued') return [];
  const f = (n: number): string => n.toLocaleString(localeTag());
  // A-OWNER-84: one grid, three max-content columns: "本轮调用 N 次，" | label | numbers; row 2 (cache) only when write or read > 0.
  // The exact numbers stay in the tip (tap / hover / focus).
  const { io, cache } = tokenFigures(u);
  const exactCache = cache ? t('ai.set.usage.sumCached', { w: f(u.tokensCacheWrite ?? 0), r: f(u.tokensCached ?? 0) }) : '';
  const exact = tn('aigen.card.usage', u.calls, { calls: f(u.calls), tin: f(u.tokensIn), tout: f(u.tokensOut) }) + exactCache;
  const id = `gen-usage-tip-${job.id}`;
  const cell = (cls: string, ...kids: Array<string | HTMLElement>): HTMLElement => h('span', { class: cls }, ...kids);
  const rows: HTMLElement[] = [
    cell('gu-n', tn('aigen.card.calls', u.calls, { calls: f(u.calls) }), cell('gu-c', t('aigen.card.callsSep'))),
    cell('gu-l', t('aigen.card.ioLabel')), cell('gu-v', `${io} Token`),
  ];
  if (cache) rows.push(cell('gu-n gu-e', ''), cell('gu-l', t('aigen.card.cacheLabel')), cell('gu-v', `${cache} Token`));
  const p: HTMLElement = h('div', { class: 'dim gen-note gen-usage', tabindex: '0', 'aria-describedby': id }, ...rows);
  // Never break a row: when the three columns do not fit, the call count takes its own line (no comma) and the label rows sit
  // under each other; if even that is too wide the type shrinks a little
  const fit = (): void => {
    if (!p.isConnected || p.clientWidth === 0) return;
    p.classList.remove('gu-narrow', 'gu-tiny');
    if (p.scrollWidth <= p.clientWidth) return;
    p.classList.add('gu-narrow');
    if (p.scrollWidth > p.clientWidth) p.classList.add('gu-tiny');
  };
  if (typeof ResizeObserver !== 'undefined') new ResizeObserver(fit).observe(p);
  return [h('span', { class: 'has-tip tip-tap gen-usage-wrap' }, p, tipEl(id, exact))];
}

/**
 * A-OWNER-68: the cancel button of a running AI bank job: a red outline, a cancel mark (x in a circle) before the label. The
 * handler gets the button so it can grey it while the cancel is sent.
 */
export function cancelButton(label: string, onClick: (btn: HTMLButtonElement) => void): HTMLButtonElement {
  const btn: HTMLButtonElement = h('button', { type: 'button', class: 'btn gen-cancel', onclick: () => onClick(btn) }, miniIcon('xCircle', 'gen-cancel-ic'), label);
  return btn;
}

/** Runs a cancel request with its button greyed meanwhile (back to normal if it failed) */
export async function whileSending(btn: HTMLButtonElement, run: () => Promise<string | null | undefined | void>): Promise<void> {
  btn.disabled = true;
  btn.setAttribute('aria-busy', 'true');
  const problem = await run();
  btn.disabled = false;
  btn.removeAttribute('aria-busy');
  if (problem) toast(problem, 4000);
}

/** The card of a generation job (running, done, failed) */
function genCard(job: GenJob): HTMLElement {
  const active = job.state === 'queued' || job.state === 'running';
  const card = h('section', { class: 'panel gen-card', 'data-gen-card': String(job.id), tabindex: '-1', 'aria-label': t('aigen.card.title', { name: job.name }) });
  if (active) {
    const line = runningLine(job);
    card.append(
      h('h3', { class: 'gen-title' }, t('aigen.card.title', { name: job.name })),
      bar(job.state === 'queued' ? 0 : line.pct, t('aigen.card.progressLabel')),
      h('p', { class: 'gen-line', 'aria-live': 'polite' }, t(line.key, { made: job.made, goal: job.goal })),
      h('p', { class: 'dim gen-note' }, t('aigen.card.leave')),
      ...usageNote(job),
      job.phase === 'saving'
        ? withReason(cancelButton(t('aigen.card.cancel'), () => undefined), t('aigen.err.tooLate'))
        : cancelButton(t('aigen.card.cancel'), (btn) => void askCancel(job, btn)));
    return card;
  }
  // A-OWNER-33: the close button first, floated into the card's top-right corner, so the text wraps beside it
  card.append(dismissButton(job));
  if (job.state === 'done') {
    card.append(
      h('p', { class: 'gen-line gen-done' }, miniIcon('check', 'gen-ic'), t('aigen.card.done', { name: job.name, made: job.made })),
      ...usageNote(job),
      h('div', { class: 'acts gen-acts' },
        h('button', { type: 'button', class: 'btn primary', onclick: () => practiceBank(job) }, t('aigen.card.practice'))));
  } else {
    card.append(
      h('p', { class: 'gen-line gen-failed', role: 'status' }, miniIcon('xCircle', 'gen-ic'), t('aigen.card.failed', { reason: jobReasonText(job) })),
      h('p', { class: 'dim gen-note' }, t('aigen.card.notCounted')),
      ...usageNote(job));
  }
  return card;
}

/** The close "x" of a finished job's card (A-OWNER-33): a rounded-square outline button in the card's top-right corner */
export function dismissButton(job: GenJob): HTMLButtonElement {
  return h('button', { type: 'button', class: 'gen-x', 'aria-label': t('aigen.card.dismiss'), title: t('aigen.card.dismiss'), onclick: () => void dismissJob(job) }, miniIcon('x'));
}

/** 去练习: Home, the new bank's sheets checked, mode all, the start bar in view, then the card is dismissed */
export function practiceBank(job: GenJob): void {
  const bank = job.bankId != null ? S.banks.find((b) => b.id === job.bankId) : undefined;
  if (bank) {
    S.cfg.selected = new Set(bank.sheets.filter((s) => s.usable > 0).map((s) => sheetKey(bank.id, s.name)));
    S.cfg.mode = 'all';
  }
  navigate({ name: 'home' });
  void dismissJob(job);
  setTimeout(() => document.getElementById('home-setup-h')?.scrollIntoView({ block: 'start' }), 80);
}

/**
 * The card host (C-F1): the client-only upload card first, then the active job and the unseen finished ones (at most 3).
 * It draws itself from state/ai-gen.ts. Never on the quiz page (no page puts it there).
 */
export function genCardHost(where: 'home' | 'banks'): HTMLElement {
  const host = h('div', { class: 'gen-cards', 'data-gen-host': where });
  const draw = (): void => {
    const gen = aiGenStatus();
    const kids: HTMLElement[] = [];
    const up = currentUpload();
    if (up && S.route.name !== 'quiz') kids.push(fileCard(null, up));
    if (S.route.name !== 'quiz') {
      for (const j of cardJobs(gen)) kids.push(j.source === 'file' ? fileCard(j, null) : genCard(j));
    }
    host.replaceChildren(...kids);
    host.hidden = kids.length === 0;
  };
  draw();
  subscribeGen(host, draw);
  return host;
}

