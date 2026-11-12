# -*- coding: utf-8 -*-
"""The screenshot scenes: headless Chrome drives the real UI through DOM clicks, one browser per language.

shoot_language() writes, into one folder, the 27 images the READMEs and docs use (20 desktop images 1280 px wide and 7
iPhone images 390 x 844, both at device scale 2). Since v1.0 the UI is phone-first (icons with tap-and-hold labels, a
one-row practice bar, a collapsible AI+ panel under the card, a centered GitHub footer line); the scenes follow it:

  desktop-1-home        Home with the saved exam to continue (green Start exam, orange Start practice), the whole page down
                        to the compact footer (disclaimer, then the centered "GitHub | Xexam <version>" line)
  desktop-2-quiz        an exam in progress, on a hotspot question with its first item answered
  desktop-3-result      the result of a whole exam: the score and the first answer card (an orange wrong option, the verdict)
  desktop-4-records     Study records
  desktop-5-banks       Question banks: the preview of an update ("Kitchen basics (v2)") and the guide unfolded, shortened
                        to its first two sections (basics and the six columns) so the whole guide card ends inside the image
  desktop-6-settings    Settings, as the administrator: with the AI chat mode section (Normal selected; administrators only)
  desktop-7-dark        the exam page in the dark theme
  desktop-8-about       About: the newest changelog entry shortened to the bullets that fit, so the whole What's new card
                        (with the folded earlier versions) ends inside the image
  desktop-9-signin      the sign-in page
  desktop-10-users      Users: the AI+ feature line of each user (shown while AI+ is available), the bright AI+ badge
                        of the administrators and sam, the dark one of alex (AI+ feature paused), none for kim
  desktop-11-practice   practice: one question checked right, the next one checked wrong (orange) with the right ones green;
                        the bar shows BOTH Check answer (greyed once checked) and Next, and the collapsed AI+ panel is
                        under the card
  desktop-12-user-banks the "Question banks" dialog of a user
  desktop-13-logs       System log, the whole card (lead, filters, lines, note) in view
  desktop-14-ai-chat    AI Chat as the administrator: a chat started through the API opened, a second question typed
                        into its composer and sent through the UI, its answer finished; the administrator notice, the
                        green "Free mode" tag right of the title (the scene turns the administrator's free mode on through
                        POST /api/ai/free first and off again afterwards; AI Chat itself has no mode switch, the choice
                        is in Settings), the who line (You / AI, model, time) above and
                        Copy (with Remember this / Retry) under every message are visible. The window is made just tall
                        enough for the whole conversation box and the image ends below it. The chat is deleted again
                        through the API, so every language starts without chats
  desktop-15-ai-settings AI settings, tab "AI+ users" (#/ai-settings?tab=users): the status chips and the ai-service line
                        above the category tabs, the users with the AI+ feature (badge, calls line, AI+ feature: alex Paused
                        with Resume, sam Active with Pause; administrators always). The scene first presses Refresh list and
                        Test on the "AI+ function" tab (the connection with the made-up address and the masked made-up key)
  desktop-16-ai-usage   AI settings, tab "AI usage": the usage statistics with the "Today" column and the cached-tokens
                        column (they grow by a few calls per language)
  desktop-17-ai-hint    a practice question of the made-up demo bank with the AI+ panel expanded on the AI hint (the
                        screenshot user has the AI+ feature); the stub's hint, in the UI language, and the muted "Saved in
                        AI Chat" text (no Open in AI Chat control: nothing in the panel navigates away)
  desktop-18-ai-result  the result of the exam: card 1 with the AI+ panel expanded on the AI analysis (AI 解析), card 2's
                        collapsed AI+ bar below it
  desktop-19-ai-summary Study records with the AI summary window open: the "based on" line, the stub's four headings and the
                        "Saved in AI Chat" text
  desktop-20-ai-safety  AI settings, tab "AI safety": chat retention and the safety limits (the per-user request caps, the
                        "Answer and memory length" group, the server protection)
  iphone-1-home         Home with the latest card (one-row actions) and the start bar
  iphone-2-quiz         an exam: compact top row, one-row bar (previous, Next, flag), collapsed answer sheet
  iphone-3-menu         the menu
  iphone-4-records      Study records: compact KPIs, folded filters, three-row cards
  iphone-5-ai-hint      the practice page with the AI+ panel expanded on the AI hint, scrolled into view under the top row
  iphone-6-result       the exam result: compact score, collapsed strip, cards with their AI+ bar
  iphone-7-ai-chat      AI Chat full height with a short conversation

Layout checks (assertions, not images) run in the same pass, in every language: at 360, 390 and 412 px the practice bar is
ONE row with Check answer and Next as two separate buttons in fixed places (nothing moves between an unchecked and a
checked question, or between questions), 44 px touch targets, no sideways overflow; the paused user's greyed AI+ bar is
visible at 390 and 360 px; the exam bar is one row without a check button; the AI+ panel and the AI summary window show
the "Saved in AI Chat" text, are not links / buttons and have no Open in AI Chat control; no title focused by script draws
a focus frame; the footer line is centered with its two links (Home, phone Home, AI Chat list pane); AI settings has the
four category tabs and, in AI safety, the "Answer and memory length" group; Study records cards are at most 140 px tall at
360 px; at 800 px the Study records list (en and ja) is in the card layout with no horizontal scroll and its Delete buttons
on one line. The AI asks of the scenes are saved in AI Chat like any other; the run deletes the administrator's AI chats
before the AI Chat scenes and at the end.

The made-up AI provider (tests/integration/ai_stub.py) answers in the reply language the page sends (the Settings
language), so each language's AI Chat image shows questions and answers in that language. Every value on the AI images
is made up (the stub's address, the key hint ...9c21, the demo-* models); real addresses, keys or models never appear.

Every desktop image is cut where no text line, control or card is split (see FIT_JS) or measured to its card.
The page clock runs in UTC and the browser's language follows the UI language.
"""
import base64
import json
import os
import time

import seed
from browser import Chrome, strip_png

SCENES = ['desktop-%d-%s.png' % kv for kv in enumerate(
    ['home', 'quiz', 'result', 'records', 'banks', 'settings', 'dark', 'about', 'signin', 'users', 'practice',
     'user-banks', 'logs', 'ai-chat', 'ai-settings', 'ai-usage', 'ai-hint', 'ai-result', 'ai-summary', 'ai-safety'],
    start=1)] + [
    'iphone-1-home.png', 'iphone-2-quiz.png', 'iphone-3-menu.png', 'iphone-4-records.png', 'iphone-5-ai-hint.png',
    'iphone-6-result.png', 'iphone-7-ai-chat.png']

BROWSER_LANG = {'en': 'en-US', 'zh-CN': 'zh-CN', 'ja': 'ja'}
UI_LANG = {'en': 'en', 'zh-CN': 'zh', 'ja': 'ja'}          # the uiLang / questionLang setting
DESKTOP_UA = 'Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0.0.0 Safari/537.36'
IPHONE_UA = ('Mozilla/5.0 (iPhone; CPU iPhone OS 17_5 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) '
             'Version/17.5 Mobile/15E148 Safari/604.1')
WIDTH, PHONE_W, PHONE_H = 1280, 390, 844
# Self-made study questions of the AI Chat scene, per UI language: the earlier chat, then the one typed into the composer
AI_QUESTIONS = {
    'en': ('How can I remember new vocabulary for my exam?', 'Can you give me a short plan for one week?'),
    'zh': ('怎样才能更好地记住考试要用的新词汇？', '能给我一个一周的简短复习计划吗？'),
    'ja': ('試験のために新しい単語をうまく覚えるにはどうすればいいですか？', '一週間の短い復習プランを作ってもらえますか？'),
}

# Installed in every page before its own scripts
PRELUDE = r"""
(() => {
  const real = Date.now.bind(Date);
  window.__skew = 0;                                  // demo only: pretend that time has passed (a script answers in a second)
  Date.now = () => real() + window.__skew;
  window.__wait = ms => new Promise(r => setTimeout(r, ms));
  window.__qa = s => [...document.querySelectorAll(s)];
  // a box that is really on screen (not display: none, not visibility: hidden, not empty)
  window.__shown = s => {
    const e = typeof s === 'string' ? document.querySelector(s) : s;
    if (!e) return false;
    const cs = getComputedStyle(e), r = e.getBoundingClientRect();
    return cs.display !== 'none' && cs.visibility !== 'hidden' && r.width > 0 && r.height > 0;
  };
  // the question on screen, found in /api/bank by its stem in any of the three languages (six-column questions: English in
  // stem, Chinese in zh, Japanese in ja); the key is the first line long enough to tell questions apart (not "HOTSPOT")
  window.__current = async () => {
    window.__bank = window.__bank || (await (await fetch('/api/bank')).json()).questions;
    const shown = document.querySelector('.stem').innerText.replace(/\s+/g, '');
    const key = s => {
      const ls = (s || '').split('\n').map(x => x.trim()).filter(Boolean);
      return (ls.find(l => l.length > 12) || ls[0] || '\u0000').replace(/\s+/g, '').slice(0, 40);
    };
    return window.__bank.find(q => [q.stem, q.en?.stem, q.zh?.stem, q.ja?.stem].some(s => s && shown.includes(key(s))));
  };
  window.__en = q => (q.lang === 'en' ? q.stem : q.en?.stem) || '';
  // answer the question on screen; bad = answer it wrongly
  window.__answer = async (bad) => {
    const q = await window.__current();
    if (q.kind === 'hotspot') {
      q.items.forEach((it, k) => window.__qa('.hs-item')[k].querySelectorAll('.sheet-chip')[bad && k === 0 ? (q.answer[k] + 1) % it.choices.length : q.answer[k]].click());
    } else {
      const keys = bad ? [q.options.find(o => !q.answer.includes(o.key)).key] : q.answer;
      keys.forEach(k => window.__qa('.opt').find(b => b.querySelector('.bubble').textContent === k).click());
    }
    return q.kind;
  };
  // wait until the text of `sel` (a hint, an explanation, a summary) has appeared and stopped growing; returns its length
  window.__settle = async (sel) => {
    let last = -1, still = 0;
    for (let i = 0; i < 120; i++) {
      await window.__wait(150);
      const n = (document.querySelector(sel)?.innerText || '').length;
      still = n > 30 && n === last ? still + 1 : 0;
      last = n;
      if (still >= 6) return n;
    }
    return -1;
  };
  // Start exam / Start practice from Home in bank order (shuffle off), so every language shows the same questions
  window.__start = async (kind) => {
    const sh = document.querySelector('.home-shuffle input');
    if (sh && sh.checked) sh.click();
    document.querySelector(kind === 'practice' ? '.page-home .home-go-btns .btn.big:not(.primary)' : '.page-home .home-go-btns .btn.primary.big').click();
    await window.__wait(450);
  };
})();
"""

JS_SETUP = r"""(async (lang, theme) => {
  const post = (u, b) => fetch(u, {method: 'POST', headers: {'Content-Type': 'application/json'}, body: JSON.stringify(b || {})});
  const r = await post('/api/settings', {uiLang: lang, questionLang: lang, theme, autoAdvance: false});
  if (!r.ok) throw new Error('POST /api/settings: ' + r.status);
  localStorage.removeItem('xexam.settings.v1');       // only the first-paint cache; a saved exam stays
  return 'settings ' + lang + ' ' + theme;
})"""

# A whole exam through the UI: every question, two of them wrong (the fruit question and the first item of the sun hotspot)
JS_EXAM = r"""(async () => {
  const wait = window.__wait, qa = window.__qa, q1 = s => document.querySelector(s);
  await window.__start('exam');
  const n = qa('.tile').length;
  for (let i = 0; i < n; i++) {
    qa('.tile')[i].click(); await wait(60);
    const q = await window.__current();
    const bad = /fruits/i.test(window.__en(q)) || (q.kind === 'hotspot' && /sun rises/i.test(JSON.stringify(q.items || []) + window.__en(q)));
    await window.__answer(bad);
  }
  window.__skew = 394000;                              // the result shows about 6:34
  q1('[data-focus="rail-submit"]').click(); await wait(250);
  q1('dialog[open] .btn.primary').click(); await wait(1600);
  window.scrollTo(0, 0);
  return q1('.score .big')?.getAttribute('aria-label') + ' / ' + n + ' questions';
})()"""

# Start an exam, answer the first questions, open the first hotspot question and answer its first item
JS_QUIZ = r"""(async () => {
  const wait = window.__wait, qa = window.__qa;
  window.__skew = 0;
  await window.__start('exam');
  let hs = -1;
  for (let i = 0; i < qa('.tile').length; i++) {
    qa('.tile')[i].click(); await wait(60);
    if (document.querySelector('.hs-item') && hs < 0) { hs = i; continue; }
    if (i < 3 && !document.querySelector('.hs-item')) await window.__answer(false);
  }
  qa('.tile')[hs].click(); await wait(120);
  const q = await window.__current();
  qa('.hs-item')[0].querySelectorAll('.sheet-chip')[q.answer[0]].click();
  window.__skew = 83000;
  await wait(1200);
  document.activeElement?.blur?.();
  window.scrollTo(0, 0);
  return 'hotspot at ' + (hs + 1);
})()"""

# Practice: question 1 right (single choice, checked on pick), then the fruit question (multiple choice): A (right) and
# B (wrong) chosen, Check answer -> wrong: B orange, A and C green, the explanation and the memory aid
JS_PRACTICE = r"""(async () => {
  const wait = window.__wait, qa = window.__qa, q1 = s => document.querySelector(s);
  window.__skew = 0;
  await window.__start('practice');
  await window.__answer(false); await wait(300);
  let found = -1;
  for (let i = 0; i < qa('.tile').length; i++) {
    qa('.tile')[i].click(); await wait(80);
    const q = await window.__current();
    if (/fruits/i.test(window.__en(q))) { found = i; break; }
  }
  ['A', 'B'].forEach(k => qa('.opt').find(b => b.querySelector('.bubble').textContent === k).click());
  await wait(100);
  q1('[data-focus="q-check"]').click(); await wait(500);
  window.__skew = 71000;
  await wait(1200);
  document.activeElement?.blur?.();
  window.scrollTo(0, 0);
  return 'question ' + (found + 1) + ': ' + (q1('.pverdict')?.innerText.split('\n')[0] || 'no verdict');
})()"""

JS_SAVE_EXIT = r"""(async () => { document.querySelector('.qbar .qexit').click(); await window.__wait(400); window.scrollTo(0, 0); return location.hash; })()"""

JS_CLEAR_SAVED = r"""Object.keys(localStorage).filter(k => /session\.v1$/.test(k)).forEach(k => localStorage.removeItem(k)); 'cleared'"""

JS_RESUME = r"""(async () => { document.querySelector('.resume .btn.primary')?.click(); await window.__wait(600); window.scrollTo(0, 0); return location.hash })()"""

JS_UPDATE = r"""(async () => {
  const card = window.__qa('.bk-bank').find(c => /Kitchen basics/.test(c.innerText));
  card.querySelector('.bk-bankacts [id^="bk-update-"]').click(); await window.__wait(400);
  return document.querySelector('#bk-target') ? 'update target set' : 'no update target';
})()"""

# The height (from the current scroll position, at most `cap`) at which to cut a tall page so that no text line, control
# or card is split: the lowest point that no such box crosses, plus up to 24 px of the free space below the last one.
# Boxes clipped by a scrolling ancestor count only where they are visible; fixed elements, and cards that start too high
# to be kept whole (page-sized wrappers), are ignored.
FIT_JS = r"""((cap, minimum) => {
  const top = window.scrollY, docH = Math.ceil(document.documentElement.scrollHeight);
  if (docH - top <= cap) return Math.max(minimum, docH - top);
  const limit = top + cap, leaves = [], frames = [];
  const clipOf = el => {
    let lo = -Infinity, hi = Infinity;
    for (let a = el.parentElement; a && a !== document.body; a = a.parentElement) {
      if (getComputedStyle(a).overflowY !== 'visible') {
        const r = a.getBoundingClientRect();
        lo = Math.max(lo, r.top + scrollY); hi = Math.min(hi, r.bottom + scrollY);
      }
    }
    return [lo, hi];
  };
  for (const el of document.body.querySelectorAll('*')) {
    const cs = getComputedStyle(el);
    if (cs.display === 'none' || cs.visibility === 'hidden' || cs.position === 'fixed' || el.closest('dialog:not([open])')) continue;
    const text = [...el.childNodes].some(n => n.nodeType === 3 && n.textContent.trim());
    const atomic = el.matches('img, svg, button, input, select, textarea, canvas, video, hr');
    const framed = !text && !atomic && ((parseFloat(cs.borderTopWidth) > 0 && cs.borderTopStyle !== 'none') ||
      !/^(rgba\(0, 0, 0, 0\)|transparent)$/.test(cs.backgroundColor) || cs.boxShadow !== 'none');
    if (!text && !atomic && !framed) continue;
    const [lo, hi] = clipOf(el);
    for (const r of el.getClientRects()) {
      const a = Math.max(r.top + scrollY, lo), b = Math.min(r.bottom + scrollY, hi);
      if (r.width < 1 || b - a < 1) continue;
      if (framed && a < top + minimum) continue;              // starts too high to be kept whole (a page-sized wrapper)
      (framed ? frames : leaves).push([a, b]);
    }
  }
  const cut = boxes => {
    let y = limit;
    for (;;) {
      const hit = boxes.filter(b => b[0] < y && b[1] > y);
      if (!hit.length) break;
      y = Math.min(...hit.map(b => b[0]));
    }
    if (y < top + minimum) return null;
    const above = boxes.filter(b => b[1] <= y).map(b => b[1]);
    return Math.min(y, (above.length ? Math.max(...above) : y) + 24) - top;
  };
  return Math.floor(cut(leaves.concat(frames)) ?? cut(leaves) ?? cap);
})"""

# The System log card: its bottom edge (page coordinates) and the number of lines
JS_LOG_CARD = r"""(() => {
  window.scrollTo(0, 0);
  const c = document.querySelector('.lg-panel');             // the card: filters, search, status, the lines (the note is a tip since v1.0)
  return [Math.ceil(c.getBoundingClientRect().bottom + window.scrollY), document.querySelectorAll('.lg-line').length];
})()"""

# Question banks: unfold the guide (a click on its summary, as a reader would) and hide everything of it after its first
# two sections (basics, the six columns; the whole guide is several thousand pixels tall), then the bottom of the two
# columns (page coordinates), whether the guide is open and how many of its parts were hidden
JS_BANKS_GUIDE = r"""(async () => {
  const d = document.querySelector('#bk-guide-d');
  if (d && !d.open) { document.querySelector('#bk-guide-s').click(); await window.__wait(500); }
  const third = document.querySelectorAll('#bk-guide-d .bk-guide-body > h3')[2];
  let hidden = 0;
  for (let el = third; el; el = el.nextElementSibling) { el.style.display = 'none'; hidden++; }
  window.scrollTo(0, 0);
  const grid = document.querySelector('.page-banks .bk-grid');
  return [Math.ceil(grid.getBoundingClientRect().bottom + window.scrollY), !!document.querySelector('#bk-guide-d[open]'),
          hidden];
})()"""

# About: hides the last top-level bullets of the newest changelog entry (and nothing else) until the bottom of the What's
# new card is at most `cap` px down the page, keeping at least one bullet; returns that bottom (page coordinates) and the
# number of bullets kept and hidden
JS_ABOUT = r"""((cap) => {
  window.scrollTo(0, 0);
  const card = document.querySelector('.ab-news');
  const items = [...document.querySelectorAll('.ab-first .ab-body > ul > li')];
  const bottom = () => Math.ceil(card.getBoundingClientRect().bottom + window.scrollY);
  let shown = items.length;
  while (shown > 1 && bottom() > cap) items[--shown].style.display = 'none';
  return [bottom(), shown, items.length - shown];
})"""


# Users: what the AI+ parts of the page show (for the log of the run)
JS_USERS_STATE = r"""(() => {
  const n = s => document.querySelectorAll(s).length;
  return n('.ai-badge:not(.off)') + ' bright badge(s), ' + n('.ai-badge.off') + ' dark, ' + n('.us-ailine') + ' AI+ line(s)';
})()"""

# AI settings: the AI+ feature cells of the users with the AI+ feature (for the log of the run)
JS_AI_FUNCTIONS = r"""(() => [...document.querySelectorAll('[id^="ais-fn-"]')].map(b => b.closest('tr')?.innerText
  .replace(/\s+/g, ' ').trim().slice(0, 60)).join(' | ') || 'no Pause / Resume button')()"""

# AI Chat: open the chat whose title starts like `title` in the chat list
JS_AI_OPEN = r"""(async (title) => {
  const list = document.querySelector('.ai-list');
  if (!list || !window.__shown(list)) {
    return document.querySelectorAll('.ai-msg').length ? document.querySelectorAll('.ai-msg').length + ' messages shown (already open)' : 'no chat list';
  }
  const key = title.replace(/\s+/g, ' ').slice(0, 16);
  const item = [...list.querySelectorAll('button, a, [role="button"]')].find(el => (el.innerText || '').includes(key));
  if (!item) return 'chat not found';
  item.click(); await window.__wait(900);
  return document.querySelectorAll('.ai-msg').length + ' messages shown';
})"""

# AI Chat: focus the composer (the text itself is typed through the DevTools protocol)
JS_AI_FOCUS = r"""(() => { const t = document.querySelector('#ai-input'); if (!t) return 'no composer'; t.focus(); return 'focused'; })()"""

# AI Chat: press Send and wait until the answer has finished (no streaming message left, two answers shown)
JS_AI_SEND = r"""(async () => {
  document.querySelector('#ai-send').click();
  let n = 0;
  for (let i = 0; i < 150; i++) {
    await window.__wait(200);
    n = document.querySelectorAll('.ai-msg.assistant').length;
    if (i > 3 && n >= 2 && !document.querySelector('.ai-msgs [aria-busy="true"]')) break;
  }
  await window.__wait(600);
  document.activeElement?.blur?.();
  window.scrollTo(0, 0);
  return n + ' answers; notice ' + (document.querySelector('.ai-notice') ? 'shown' : 'MISSING') +
    '; free-mode tag ' + (document.querySelector('.ai-mode-tag') ? 'shown' : 'MISSING') +
    '; mode switch ' + (document.querySelector('#ai-free') ? 'STILL THERE' : 'none');
})()"""

# AI Chat: scroll the conversation box to its end; returns how many px of the conversation do not fit in the box
JS_AI_OVER = r"""(() => {
  const sc = document.getElementById('ai-scroll');
  if (!sc) return 0;
  sc.scrollTop = sc.scrollHeight;
  return Math.ceil(sc.scrollHeight - sc.clientHeight);
})()"""

# AI Chat: the bottom (page coordinates) of the chat list and the conversation
JS_AI_PANES_BOTTOM = r"""(() => {
  window.scrollTo(0, 0);
  const p = document.getElementById('ai-panes');
  if (!p) return null;
  // 24 px below the panes, but never into the page footer (a one-line disclaimer puts the footer closer)
  const bottom = Math.ceil(p.getBoundingClientRect().bottom + window.scrollY) + 24;
  const f = document.querySelector('footer.foot');
  return f ? Math.min(bottom, Math.floor(f.getBoundingClientRect().top + window.scrollY)) : bottom;
})()"""

# AI settings: press Refresh list (the line "N models found" appears under the model), then Test, and wait for each
# result (the page reads the usage again after both, so the Usage card counts these calls too)
JS_AI_TEST = r"""(async () => {
  const wait = window.__wait, q = s => document.querySelector(s);
  const inCard = (heading, s) => document.getElementById(heading)?.closest('section')?.querySelector(s);
  const until = async (ok, ms) => {
    for (let t = 0; t < ms; t += 100) { await wait(100); if (ok()) return true; }
    return false;
  };
  await until(() => q('#ai-test-btn'), 10000);                  // the page reads its settings first
  // a saved and tested model is one line with a pencil (v1.0): open it to get the model field with Refresh list
  if (!q('#ai-models-btn') && q('#ai-model-edit')) { q('#ai-model-edit').click(); await until(() => q('#ai-models-btn'), 3000); }
  if (!q('#ai-models-btn') || !q('#ai-test-btn')) {
    return 'no Refresh list / Test button (page: ' + document.querySelector('#app, main, body').innerText.replace(/\s+/g, ' ').slice(0, 300) + ')';
  }
  q('#ai-models-btn').click();
  const listed = await until(() => !q('#ai-models-btn')?.disabled && inCard('ais-model-h', '.ais-note[role="status"]'), 8000);
  const note = inCard('ais-model-h', '.ais-note[role="status"]')?.innerText || '';
  q('#ai-test-btn').click();
  const tested = await until(() => !q('#ai-test-btn')?.disabled && inCard('ais-test-h', '.ais-ok, .ais-fail, .ais-ferr'), 15000);
  const result = inCard('ais-test-h', '.ais-ok')?.innerText || 'TEST FAILED';
  await wait(1200);                                    // the usage tables are read again after the test
  document.activeElement?.blur?.();
  window.scrollTo(0, 0);
  return (listed ? note : 'NO MODEL LIST') + '; ' + (tested ? result : 'NO TEST RESULT');
})()"""

# AI settings: [top, bottom] (page coordinates) of the card whose heading has the id `first`, through the card whose
# heading has the id `last`
JS_AI_CARDS = r"""((first, last) => {
  window.scrollTo(0, 0);
  const card = id => document.getElementById(id)?.closest('section');
  const a = card(first), b = card(last);
  if (!a || !b) return null;
  const y = window.scrollY;
  return [Math.floor(a.getBoundingClientRect().top + y), Math.ceil(b.getBoundingClientRect().bottom + y)];
})"""


# AI scenes. Practice: start a practice from Home, open the AI+ panel on the AI hint with the button of the action bar,
# wait until the stub's hint is complete and the "Saved in AI Chat" text is there; with `phone`, the panel is scrolled just
# under the pinned top row. Returns what happened (for the log of the run)
JS_AI_HINT = r"""(async (phone) => {
  window.__skew = 0;
  await window.__start('practice');
  const b = document.querySelector('[data-focus="q-aihint"]');
  if (!b) return 'NO AI HINT BUTTON';
  if (b.getAttribute('aria-disabled') === 'true') return 'AI HINT BUTTON GREYED OUT';
  b.click();
  const n = await window.__settle('#aip-panel-body');
  for (let i = 0; i < 40 && !window.__shown('#aip-panel .aip-saved'); i++) await window.__wait(150);
  if (phone) {
    const p = document.querySelector('#aip-panel');
    const top = parseFloat(getComputedStyle(document.documentElement).getPropertyValue('--top-h')) || 0;
    if (p) window.scrollBy(0, p.getBoundingClientRect().top - top - 8);
    await window.__wait(300);
  }
  const open = document.querySelector('#aip-panel') && !document.querySelector('#aip-panel.collapsed');
  return n < 0 ? 'HINT DID NOT FINISH' : 'hint of ' + n + ' characters; panel ' + (open ? 'expanded' : 'NOT EXPANDED');
})"""

# What the owner's feedback asks of an AI+ window (the panel `#aip-panel` or the summary `dialog.aip-sum`): the muted "Saved
# in AI Chat" text is visible, is plain text (not a button or a link), nothing in the window navigates or names an Open in
# AI Chat control, and no element focused by script (a title, a card: tabindex -1) draws a focus frame. Returns
# [problems, the text of the tip that names the chat]
JS_STUDY_CHECK = r"""((scope, saved) => {
  const bad = [];
  const root = document.querySelector(scope);
  if (!root) return [['no ' + scope], ''];
  const s = root.querySelector(saved);
  let tip = '';
  if (!s) bad.push('no Saved text (' + saved + ')');
  else {
    if (!window.__shown(s)) bad.push('the Saved text is not visible');
    if (/^(BUTTON|A)$/.test(s.tagName) || s.closest('button, a')) bad.push('the Saved text is a button or a link');
    if (!s.textContent.trim()) bad.push('the Saved text is empty');
    tip = s.parentElement?.querySelector('.tip[role="tooltip"]')?.textContent.trim() || '';
  }
  if (root.querySelector('a[href]')) bad.push('a link inside ' + scope);
  if (root.querySelector('button[data-slot="chat"]')) bad.push('an Open in AI Chat button inside ' + scope);
  for (const el of root.querySelectorAll('button, a, [role="button"]')) {
    const name = (el.getAttribute('aria-label') || el.title || '').trim();
    if (/open.*AI Chat|AI 对话|AI チャット/i.test(name) && !/Saved|已保存|保存/.test(name)) bad.push('a control that names AI Chat: ' + name);
  }
  for (const el of [root, ...root.querySelectorAll('[tabindex="-1"]')]) {
    if (el.getAttribute('tabindex') !== '-1') continue;
    el.focus({preventScroll: true});
    const cs = getComputedStyle(el);
    if (cs.outlineStyle !== 'none' && parseFloat(cs.outlineWidth) > 0) bad.push('a focus frame on ' + (el.id || el.className || el.tagName));
  }
  return [bad, tip];
})"""

# The footer of the page (FB-4): the disclaimer, then the centered "GitHub | Xexam <version>" line as the last line; no old
# left-aligned "Xexam v..." line; both links open in a new tab with rel noopener noreferrer. `root` limits the search (the AI
# Chat list pane); `touch` also asks for 44 px tall links. Returns [problems, notes]
JS_FOOTER = r"""((root, touch) => {
  const bad = [], notes = [];
  const scope = (root && document.querySelector(root)) || document;
  const line = scope.querySelector('.gh-line');
  if (!line) return [['no GitHub line (.gh-line) in ' + (root || 'the page')], notes];
  const links = [...line.querySelectorAll('a.gh-link')];
  if (links.length !== 2) bad.push('the GitHub line has ' + links.length + ' links (two expected)');
  const hrefs = links.map(a => a.getAttribute('href'));
  if (hrefs[0] !== 'https://github.com/Yumin-Space/xexam') bad.push('the GitHub link points to ' + hrefs[0]);
  if (hrefs[1] !== 'https://github.com/Yumin-Space/xexam/releases') bad.push('the version link points to ' + hrefs[1]);
  for (const a of links) {
    if (a.target !== '_blank' || !/noopener/.test(a.rel) || !/noreferrer/.test(a.rel)) bad.push('a footer link does not open safely in a new tab');
  }
  if (!/Xexam\s+\S*\d/.test(links[1]?.textContent || '')) bad.push('the version link has no version: ' + (links[1]?.textContent || ''));
  if (!window.__shown(line)) bad.push('the GitHub line is not visible');
  const lr = [...line.querySelectorAll('a, span')].map(e => e.getBoundingClientRect());
  const left = Math.min(...lr.map(r => r.left)), right = Math.max(...lr.map(r => r.right));
  const host = line.getBoundingClientRect();
  const off = Math.abs((left + right) / 2 - (host.left + host.right) / 2);
  if (off > 3) bad.push('the GitHub line is not centered (' + Math.round(off) + ' px off)');
  if (!root) {
    const note = document.querySelector('.foot-note');
    if (note && note.getBoundingClientRect().top > host.top) bad.push('the GitHub line is above the disclaimer');
    if (!note) notes.push('no .foot-note (disclaimer) on this page');
    for (const f of document.querySelectorAll('.foot')) {
      if (/Xexam\s+v?\d+\.\d+/.test(f.textContent.replace(line.textContent, ''))) bad.push('the old "Xexam v..." line is still in the footer');
    }
    const de = document.documentElement;
    if (host.bottom > de.scrollHeight + 1) bad.push('the line is outside the page');
  }
  if (touch) {
    const small = links.filter(a => Math.max(a.getBoundingClientRect().height, line.getBoundingClientRect().height) < 43.5);
    if (small.length) notes.push('WARNING: a footer link is under 44 px tall (' + Math.round(links[0].getBoundingClientRect().height) + ')');
  }
  return [bad, notes];
})"""

# The practice action bar at one phone width, as a user with (`expectAi`) or without the AI+ feature: ONE row, Check answer
# and Next both there as separate buttons in fixed places (nothing moves before / after the check or on the next question),
# Check greyed when there is nothing to check or it is checked, 44 px targets, no sideways overflow, and the AI controls
# bright (AI+ user) or greyed with the greyed AI+ bar in view (paused user). Returns the list of problems (empty = fine)
JS_BAR_CHECK = r"""(async (expectAi) => {
  const q = s => document.querySelector(s), wait = window.__wait;
  const bad = [];
  window.__skew = 0;
  await window.__start('practice');
  await wait(400);
  const slots = ['prev', 'check', 'next', 'flag'];
  const rect = n => {
    const b = q('.pfoot [data-slot="' + n + '"]');
    if (!b) return null;
    const r = b.getBoundingClientRect();
    return [Math.round(r.left), Math.round(r.top), Math.round(r.width), Math.round(r.height)];
  };
  const snap = () => Object.fromEntries(slots.map(n => [n, rect(n)]));
  const compare = (a, b, when) => {
    for (const n of slots) {
      if (!a[n] || !b[n]) bad.push('the ' + n + ' button is missing ' + when);
      else if (a[n].some((v, i) => Math.abs(v - b[n][i]) > 1)) bad.push('the ' + n + ' button moved ' + when + ': ' + a[n] + ' -> ' + b[n]);
    }
  };
  const greyed = b => b.disabled || b.getAttribute('aria-disabled') === 'true';
  const check = () => q('.pfoot [data-slot="check"]'), next = () => q('.pfoot [data-slot="next"]');
  if (!check() || !next()) return ['Check answer and Next are not both on the bar'];
  const btns = window.__qa('.pfoot button').filter(b => window.__shown(b));
  const tops = [];
  for (const b of btns) {
    const t = Math.round(b.getBoundingClientRect().top);
    if (!tops.some(x => Math.abs(x - t) < 6)) tops.push(t);
  }
  if (tops.length !== 1) bad.push('the bar has ' + tops.length + ' rows (one expected)');
  // 44 px targets: the visual box may be 40 px when its hit area (a pseudo-element) reaches 44 px: probe 21.5 px above and
  // below the centre
  const hit = b => {
    const r = b.getBoundingClientRect(), cx = r.left + r.width / 2, cy = r.top + r.height / 2;
    return [cy - 21.5, cy + 21.5].every(y => y < 0 || y > innerHeight || (document.elementFromPoint(cx, y) && b.contains(document.elementFromPoint(cx, y))));
  };
  const small = btns.filter(b => { const r = b.getBoundingClientRect(); return r.width < 39.5 || (r.height < 43.5 && !hit(b)); });
  if (small.length) bad.push(small.length + ' bar button(s) smaller than 44 px (' + small.map(b => b.dataset.slot + ' ' + Math.round(b.getBoundingClientRect().width) + 'x' + Math.round(b.getBoundingClientRect().height)).join(', ') + ')');
  const de = document.documentElement;
  if (de.scrollWidth > de.clientWidth + 1) bad.push('the page scrolls horizontally by ' + (de.scrollWidth - de.clientWidth) + ' px');
  const hint = q('[data-focus="q-aihint"]'), explain = q('[data-focus="q-aiexplain"]');
  if (expectAi) {
    if (!hint || !explain) bad.push('the AI hint / AI explanation buttons are missing');
    else if (greyed(hint) || greyed(explain)) bad.push('the AI buttons are greyed for an AI+ user');
  } else {
    if (!hint) bad.push('no AI hint button (greyed expected)');
    else if (!greyed(hint)) bad.push('the AI hint button is not greyed out');
    if (!window.__shown('[data-aip-dock-na]')) bad.push('the greyed AI+ bar is not visible');
  }
  if (!greyed(check())) bad.push('Check answer is not greyed before anything is chosen');
  if (greyed(next())) bad.push('Next is greyed on the first question');
  const s0 = snap();
  await window.__answer(false);
  await wait(400);
  if (!greyed(check())) { check().click(); await wait(500); }
  compare(s0, snap(), 'after the check');
  if (!greyed(check())) bad.push('Check answer is not greyed after the check');
  if (greyed(next())) bad.push('Next is greyed after the check');
  next().click();
  await wait(500);
  compare(s0, snap(), 'on the next question');
  if (!greyed(check())) bad.push('Check answer is not greyed on the next (unanswered) question');
  return bad;
})"""

# The exam bar (a phone): one row, a Next / Submit button, no Check button, no AI buttons. Returns the problems
JS_EXAM_BAR = r"""(() => {
  const bad = [];
  const btns = window.__qa('.pfoot button').filter(b => window.__shown(b));
  const tops = [];
  for (const b of btns) {
    const t = Math.round(b.getBoundingClientRect().top);
    if (!tops.some(x => Math.abs(x - t) < 6)) tops.push(t);
  }
  if (tops.length !== 1) bad.push('the exam bar has ' + tops.length + ' rows (one expected)');
  if (document.querySelector('.pfoot [data-slot="check"], .pfoot [data-act="check"]')) bad.push('the exam bar has a Check answer button');
  if (document.querySelector('.pfoot [data-slot="hint"], .pfoot [data-slot="explain"]')) bad.push('the exam bar has AI buttons');
  if (!document.querySelector('.pfoot [data-slot="next"]')) bad.push('the exam bar has no Next / Submit button');
  const de = document.documentElement;
  if (de.scrollWidth > de.clientWidth + 1) bad.push('the page scrolls horizontally');
  return bad;
})()"""

# AI settings: the category tabs under the status chips (FB-6). Returns [problems, the labels of the tabs, the label of the
# selected one]
JS_AI_TABS = r"""((expected) => {
  const bad = [];
  const tabs = window.__qa('button[aria-pressed][aria-controls]').filter(b => window.__shown(b));
  if (tabs.length !== 4) bad.push(tabs.length + ' category tabs (four expected)');
  const on = tabs.filter(b => b.getAttribute('aria-pressed') === 'true');
  if (on.length !== 1) bad.push(on.length + ' tabs selected (one expected)');
  const label = b => (b.getAttribute('aria-label') || b.innerText || '').replace(/\s+/g, ' ').trim();
  if (on[0] && expected != null && tabs.indexOf(on[0]) !== expected) bad.push('tab ' + (tabs.indexOf(on[0]) + 1) + ' is selected, tab ' + (expected + 1) + ' expected');
  const target = on[0] && document.getElementById(on[0].getAttribute('aria-controls'));
  if (on[0] && !target) bad.push('the selected tab controls nothing that exists');
  const de = document.documentElement;
  if (de.scrollWidth > de.clientWidth + 1) bad.push('the page scrolls horizontally');
  return [bad, tabs.map(label), on[0] ? label(on[0]) : ''];
})"""

# AI safety: the sub-groups of the limits card and the number of fields (FB-5: the answer / memory length is its own group
# between the request caps and the server protection). Returns [problems, the group titles]
JS_AI_SAFETY = r"""(() => {
  const bad = [];
  const groups = window.__qa('.ais-subgroup').map(g => (g.querySelector('.ais-h3')?.innerText || '').replace(/\s+/g, ' ').trim());
  if (groups.length < 3) bad.push(groups.length + ' limit sub-groups (the caps, the answer / memory length, the server protection expected)');
  if (!document.getElementById('ais-caps-h')) bad.push('no request caps group');
  if (!document.getElementById('ais-srv-h')) bad.push('no server protection group');
  return [bad, groups];
})()"""

# Study records at 360 px: every card is at most 140 px tall, no sideways overflow; returns the problems
JS_RECORDS_PHONE = r"""(() => {
  const bad = [];
  const rows = window.__qa('.rec-table tbody tr').filter(r => window.__shown(r));
  if (!rows.length) return ['no record cards'];
  const tall = rows.filter(r => r.getBoundingClientRect().height > 140);
  if (tall.length) bad.push(tall.length + ' record card(s) taller than 140 px (the tallest ' + Math.round(Math.max(...tall.map(r => r.getBoundingClientRect().height))) + ' px)');
  const de = document.documentElement;
  if (de.scrollWidth > de.clientWidth + 1) bad.push('the page scrolls horizontally');
  return bad;
})()"""

# The number of rows of buttons in the action bar of the practice page (buttons whose tops are within 6 px share a row)
JS_BAR_ROWS = r"""(() => {
  const tops = [];
  for (const el of document.querySelectorAll('.pfoot button')) {
    if (!el.offsetParent) continue;
    const t = Math.round(el.getBoundingClientRect().top);
    if (!tops.some(x => Math.abs(x - t) < 6)) tops.push(t);
  }
  return tops.length;
})()"""

# Result page of the exam: open the AI+ panel of the first answer card on its AI analysis (AI 解析), wait for the text and the
# Saved text; returns [text length or a message, the lowest bottom (page coordinates) of that card's panel and the collapsed
# bar of card 2 under it]
JS_AI_RESULT = r"""(async () => {
  const b = window.__qa('.rv .aip-ask').find(x => x.getAttribute('aria-disabled') !== 'true');
  if (!b) return ['NO AI ANALYSIS BUTTON (data-focus on the page: ' + window.__qa('[data-focus]').map(x => x.dataset.focus).slice(0, 25).join(' ') + ')', 0];
  b.click();
  const n = await window.__settle('#aip-panel-body');
  for (let i = 0; i < 40 && !window.__shown('#aip-panel .aip-saved'); i++) await window.__wait(150);
  const cards = window.__qa('.rv');
  const second = cards[1]?.querySelector('.aip-stub, .aip-dock, .aip-panel');
  const pop = document.querySelector('#aip-panel');
  const bottom = Math.max(cards[0].getBoundingClientRect().bottom, pop ? pop.getBoundingClientRect().bottom : 0,
    second ? second.getBoundingClientRect().bottom : 0) + window.scrollY;
  return [n, Math.ceil(bottom), second ? 'card 2 bar shown' : 'NO collapsed bar on card 2'];
})()"""

# Study records: open the AI summary window of the first record that has a usable summary button; returns the text length,
# the "based on" line and whether the Saved text is there
JS_AI_SUMMARY = r"""(async () => {
  const b = window.__qa('.rec-table .rec-ai button').find(x => x.getAttribute('aria-disabled') !== 'true');
  if (!b) return 'NO AI SUMMARY BUTTON';
  b.click();
  const n = await window.__settle('#aip-sum-body');
  for (let i = 0; i < 40 && !window.__shown('dialog.aip-sum .aip-sum-saved'); i++) await window.__wait(150);
  const meta = document.querySelector('#aip-sum-meta')?.innerText.replace(/\s+/g, ' ') || '';
  return (n < 0 ? 'SUMMARY DID NOT FINISH' : 'summary of ' + n + ' characters') + '; ' + (meta || 'NO "based on" LINE');
})()"""

# Study records at 800 px: the card layout (rows are grids), no horizontal scroll, every Delete button on one line;
# returns a list of the problems found (empty = fine)
JS_RECORDS_LAYOUT = r"""(() => {
  const bad = [];
  const table = document.querySelector('.rec-table');
  if (!table) return ['no records table'];
  if (!table.classList.contains('has-ai')) bad.push('the table has no has-ai class');
  const row = table.querySelector('tbody tr');
  if (!row || getComputedStyle(row).display !== 'grid') bad.push('the rows are not in the card layout');
  const de = document.documentElement;
  if (de.scrollWidth > de.clientWidth + 1) bad.push('the page scrolls horizontally');
  const sc = document.querySelector('.rec-scroll');
  if (sc && sc.scrollWidth > sc.clientWidth + 1) bad.push('the list scrolls horizontally');
  for (const el of document.querySelectorAll('.rec-del')) {
    if (!el.offsetParent) continue;
    const r = document.createRange();
    r.selectNodeContents(el);
    const tops = new Set([...r.getClientRects()].map(x => Math.round(x.top / 4)));
    if (tops.size > 1) { bad.push('a Delete button wraps onto two lines'); break; }
  }
  return bad;
})()"""


class Shooter:
    def __init__(self, cdp, base, out, only):
        self.cdp, self.base, self.out, self.only = cdp, base, out, only
        self.written = []

    def wanted(self, name):
        return not self.only or name in self.only

    def metrics(self, w, h, mobile):
        self.cdp.call('Emulation.setDeviceMetricsOverride', width=w, height=int(h), deviceScaleFactor=2, mobile=mobile,
                      screenWidth=w, screenHeight=int(h))

    def nav(self, route='#/', settle=1.6):
        # a URL that differs only in its hash would not reload the page (and not read the settings again)
        self.cdp.call('Page.navigate', url='about:blank')
        time.sleep(0.2)
        self.cdp.call('Page.navigate', url=self.base + '/' + route)
        time.sleep(settle)

    def js(self, code):
        return self.cdp.evaluate(code)

    def png(self, name, w, h, mobile, clip=None, scroll_y=None):
        """The w x h window as `name`; with `clip`, only its top `clip` px (for a page whose layout follows the window
        height, so the window cannot simply be made as tall as the image); with `scroll_y`, scrolled down that far."""
        self.metrics(w, h, mobile)
        time.sleep(0.5)
        if scroll_y is not None:
            self.js('window.scrollTo(0, %d); window.scrollY' % scroll_y)
            time.sleep(0.3)
        shot = {'format': 'png'}
        if clip:
            shot['clip'] = {'x': 0, 'y': 0, 'width': w, 'height': int(clip), 'scale': 1}
            h = int(clip)
        data = strip_png(base64.b64decode(self.cdp.call('Page.captureScreenshot', **shot)['data']))
        path = os.path.join(self.out, name)
        for attempt in range(8):                     # a viewer or a virus scanner may hold the file for a moment
            try:
                with open(path + '.tmp', 'wb') as fh:
                    fh.write(data)
                os.replace(path + '.tmp', path)
                break
            except OSError:
                time.sleep(0.5 * (attempt + 1))
        else:
            raise OSError('could not write ' + path)
        self.written.append(path)
        print('  %-26s %4d x %-4d %8d bytes' % (name, w * 2, int(h) * 2, len(data)))

    def desktop(self):
        self.cdp.call('Emulation.setTouchEmulationEnabled', enabled=False)
        self.cdp.call('Emulation.setUserAgentOverride', userAgent=DESKTOP_UA)
        self.metrics(WIDTH, 900, False)

    def iphone(self):
        self.cdp.call('Emulation.setTouchEmulationEnabled', enabled=True, maxTouchPoints=5)
        self.cdp.call('Emulation.setUserAgentOverride', userAgent=IPHONE_UA)
        self.metrics(PHONE_W, PHONE_H, True)

    def page(self, name, cap=1100, fixed=None, minimum=560):
        """A desktop image: `fixed` px tall, or as tall as the page up to `cap`, cut where nothing is split."""
        if not self.wanted(name):
            return
        if fixed is None:
            self.metrics(WIDTH, 900, False)
            time.sleep(0.3)
            fixed = self.js('(%s)(%d, %d)' % (FIT_JS, cap, minimum))
        self.png(name, WIDTH, fixed, False)

    def phone(self, name, scroll_to=None):
        if not self.wanted(name):
            return
        if scroll_to:
            self.js("document.querySelector(%s)?.scrollIntoView({block: 'start'}); window.scrollBy(0, -64); 'ok'"
                    % json.dumps(scroll_to))
        self.png(name, PHONE_W, PHONE_H, True)


def shoot_language(chrome_exe, base, cookie, lang, out, upload, only=(), with_log=True):
    """All scenes of one language into `out`. `upload`: the path of "Kitchen basics (v2).xlsx". Returns the files."""
    os.makedirs(out, exist_ok=True)
    code = UI_LANG[lang]
    before = seed.record_ids(base, cookie)
    with Chrome(chrome_exe, BROWSER_LANG[lang]) as chrome:
        cdp = chrome.page
        for domain in ('Page', 'Runtime', 'DOM', 'Network'):
            cdp.call(domain + '.enable')
        cdp.call('Page.addScriptToEvaluateOnNewDocument', source=PRELUDE)
        cdp.call('Emulation.setLocaleOverride', locale=BROWSER_LANG[lang])
        cdp.call('Emulation.setTimezoneOverride', timezoneId='UTC')
        cdp.call('Emulation.setEmulatedMedia', features=[{'name': 'prefers-color-scheme', 'value': 'light'}])
        s = Shooter(cdp, base, out, set(only))
        _desktop_scenes(s, base, cookie, code, upload, with_log)
        _ai_scenes(s, base, cookie, code)
        _phone_scenes(s, base, cookie, code)
        _check_practice_bar(s, base, cookie, code)
        seed.delete_ai_chats(base, cookie)
        # leave nothing behind for the next language: the saved exam and the exam record of this run
        s.nav()
        s.js(JS_CLEAR_SAVED)
    added = seed.record_ids(base, cookie) - before
    if added:
        seed.delete_records(base, cookie, added)
    return s.written


def _desktop_scenes(s, base, cookie, code, upload, with_log):
    cdp = s.cdp
    s.desktop()
    cdp.call('Network.clearBrowserCookies')
    s.nav(settle=2.4)
    s.page('desktop-9-signin.png', cap=900)
    cdp.call('Network.setCookie', name=seed.COOKIE, value=cookie, url=base + '/', httpOnly=True, sameSite='Strict')
    s.nav()
    print('  ' + s.js(JS_SETUP + "(%s, 'light')" % json.dumps(code)))
    s.js(JS_CLEAR_SAVED)
    s.nav()
    if s.wanted('desktop-3-result.png') or s.wanted('desktop-18-ai-result.png'):
        print('  exam:', s.js(JS_EXAM))
        # the score and the first answer card, cut just below that card
        bottom = s.js("(() => { const c = document.querySelector('.page-result .stem').closest('article, li, section, .panel'); "
                      "return Math.ceil(c.getBoundingClientRect().bottom + window.scrollY) })()")
        s.page('desktop-3-result.png', fixed=bottom + 9)
        if s.wanted('desktop-18-ai-result.png'):
            # the AI analysis window of that first card (the exam is saved, so the button is bright)
            s.metrics(WIDTH, 900, False)
            time.sleep(0.4)
            text, low, card2 = (s.js(JS_AI_RESULT) + [''])[:3]
            print('  ai result:', text if isinstance(text, str) else ('analysis of %d characters' % text if text > 0 else 'ANALYSIS DID NOT FINISH'),
                  '; ' + card2 if card2 else '')
            _study_check(s, '#aip-panel', '.aip-saved', 'AI+ panel on the result page')
            s.page('desktop-18-ai-result.png', fixed=min(1500, max(560, low + 24)))
    if s.wanted('desktop-4-records.png') or s.wanted('desktop-19-ai-summary.png'):
        s.nav('#/records')
        s.page('desktop-4-records.png', cap=1320)
        if s.wanted('desktop-4-records.png') and code in ('en', 'ja'):
            _check_records_layout(s, code)
        if s.wanted('desktop-19-ai-summary.png'):
            s.desktop()
            s.nav('#/records')
            print('  ai summary:', s.js(JS_AI_SUMMARY))
            _study_check(s, 'dialog.aip-sum', '.aip-sum-saved', 'AI summary window')
            s.png('desktop-19-ai-summary.png', WIDTH, 900, False)
    if s.wanted('desktop-17-ai-hint.png'):
        s.desktop()
        s.js(JS_CLEAR_SAVED)
        s.nav()
        print('  ai hint:', s.js('(%s)(false)' % JS_AI_HINT))
        _study_check(s, '#aip-panel', '.aip-saved', 'AI+ panel on the practice page')
        low = s.js("Math.ceil(Math.max(document.querySelector('.pfoot').getBoundingClientRect().bottom, "
                   "document.querySelector('#aip-panel')?.getBoundingClientRect().bottom || 0) + window.scrollY)")
        s.page('desktop-17-ai-hint.png', fixed=min(1500, max(560, low + 40)))
        s.js(JS_CLEAR_SAVED)
    if s.wanted('desktop-11-practice.png'):
        s.nav()
        print('  practice:', s.js(JS_PRACTICE))
        s.page('desktop-11-practice.png', cap=1250)
        s.js(JS_CLEAR_SAVED)
    s.nav()
    print('  quiz:', s.js(JS_QUIZ))
    s.page('desktop-2-quiz.png', cap=900)
    s.js(JS_SAVE_EXIT)
    if s.wanted('desktop-1-home.png'):
        _footer(s, 'Home')
    s.page('desktop-1-home.png', cap=1500)
    if s.wanted('desktop-5-banks.png'):
        s.nav('#/banks')
        print('  banks:', s.js(JS_UPDATE))
        root = cdp.call('DOM.getDocument', depth=-1)['root']['nodeId']
        node = cdp.call('DOM.querySelector', nodeId=root, selector='#bk-file')['nodeId']
        cdp.call('DOM.setFileInputFiles', files=[os.path.abspath(upload)], nodeId=node)
        time.sleep(1.5)
        print('  preview:', s.js("[...document.querySelectorAll('.bk-chip')].map(p => p.innerText.replace(/\\s+/g, ' ')).join(' | ')"))
        bottom, opened, hidden = s.js(JS_BANKS_GUIDE)
        print('  guide: %s, %d later part(s) hidden, columns end at %d px' % ('open' if opened else 'NOT OPEN', hidden,
                                                                           bottom))
        s.page('desktop-5-banks.png', fixed=bottom + 24)
        s.js("fetch('/api/import/cancel', {method: 'POST', headers: {'Content-Type': 'application/json'}, body: '{}'}).then(r => r.status)")
    s.nav('#/settings')
    s.page('desktop-6-settings.png', cap=1800)
    s.nav('#/users', settle=2.2)
    print('  users:', s.js(JS_USERS_STATE))
    s.page('desktop-10-users.png', cap=1100)
    if s.wanted('desktop-12-user-banks.png'):
        admin = seed.Session(base)
        admin.cookie[seed.COOKIE] = cookie
        alex = admin.user_id('alex')
        print('  banks dialog:', s.js("(async () => { document.querySelector('#us-banks-%d').click(); await window.__wait(900); "
                                      "return document.querySelector('dialog[open]') ? 'open' : 'no dialog' })()" % alex))
        # tall enough for the whole dialog with some backdrop above and below it
        need = s.js("Math.ceil(document.querySelector('dialog[open]').getBoundingClientRect().height)")
        s.page('desktop-12-user-banks.png', fixed=max(820, need + 120))
        s.js("document.querySelector('dialog[open]')?.close(); 'closed'")
    if with_log and s.wanted('desktop-13-logs.png'):
        s.nav('#/logs', settle=2.5)
        # the log box is min(64vh, 720px) tall: find the viewport height whose card ends 24 px above the bottom edge
        h = 1000
        for _ in range(30):
            s.metrics(WIDTH, h, False)
            time.sleep(0.3)
            bottom, lines = s.js(JS_LOG_CARD)
            if abs(bottom + 24 - h) <= 1:
                break
            h = bottom + 24
        print('  log: %d lines, card ends at %d px' % (lines, bottom))
        s.page('desktop-13-logs.png', fixed=h)
    # the dark theme: the exam page (the saved exam continued)
    s.js(JS_SETUP + "(%s, 'dark')" % json.dumps(code))
    s.nav()
    s.js(JS_RESUME)
    s.page('desktop-7-dark.png', cap=900)
    s.js(JS_SETUP + "(%s, 'light')" % json.dumps(code))
    s.nav('#/about', settle=2.4)
    bottom, kept, hidden = s.js('(%s)(%d)' % (JS_ABOUT, 1500))
    print('  about: %d bullet(s) shown, %d hidden, card ends at %d px' % (kept, hidden, bottom))
    s.page('desktop-8-about.png', fixed=bottom + 24)


def _check_records_layout(s, code):
    """At 800 px the Study records list is in the card layout with no horizontal scroll and Delete on one line (en, ja)."""
    s.metrics(800, 900, False)
    time.sleep(0.6)
    bad = s.js(JS_RECORDS_LAYOUT)
    s.metrics(WIDTH, 900, False)
    if bad:
        raise AssertionError('Study records at 800 px (%s): %s' % (code, '; '.join(bad)))
    print('  records at 800 px: card layout, no horizontal scroll, Delete on one line')


def _expect(problems, what):
    """Stops the run with a message when a layout check found problems."""
    if problems:
        raise AssertionError('%s: %s' % (what, '; '.join(problems)))


def _study_check(s, scope, saved, what):
    """The Saved text, no navigation and no focus frame in the AI+ panel / summary window that is open; then drops the
    focus (so the image shows no focus ring of a control either)."""
    bad, tip = s.js('(%s)(%s, %s)' % (JS_STUDY_CHECK, json.dumps(scope), json.dumps(saved)))
    _expect(bad, what)
    print('  %s: Saved text, no Open in AI Chat, no focus frame; tip: %s' % (what, tip or 'WARNING: no tip text found'))
    s.js("document.activeElement?.blur?.(); 'blurred'")


def _footer(s, what, root=None, touch=False):
    """The footer line of the page: centered GitHub / version links, after the disclaimer (FB-4)."""
    bad, notes = s.js('(%s)(%s, %s)' % (JS_FOOTER, json.dumps(root), 'true' if touch else 'false'))
    _expect(bad, 'footer (%s)' % what)
    print('  footer (%s): centered GitHub | Xexam <version> line, both links' % what)
    for note in notes:
        print('  ' + note)


def _tabs(s, index, what):
    bad, labels, selected = s.js('(%s)(%d)' % (JS_AI_TABS, index))
    _expect(bad, 'AI settings tabs (%s)' % what)
    print('  AI settings tabs: %s; selected: %s' % (' | '.join(labels), selected))


def _tab_image(s, name, last_heading, cap):
    """An AI settings tab as an image from the top of the page (status chips, tabs) through the card of `last_heading`."""
    if not s.wanted(name):
        return
    span = s.js('(%s)(%s, %s)' % (JS_AI_CARDS, json.dumps(last_heading), json.dumps(last_heading)))
    if span:
        s.page(name, fixed=min(cap, span[1] + 12))
    else:
        print('  warning: the card of %s was not found; %s is cut by the usual rule' % (last_heading, name))
        s.page(name, cap=cap)


def _ai_scenes(s, base, cookie, code):
    """desktop-14-ai-chat, desktop-15-ai-settings and desktop-16-ai-usage (after the System log scene, so the AI calls of
    one language do not change the log image of the next one more than necessary)."""
    # the AI asks of the scenes before are saved as chats of the administrator: start the AI Chat scenes without them
    seed.delete_ai_chats(base, cookie)
    if s.wanted('desktop-14-ai-chat.png'):
        first, second = AI_QUESTIONS[code]
        # two AI calls follow (the first question through the API, the second typed): wait until the caps of the
        # administrator have room for both and one more (a cap that is reached shows its callout), so the image never shows it
        print('  waited for the AI caps: %d s' % seed.wait_for_ai_room(base, cookie, 3))
        seed.ai_chat(base, cookie, first, code)
        print('  free mode on:', seed.set_free(base, cookie, True))   # the green tag shows while it is on
        s.nav('#/ai', settle=2.4)
        print('  ai chat:', s.js('(%s)(%s)' % (JS_AI_OPEN, json.dumps(first))))
        print('  composer:', s.js(JS_AI_FOCUS))
        s.cdp.call('Input.insertText', text=second)
        print('  ai answer:', s.js(JS_AI_SEND))
        # The conversation is a box as tall as the window allows (like a chat app; at most 900 px), with the page footer
        # always just below the window's bottom edge: make the window just tall enough for the whole conversation, then
        # keep only the part down to 24 px below the panes (the footer stays out; nothing in the box is cut)
        h = 900
        for _ in range(8):
            s.metrics(WIDTH, h, False)
            time.sleep(0.4)
            over = s.js(JS_AI_OVER)
            if over <= 0 or h >= 1400:
                break
            h = min(1400, h + over)
        bottom = s.js(JS_AI_PANES_BOTTOM)
        if over > 0:
            print('  warning: the conversation is %d px taller than its box; it is shown scrolled to its end' % over)
        s.png('desktop-14-ai-chat.png', WIDTH, h, False, clip=min(h, bottom) if bottom else None)
        print('  free mode off:', seed.set_free(base, cookie, False))
        print('  ai chats deleted:', seed.delete_ai_chats(base, cookie))
    if any(s.wanted(n) for n in ('desktop-15-ai-settings.png', 'desktop-16-ai-usage.png', 'desktop-20-ai-safety.png')):
        s.desktop()
        # the connection, the model and the test are on the first tab: press Refresh list and Test there
        s.nav('#/ai-settings?tab=feature', settle=2.6)
        _tabs(s, 0, 'AI+ function')
        print('  ai settings:', s.js(JS_AI_TEST))
        # desktop-15: the AI+ users tab (the page is read again: the users list and the usage include the calls so far)
        s.nav('#/ai-settings?tab=users', settle=2.6)
        _tabs(s, 1, 'AI+ users')
        print('  AI+ feature:', s.js(JS_AI_FUNCTIONS))
        _tab_image(s, 'desktop-15-ai-settings.png', 'ais-users-h', 1700)
        # desktop-16: the usage tab (with the Today column)
        s.nav('#/ai-settings?tab=usage', settle=2.6)
        _tabs(s, 2, 'AI usage')
        _tab_image(s, 'desktop-16-ai-usage.png', 'ais-usage-h', 1700)
        # desktop-20: the safety tab: chat retention and the limits (the caps, the answer / memory length, the server)
        s.nav('#/ai-settings?tab=safety', settle=2.6)
        _tabs(s, 3, 'AI safety')
        bad, groups = s.js(JS_AI_SAFETY)
        _expect(bad, 'AI safety tab')
        print('  AI safety groups: ' + ' | '.join(groups))
        _tab_image(s, 'desktop-20-ai-safety.png', 'ais-lim-h', 2100)


def _check_practice_bar(s, base, cookie, code):
    """The practice bar at 360, 390 and 412 px as the administrator (AI+ buttons bright), and at 390 and 360 px as alex
    (AI+ feature paused: greyed controls, the greyed AI+ bar in view): one row, Check answer and Next separate and in
    fixed places, 44 px targets, no sideways overflow. Signs the page back in as the administrator afterwards."""
    if not s.wanted('iphone-5-ai-hint.png'):
        return
    cdp = s.cdp
    s.iphone()

    def at(width, expect_ai, who):
        s.metrics(width, PHONE_H, True)
        s.nav()
        s.js(JS_CLEAR_SAVED)
        s.nav()
        _expect(s.js('(%s)(%s)' % (JS_BAR_CHECK, 'true' if expect_ai else 'false')),
                'practice bar at %d px, %s' % (width, who))
        print('  practice bar at %d px (%s): one row, Check answer and Next separate and fixed, 44 px targets, no overflow'
              % (width, who))
        s.js(JS_CLEAR_SAVED)

    for width in (360, 390, 412):
        at(width, True, 'AI+ user')
    alex = seed.Session(base).sign_in('alex', seed.DEMO_PASSWORD).cookie[seed.COOKIE]
    try:
        cdp.call('Network.setCookie', name=seed.COOKIE, value=alex, url=base + '/', httpOnly=True, sameSite='Strict')
        s.nav()
        s.js(JS_SETUP + "(%s, 'light')" % json.dumps(code))
        for width in (390, 360):
            at(width, False, 'AI+ feature paused')
    finally:
        cdp.call('Network.setCookie', name=seed.COOKIE, value=cookie, url=base + '/', httpOnly=True, sameSite='Strict')
        s.nav()


def _phone_scenes(s, base, cookie, code):
    s.iphone()
    s.nav()
    if s.wanted('iphone-1-home.png'):
        _footer(s, 'Home on a phone', touch=True)
    s.phone('iphone-1-home.png')
    s.js(JS_RESUME)
    if s.wanted('iphone-2-quiz.png'):
        _expect(s.js(JS_EXAM_BAR), 'the exam bar on a phone')
        print('  exam bar at 390 px: one row, Next / Submit, no Check answer, no AI buttons')
    s.phone('iphone-2-quiz.png')
    s.nav('#/records')
    s.js("(async () => { document.querySelector('.menu-btn').click(); await window.__wait(450); return 'menu' })()")
    s.phone('iphone-3-menu.png')
    s.nav('#/records')
    if s.wanted('iphone-4-records.png'):
        s.metrics(360, PHONE_H, True)
        time.sleep(0.6)
        _expect(s.js(JS_RECORDS_PHONE), 'Study records at 360 px')
        print('  Study records at 360 px: cards at most 140 px tall, no horizontal scroll')
        s.iphone()
        time.sleep(0.4)
    s.phone('iphone-4-records.png')
    if s.wanted('iphone-5-ai-hint.png'):
        s.js(JS_CLEAR_SAVED)
        s.nav()
        print('  ai hint (phone):', s.js('(%s)(true)' % JS_AI_HINT))
        _study_check(s, '#aip-panel', '.aip-saved', 'AI+ panel on a phone')
        rows = s.js(JS_BAR_ROWS)
        if rows != 1:
            raise AssertionError('the practice action bar at 390 px has %d rows of buttons (one expected)' % rows)
        print('  practice action bar at 390 px: %d row' % rows)
        s.png('iphone-5-ai-hint.png', PHONE_W, PHONE_H, True)
        s.js(JS_CLEAR_SAVED)
    if s.wanted('iphone-6-result.png'):
        s.nav()
        print('  exam (phone):', s.js(JS_EXAM))
        print('  ai+ bars on the result (phone):', s.js("window.__qa('.rv .aip-ask').length + ' card(s) with an AI+ bar'"))
        s.phone('iphone-6-result.png')
        s.js(JS_CLEAR_SAVED)
    if s.wanted('iphone-7-ai-chat.png'):
        first = AI_QUESTIONS[code][0]
        seed.delete_ai_chats(base, cookie)
        seed.ai_chat(base, cookie, first, code)
        s.nav('#/ai', settle=2.4)
        if s.js("window.__shown('.ai-list')"):
            _footer(s, 'AI Chat list pane', root='.ai-list', touch=True)
        else:
            print('  footer (AI Chat list pane): the list is not shown on this screen; not checked')
        print('  ai chat (phone):', s.js('(%s)(%s)' % (JS_AI_OPEN, json.dumps(first))))
        problems = s.js("(() => { const bad = []; if (document.querySelector('.ai-conv .gh-line')) bad.push('the GitHub line is inside the conversation'); "
                        "const de = document.documentElement; if (de.scrollWidth > de.clientWidth + 1) bad.push('the page scrolls horizontally'); return bad; })()")
        _expect(problems, 'AI Chat on a phone')
        s.js("document.activeElement?.blur?.(); window.scrollTo(0, 0); 'top'")
        s.phone('iphone-7-ai-chat.png')
        print('  ai chats deleted:', seed.delete_ai_chats(base, cookie))
