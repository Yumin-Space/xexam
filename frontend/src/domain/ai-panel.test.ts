import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { describe, it } from 'node:test';
import {
  bodyOverscroll, bodySize, capNote, cardAnchorPlan, chevronPlan, createAlertGate, focusAfterPress, followsText, footerSlots, headVisible,
  isNotAsked, isPhone, isShortLandscape, pairOf, pressPlan, questionChangePlan, regenerateAllowed, savedInChat, scrollDelta,
  type PanelState, type PanelTarget,
} from './ai-panel.ts';
import { emptyEntry, entryView, reduce, type Answer, type CallBlock, type Entry } from './ai-study.ts';

const hint = (qid: string): PanelTarget => ({ pair: pairOf({ qid, kind: 'hint', source: 'practice' }), dock: 'practice' });
const explain = (qid: string): PanelTarget => ({ pair: pairOf({ qid, kind: 'explain', source: 'practice' }), dock: 'practice' });
const card = (qid: string, n: number): PanelTarget => ({ pair: pairOf({ qid, kind: 'explain', source: 'result' }), dock: `rv-${n}` });
const idle: PanelState = { pair: null, dock: 'practice', collapsed: true };
const showing = (t: PanelTarget, collapsed = false): PanelState => ({ pair: t.pair, dock: t.dock, collapsed });

const good = (o: Partial<Answer> = {}): Answer => ({
  messageId: 1, status: 'done', error: null, truncated: false, content: 'text', lang: 'en', createdAt: 1, finishedAt: 2, sample: null, ...o,
});
const START = { type: 'event' as const, ev: { event: 'start', data: { chatId: 7, assistantMessageId: 2, lang: 'en' } }, now: 1 };
const view = (e: Entry, now?: number) => entryView(e, 'en', now);
const block: CallBlock = { cap: 'minute', limit: 5, retryAt: 5000 };

describe('pairOf', () => {
  it('question, kind and place', () => {
    assert.equal(pairOf({ qid: 'q1', kind: 'hint', source: 'practice' }), 'q1|hint|practice');
  });
});

describe('isNotAsked', () => {
  it('no entry, an empty entry and an empty peek count; saved, busy and noted entries do not', () => {
    assert.equal(isNotAsked(null), true);
    assert.equal(isNotAsked(view(emptyEntry())), true);
    assert.equal(isNotAsked(view({ ...emptyEntry(), shown: good(), attempt: good() })), false);
    assert.equal(isNotAsked(view({ ...emptyEntry(), loading: true })), false);
    assert.equal(isNotAsked(view({ ...emptyEntry(), requesting: true })), false);
    assert.equal(isNotAsked(view({ ...emptyEntry(), failure: { status: 500, code: 'ai.internal' } })), false);
    const streaming = reduce(reduce(emptyEntry(), { type: 'ask' }), START);
    assert.equal(isNotAsked(view(streaming)), false);
  });
});

describe('pressPlan', () => {
  it('collapsed (idle) + a press: switch, expand, open (memory / saved / one ask), scroll', () => {
    assert.deepEqual(pressPlan(idle, hint('q1'), true), { switchTo: true, expand: true, action: 'open', scroll: true });
  });
  it('another kind switches the one panel', () => {
    assert.deepEqual(pressPlan(showing(hint('q1')), explain('q1'), false), { switchTo: true, expand: true, action: 'open', scroll: true });
  });
  it('another question or another card switches', () => {
    assert.equal(pressPlan(showing(hint('q1')), hint('q2'), false).switchTo, true);
    assert.equal(pressPlan(showing(card('q1', 1)), card('q2', 2), false).switchTo, true);
    // the same pair in another dock is another place
    assert.equal(pressPlan(showing(card('q1', 1)), { ...card('q1', 1), dock: 'rv-2' }, false).switchTo, true);
  });
  it('the pressed button again on a filled entry: scroll only, no request, never collapses', () => {
    const p = pressPlan(showing(hint('q1')), hint('q1'), false);
    assert.deepEqual(p, { switchTo: false, expand: true, action: 'none', scroll: true });
  });
  it('the pressed button again on a notAsked entry: asks once (like the first press)', () => {
    assert.deepEqual(pressPlan(showing(hint('q1')), hint('q1'), true), { switchTo: false, expand: true, action: 'ask', scroll: true });
  });
  it('the same answer while collapsed: expands and reads through open (no blind ask), or asks when notAsked', () => {
    assert.deepEqual(pressPlan(showing(hint('q1'), true), hint('q1'), false), { switchTo: false, expand: true, action: 'open', scroll: true });
    assert.equal(pressPlan(showing(hint('q1'), true), hint('q1'), true).action, 'ask');
  });
  it('a card after a chevron expand follows the same rule', () => {
    assert.equal(pressPlan(showing(card('q1', 1)), card('q1', 1), true).action, 'ask');
    assert.equal(pressPlan(showing(card('q1', 1)), card('q1', 1), false).action, 'none');
  });
});

describe('chevronPlan', () => {
  it('idle: expands, no request (nothing to peek)', () => {
    assert.deepEqual(chevronPlan(idle, null), { switchTo: false, collapsed: false, action: 'none', scroll: false });
  });
  it('expanded: collapses, never stops or asks', () => {
    assert.deepEqual(chevronPlan(showing(hint('q1')), null), { switchTo: false, collapsed: true, action: 'none', scroll: false });
  });
  it('collapsed with an answer chosen: expands and peeks (never an ask)', () => {
    const p = chevronPlan(showing(hint('q1'), true), null);
    assert.equal(p.collapsed, false);
    assert.equal(p.action, 'peek');
  });
  it('the stub of another card: moves the panel there, expanded, peeks', () => {
    assert.deepEqual(chevronPlan(showing(card('q1', 1)), card('q2', 2)), { switchTo: true, collapsed: false, action: 'peek', scroll: false });
    assert.equal(chevronPlan(idle, card('q2', 2)).switchTo, true);
  });
  it('the live card head tapped (its own card): toggles', () => {
    assert.equal(chevronPlan(showing(card('q1', 1)), card('q1', 1)).collapsed, true);
  });
  it('never scrolls and never answers with an ask', () => {
    for (const st of [idle, showing(hint('q1')), showing(hint('q1'), true)]) {
      for (const req of [null, card('q9', 9)]) {
        const p = chevronPlan(st, req);
        assert.equal(p.scroll, false);
        assert.notEqual(p.action as string, 'ask');
      }
    }
  });
});

describe('questionChangePlan', () => {
  const cur = { pair: hint('q1').pair, kind: 'hint' };
  it('the same kind on the new question follows with a peek only', () => {
    assert.deepEqual(questionChangePlan(cur, { pair: hint('q2').pair, kind: 'hint' }), { change: 'follow', peek: true });
  });
  it('the same pair after a redraw changes nothing', () => {
    assert.deepEqual(questionChangePlan(cur, { ...cur }), { change: 'keep', peek: false });
  });
  it('the opener gone, or another kind: forget, never a request', () => {
    assert.deepEqual(questionChangePlan(cur, null), { change: 'forget', peek: false });
    assert.deepEqual(questionChangePlan(cur, { pair: explain('q1').pair, kind: 'explain' }), { change: 'forget', peek: false });
  });
  it('nothing chosen: keep (idle stays idle)', () => {
    assert.deepEqual(questionChangePlan(null, { pair: hint('q2').pair, kind: 'hint' }), { change: 'keep', peek: false });
  });
  it('cards: the dock must still hold the shown pair', () => {
    assert.equal(cardAnchorPlan(card('q1', 1).pair, card('q1', 1).pair), 'keep');
    assert.equal(cardAnchorPlan(card('q1', 1).pair, card('q2', 1).pair), 'forget');
    assert.equal(cardAnchorPlan(card('q1', 1).pair, null), 'forget');
  });
});

describe('scrollDelta', () => {
  const phone = { vh: 800, topH: 44, phone: true };
  it('PHONE: the head already between the line and 25 % of the window: no scroll', () => {
    assert.equal(scrollDelta({ ...phone, headTop: 52, panelBottom: 400 }), 0);
    assert.equal(scrollDelta({ ...phone, headTop: 150, panelBottom: 500 }), 0);
    assert.equal(scrollDelta({ ...phone, headTop: 200, panelBottom: 500 }), 0, '25 % of 800 = 200, inclusive');
  });
  it('PHONE: below the zone scrolls down so the head sits 8 px under --top-h', () => {
    assert.equal(scrollDelta({ ...phone, headTop: 600, panelBottom: 900 }), 600 - 52);
  });
  it('PHONE: above the line (hidden under the bar) scrolls up', () => {
    assert.equal(scrollDelta({ ...phone, headTop: -100, panelBottom: 200 }), -152);
    assert.equal(scrollDelta({ ...phone, headTop: 30, panelBottom: 300 }), -22, 'under the bar');
  });
  it('PHONE: --top-h 0 and 44 move the line', () => {
    assert.equal(scrollDelta({ vh: 800, topH: 0, phone: true, headTop: 500, panelBottom: 700 }), 492);
    assert.equal(scrollDelta({ vh: 800, topH: 44, phone: true, headTop: 500, panelBottom: 700 }), 448);
    assert.equal(scrollDelta({ vh: 800, topH: 0, phone: true, headTop: 8, panelBottom: 300 }), 0);
  });
  it('PHONE: a panel taller than the free area still puts the head on the line', () => {
    assert.equal(scrollDelta({ ...phone, headTop: 400, panelBottom: 2000 }), 348);
  });
  const wide = { vh: 900, topH: 0, phone: false };
  it('wide: fully visible: no scroll', () => {
    assert.equal(scrollDelta({ ...wide, headTop: 300, panelBottom: 700 }), 0);
  });
  it('wide: below the window: the minimum that shows the bottom', () => {
    assert.equal(scrollDelta({ ...wide, headTop: 700, panelBottom: 1100 }), 200);
  });
  it('wide: above the window: scrolls up to the head', () => {
    assert.equal(scrollDelta({ ...wide, headTop: -50, panelBottom: 350 }), -50);
  });
  it('wide: taller than the free area: the head goes to the top of it', () => {
    assert.equal(scrollDelta({ ...wide, headTop: 200, panelBottom: 1300 }), 200);
    assert.equal(scrollDelta({ ...wide, topH: 0, headTop: 0, panelBottom: 1300 }), 0);
  });
  it('wide: a pinned bottom bar and the top bar shrink the free area', () => {
    assert.equal(scrollDelta({ ...wide, bottomH: 80, headTop: 400, panelBottom: 860 }), 40);
    assert.equal(scrollDelta({ ...wide, topH: 44, headTop: 20, panelBottom: 300 }), -24);
  });
});

describe('focus after a press', () => {
  it('keyboard or an off-screen head: the title; else the pressed button keeps focus', () => {
    assert.equal(focusAfterPress(true, true), 'title');
    assert.equal(focusAfterPress(false, false), 'title');
    assert.equal(focusAfterPress(false, true), 'keep');
  });
  it('head visibility respects the pinned top row', () => {
    assert.equal(headVisible(100, 800, 44), true);
    assert.equal(headVisible(20, 800, 44), false);
    assert.equal(headVisible(900, 800, 0), false);
  });
});

describe('body size and phone detection', () => {
  it('phone = narrow, or a short landscape', () => {
    assert.equal(isPhone(390, 844), true);
    assert.equal(isPhone(600, 900), true);
    assert.equal(isPhone(601, 900), false);
    assert.equal(isPhone(844, 390), true);
    assert.equal(isPhone(1000, 500), true);
    assert.equal(isPhone(1000, 501), false);
    assert.equal(isShortLandscape(844, 390), true);
    assert.equal(isShortLandscape(390, 844), false);
  });
  it('wide: fixed clamp(180, 32vh, 340)', () => {
    assert.deepEqual(bodySize(1280, 800), { mode: 'fixed', height: 256 });
    assert.equal(bodySize(1280, 500 + 1).height, 180, 'the lower clamp');
    assert.equal(bodySize(1280, 1400).height, 340, 'the upper clamp');
  });
  it('phone portrait: auto, min 72, max clamp(160, 38vh, 300)', () => {
    assert.deepEqual(bodySize(390, 844), { mode: 'auto', minHeight: 72, maxHeight: 300 });
    assert.equal(bodySize(390, 500).maxHeight, 190);
    assert.equal(bodySize(390, 400).maxHeight, 160);
  });
  it('short landscape: max(120, height - 220), also when the width is narrow', () => {
    assert.deepEqual(bodySize(844, 390), { mode: 'short', maxHeight: 170 });
    assert.equal(bodySize(740, 360).maxHeight, 140);
    assert.equal(bodySize(844, 280).maxHeight, 120);
    assert.equal(bodySize(560, 360).mode, 'short');
  });
  it('overscroll: phones let the page move on; wide keeps contain', () => {
    assert.equal(bodyOverscroll(true), 'auto');
    assert.equal(bodyOverscroll(false), 'contain');
  });
  it('streaming follows the text within 24 px of the bottom only', () => {
    assert.equal(followsText(1000, 660, 320), true);
    assert.equal(followsText(1000, 600, 320), false);
  });
});

describe('footerSlots', () => {
  it('a question never asked: Copy, Regenerate and Stop are all disabled', () => {
    const s = footerSlots({ idle: false, view: view(emptyEntry()), blocked: false, hasChat: false });
    assert.equal(s.copy.disabled, true);
    assert.equal(s.regenerate.disabled, true);
    assert.equal(s.regenerate.ariaDisabled, false);
    assert.equal(s.stop.disabled, true);
    assert.equal(s.chat.visible, false);
  });
  const slots = (e: Entry, blocked = false, hasChat = false) => footerSlots({ idle: false, view: view(e), blocked, hasChat });
  const doneEntry: Entry = { ...emptyEntry(), chatId: 3, shown: good(), attempt: good() };

  it('idle: every slot disabled, the chat link invisible, no request possible', () => {
    const s = footerSlots({ idle: true, view: null, blocked: false, hasChat: false });
    assert.equal(s.copy.disabled && s.regenerate.disabled && s.stop.disabled, true);
    assert.equal(s.chat.visible, false);
    assert.equal(footerSlots({ idle: true, view: view(doneEntry), blocked: true, hasChat: true }).regenerate.ariaDisabled, false);
  });
  it('empty entry (notAsked): copy and stop disabled, nothing is being written', () => {
    const s = slots(emptyEntry());
    assert.equal(s.copy.disabled, true);
    assert.equal(s.stop.disabled, true);
    assert.equal(s.regenerate.writing, false);
  });
  it('loading: nothing to copy; stop disabled', () => {
    const s = slots({ ...emptyEntry(), loading: true });
    assert.equal(s.copy.disabled, true);
    assert.equal(s.stop.disabled, true);
    assert.equal(s.regenerate.disabled, true);
  });
  it('requesting: stop enabled, regenerate disabled and "writing"', () => {
    const s = slots({ ...emptyEntry(), requesting: true });
    assert.equal(s.stop.disabled, false);
    assert.deepEqual(s.regenerate, { disabled: true, ariaDisabled: false, writing: true });
  });
  it('streaming: copy follows the text; regenerate is the writing slot; stop enabled', () => {
    const before = slots(reduce(reduce(emptyEntry(), { type: 'ask' }), START));
    assert.equal(before.copy.disabled, true);
    const streaming = reduce(reduce(reduce(emptyEntry(), { type: 'ask' }), START), { type: 'event', ev: { event: 'delta', data: { text: 'Hi' } } });
    const s = slots(streaming);
    assert.equal(s.copy.disabled, false);
    assert.equal(s.stop.disabled, false);
    assert.equal(s.regenerate.writing, true);
    assert.equal(s.regenerate.disabled, true);
  });
  it('done: copy, regenerate enabled; stop disabled; chat visible only with a chat id', () => {
    const s = slots(doneEntry, false, true);
    assert.equal(s.copy.disabled, false);
    assert.deepEqual(s.regenerate, { disabled: false, ariaDisabled: false, writing: false });
    assert.equal(s.stop.disabled, true);
    assert.equal(s.chat.visible, true);
    assert.equal(slots(doneEntry, false, false).chat.visible, false);
  });
  it('the saved-in-AI-Chat text: visible with a chat, carrying the server title for its tip', () => {
    const withTitle = footerSlots({ idle: false, view: view(doneEntry), blocked: false, hasChat: true, chatTitle: '2026-11-12 09:30-练习' });
    assert.deepEqual(withTitle.chat, { visible: true, title: '2026-11-12 09:30-练习' });
    assert.deepEqual(footerSlots({ idle: false, view: view(doneEntry), blocked: false, hasChat: true }).chat, { visible: true, title: null });
    // No chat: the slot keeps its place but is invisible, and never offers a title
    assert.deepEqual(footerSlots({ idle: false, view: view(doneEntry), blocked: false, hasChat: false, chatTitle: 'x' }).chat, { visible: false, title: null });
  });
  it('savedInChat: the text needs this answer in the chat, not only the session chat', () => {
    assert.equal(savedInChat(undefined), false);
    assert.equal(savedInChat(emptyEntry()), false);
    assert.equal(savedInChat(doneEntry), true);
    // The saved route of a question not asked yet: the session chat exists, this answer does not
    const notAsked = reduce(emptyEntry(), { type: 'saved', saved: { chatId: 3, chatTitle: '2026-11-12 09:30-练习', latest: null, lastGood: null } });
    assert.equal(notAsked.chatId, 3);
    assert.equal(savedInChat(notAsked), false);
    // An answer being written is saved from its start (the start event brings the chat id and its title)
    const start3 = { type: 'event' as const, ev: { event: 'start', data: { chatId: 3, chatTitle: '2026-11-12 09:30-练习', assistantMessageId: 2 } }, now: 1 };
    const started = reduce(reduce(notAsked, { type: 'ask' }), start3);
    assert.equal(savedInChat(started), true);
    assert.equal(started.chatTitle, '2026-11-12 09:30-练习');
    assert.equal(savedInChat(reduce(reduce(emptyEntry(), { type: 'ask' }), START)), true);
    // A request refused before any stream saves nothing
    assert.equal(savedInChat(reduce(notAsked, { type: 'askFailed', error: { status: 500, code: 'ai.internal' } })), false);
  });
  it('the panel never navigates to AI Chat (the footer text is not a link)', () => {
    const src = readFileSync(new URL('../components/ai-panel.ts', import.meta.url), 'utf8');
    assert.equal(/openChat|navigate\(|location\.hash/.test(src), false);
  });
  it('the saved-in-chat tip names the chat like AI Chat (title plus kind suffix)', () => {
    const src = readFileSync(new URL('../components/ai-panel.ts', import.meta.url), 'utf8');
    assert.ok(src.includes('chatNameOf({ title, kind }'));
  });
  it('stopped and error phases can regenerate', () => {
    const stopped = reduce(reduce(reduce(emptyEntry(), { type: 'ask' }), START), { type: 'localStop' });
    assert.equal(slots(stopped).regenerate.disabled, false);
    const failed = reduce(emptyEntry(), { type: 'askFailed', error: { status: 500, code: 'ai.internal' } });
    assert.equal(slots(failed).regenerate.disabled, false);
  });
  it('a busy refusal with a running answer elsewhere cannot regenerate (the note row has the busy buttons)', () => {
    const running = { chatId: 9, messageId: 1, chatKind: 'practice', kind: 'hint', questionId: 'q1', questionNo: 1, source: 'practice', recordId: null } as const;
    const busy = reduce(emptyEntry(), { type: 'askFailed', error: { status: 409, code: 'ai.busy', running } });
    assert.equal(slots(busy).regenerate.disabled, true);
  });
  it('blocked by the caps: regenerate is aria-disabled, not disabled; a real disable stays disabled', () => {
    assert.deepEqual(slots(doneEntry, true).regenerate, { disabled: false, ariaDisabled: true, writing: false });
    const writing = slots({ ...emptyEntry(), requesting: true }, true).regenerate;
    assert.equal(writing.ariaDisabled, false);
    assert.equal(writing.disabled, true);
    assert.equal(slots(doneEntry, true).copy.disabled, false, 'a saved answer stays copyable');
  });
  it('the regenerate guard ignores a press while requesting or streaming', () => {
    assert.equal(regenerateAllowed(undefined), true);
    assert.equal(regenerateAllowed({ requesting: true, attempt: null }), false);
    assert.equal(regenerateAllowed({ requesting: false, attempt: { status: 'streaming' } }), false);
    assert.equal(regenerateAllowed({ requesting: false, attempt: { status: 'done' } }), true);
  });
});

describe('capNote', () => {
  const saved = view({ ...emptyEntry(), shown: good(), attempt: good() });
  const refused = view({ ...emptyEntry(), failure: { status: 429, code: 'ai.callLimit', quota: block } });
  it('blocked + expanded on a notAsked entry: the callout, without alert', () => {
    assert.deepEqual(capNote({ block, expanded: true, view: view(emptyEntry()) }), { show: true, alert: false, source: 'standing' });
    assert.equal(capNote({ block, expanded: true, view: null }).show, true);
  });
  it('blocked + saved answer: no callout (the answer stays readable)', () => {
    assert.deepEqual(capNote({ block, expanded: true, view: saved }), { show: false, alert: false, source: 'none' });
  });
  it('collapsed or not blocked: nothing', () => {
    assert.equal(capNote({ block, expanded: false, view: view(emptyEntry()) }).show, false);
    assert.equal(capNote({ block: null, expanded: true, view: view(emptyEntry()) }).show, false);
  });
  it('a refused press: the entry note is the callout, with alert', () => {
    assert.deepEqual(capNote({ block, expanded: true, view: refused }), { show: true, alert: true, source: 'refused' });
  });
  it('a refused regenerate on a saved answer keeps the answer and shows the alert note', () => {
    const v = view({ ...emptyEntry(), shown: good(), attempt: good(), failure: { status: 429, code: 'ai.callLimit', quota: block } });
    assert.equal(v.text, 'text');
    assert.equal(capNote({ block, expanded: true, view: v }).source, 'refused');
  });
  it('the refusal expires with its retryAt: the saved answer or the prompt shows again, the block callout follows the block', () => {
    const e: Entry = { ...emptyEntry(), failure: { status: 429, code: 'ai.callLimit', quota: block } };
    assert.equal(capNote({ block, expanded: true, view: view(e, 5000) }).source, 'standing');
    assert.equal(capNote({ block: null, expanded: true, view: view(e, 5000) }).show, false);
  });
  it('another note (busy, error) beats the standing callout', () => {
    const busyErr = view({ ...emptyEntry(), failure: { status: 500, code: 'ai.internal' } });
    assert.equal(capNote({ block, expanded: true, view: busyErr }).show, false);
  });
  it('the alert gate announces each new note once', () => {
    const g = createAlertGate();
    assert.equal(g.take('callLimit'), true);
    assert.equal(g.take('callLimit'), false);
    assert.equal(g.take(null), false);
    assert.equal(g.take('callLimit'), true, 'after it went away, it is announced again');
    assert.equal(g.take('other'), true);
  });
});

describe('FB-7 collapsed head state on phones', () => {
  it('shrinks with an ellipsis at <= 400 px and hides only below 360 px', () => {
    const css = readFileSync(new URL('../styles/ai-panel.css', import.meta.url), 'utf8');
    const b400 = css.match(/@media \(max-width: 400px\) \{([\s\S]*?)\n\}/)?.[1] ?? '';
    assert.ok(b400, 'a 400 px block');
    assert.ok(!/aip-head-state \{ display: none/.test(b400), 'not hidden at 400 px');
    for (const part of ['min-width: 0', 'flex: 0 1 auto', 'overflow: hidden', 'text-overflow: ellipsis', 'white-space: nowrap']) {
      assert.ok(b400.includes(part), part);
    }
    const b359 = css.match(/@media \(max-width: 359px\) \{([\s\S]*?)\n\}/)?.[1] ?? '';
    assert.ok(/aip-head-state \{ display: none; \}/.test(b359), 'hidden below 360 px');
    // The busy dot stands in for the text only where the text is hidden (spec 2.2 "in its place")
    assert.ok(!b400.includes('aip-dot'), 'no dot beside the text at 360 to 400 px');
    assert.ok(/\.aip-dot:not\(\[hidden\]\) \{ display: inline-block; \}/.test(b359), 'the dot below 360 px');
  });
});
