'use strict';
/* Xexam —— 前端。所有来自 Excel 的文字都用 textContent 写入页面，不拼 HTML。 */

const LS_KEY = 'xexam.session.v1';
const LS_LANG = 'xexam.lang.v1';
const MODE_NAME = { all: '全部题目', weak: '错题本', retry: '重做错题' };
const SVGNS = 'http://www.w3.org/2000/svg';

const S = {
  lang: (() => { try { return localStorage.getItem(LS_LANG) === 'zh' ? 'zh' : 'ja'; } catch (e) { return 'ja'; } })(),   // 答题时题目显示的语言
  view: 'loading',          // loading | fatal | home | admin | quiz | result
  info: { version: '', canShutdown: false },
  admin: { preview: null, busy: false, imports: [], done: null, error: '' },
  bank: null,
  progress: null,
  bankSig: '',
  cfg: { mode: 'all', selected: null, known: [], count: null, shuffle: true },
  session: null,
  result: null,
  rvFilter: 'wrong',
  timer: null,
  refresh: [],
};

/* ---------------------------------------------------------------- 小工具 */

function h(tag, props, ...kids) {
  const el = document.createElement(tag);
  for (const [k, v] of Object.entries(props || {})) {
    if (v == null || v === false) continue;
    if (k === 'class') el.className = v;
    else if (k.startsWith('on') && typeof v === 'function') el.addEventListener(k.slice(2), v);
    else if (['checked', 'disabled', 'value', 'open'].includes(k)) el[k] = v;
    else el.setAttribute(k, v === true ? '' : v);
  }
  for (const kid of kids.flat(Infinity)) {
    if (kid == null || kid === false) continue;
    el.append(kid instanceof Node ? kid : document.createTextNode(String(kid)));
  }
  return el;
}

const paras = (text, extra, lang) =>
  String(text || '').split('\n').filter((l) => l.trim()).map((l) => h('p', { lang: lang || 'ja', class: extra }, l.trim()));

function fmtTime(sec) {
  sec = Math.max(0, Math.floor(sec));
  const hh = Math.floor(sec / 3600), mm = Math.floor((sec % 3600) / 60), ss = sec % 60;
  const p = (n) => String(n).padStart(2, '0');
  return hh ? `${hh}:${p(mm)}:${p(ss)}` : `${mm}:${p(ss)}`;
}

function shuffle(arr) {
  const a = arr.slice();
  for (let i = a.length - 1; i > 0; i--) {
    const j = Math.floor(Math.random() * (i + 1));
    [a[i], a[j]] = [a[j], a[i]];
  }
  return a;
}

let toastTimer;
function toast(msg, ms = 3400) {
  const t = document.getElementById('toast');
  t.textContent = msg;
  t.classList.add('show');
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => t.classList.remove('show'), ms);
}

// 和服务端（Python 的 round）一致：恰好 .5 时取偶数，这样成绩页和学习记录里的百分比不会差 1
function roundHalfEven(x) {
  const f = Math.floor(x), d = x - f;
  if (Math.abs(d - 0.5) < 1e-9) return f % 2 === 0 ? f : f + 1;
  return Math.round(x);
}

function ask({ title, body, ok = '确定', cancel = '取消', danger = false }) {
  return new Promise((resolve) => {
    const d = h('dialog', null,
      h('h3', null, title),
      h('p', null, body),
      h('div', { class: 'acts' },
        h('button', { class: 'btn', type: 'button', onclick: () => d.close('cancel') }, cancel),
        h('button', { class: 'btn ' + (danger ? 'danger' : 'primary'), type: 'button', onclick: () => d.close('ok') }, ok)));
    if (typeof d.showModal !== 'function') { resolve(window.confirm(title + '\n' + body)); return; }
    d.addEventListener('close', () => { d.remove(); resolve(d.returnValue === 'ok'); });
    document.body.append(d);
    d.showModal();
  });
}

async function post(path, body) {
  const r = await fetch(path, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(body || {}) });
  if (!r.ok) throw new Error('HTTP ' + r.status + ': ' + (await r.text().catch(() => '')));
  return r.json();
}

function svgMark(ok) {
  const s = document.createElementNS(SVGNS, 'svg');
  s.setAttribute('viewBox', '0 0 20 20');
  s.setAttribute('fill', 'none');
  s.setAttribute('stroke-width', '2.4');
  s.setAttribute('stroke-linecap', 'round');
  s.setAttribute('stroke-linejoin', 'round');
  s.setAttribute('aria-hidden', 'true');
  const p = document.createElementNS(SVGNS, 'path');
  p.setAttribute('d', ok ? 'M4 10.5l4 4 8-9' : 'M5 5l10 10M15 5L5 15');
  s.append(p);
  return s;
}

/* ---------------------------------------------------------------- 数据 */

const qStat = (id) => (S.progress && S.progress.questions && S.progress.questions[id]) || null;
const masterStreak = () => (S.progress && S.progress.masterStreak) || 2;
function inNotebook(id) {
  const r = qStat(id);
  return !!r && r.wrong > 0 && r.streak < masterStreak();
}

const sigOf = (b) => JSON.stringify([b.error, b.sheets.map((s) => [s.name, s.usable, s.skipped.length]), b.questions.map((q) => q.id)]);

function syncSelection() {
  const names = S.bank.sheets.filter((s) => s.usable > 0).map((s) => s.name);
  const c = S.cfg;
  if (!c.selected) c.selected = new Set(names);
  else {
    for (const n of names) if (!c.known.includes(n)) c.selected.add(n);
    for (const n of [...c.selected]) if (!names.includes(n)) c.selected.delete(n);
  }
  c.known = names;
}

function pool() {
  const sel = S.cfg.selected;
  return S.bank.questions.filter((q) => sel.has(q.sheet) && (S.cfg.mode !== 'weak' || inNotebook(q.id)));
}

function setBank(b) {
  S.bank = b;
  S.bankSig = sigOf(b);
  syncSelection();
}

/* ---------------------------------------------------------------- 作答与评分 */

function stateOf(q, a) {      // 0 未答  1 部分  2 已答完
  if (!Array.isArray(a)) return 0;
  if (q.kind === 'hotspot') {
    const c = a.filter((x) => x != null).length;
    return c === 0 ? 0 : c === q.items.length ? 2 : 1;
  }
  return a.length ? 2 : 0;
}

function grade(q, a) {
  if (!Array.isArray(a)) return false;
  if (q.kind === 'hotspot') return a.length === q.answer.length && q.answer.every((x, i) => a[i] === x);
  const mine = [...a].sort(), right = [...q.answer].sort();
  return mine.length === right.length && mine.every((x, i) => x === right[i]);
}

const kindLabel = (q) => (q.kind === 'hotspot' ? '热点题' : q.kind === 'multi' ? '多选' : '单选');
const elapsed = (se) => se.base + (Date.now() - se.t0) / 1000;

/* ---------------------------------------------------------------- 暂存 / 恢复 */

function saveSession() {
  const se = S.session;
  if (!se) return;
  try {
    localStorage.setItem(LS_KEY, JSON.stringify({
      v: 1, ids: se.qs.map((q) => q.id), answers: se.answers, flags: [...se.flags],
      idx: se.idx, elapsed: Math.floor(elapsed(se)), mode: se.mode, sheets: se.sheets,
    }));
  } catch (e) { /* 存不了就算了 */ }
}

const clearSaved = () => { try { localStorage.removeItem(LS_KEY); } catch (e) { /* ignore */ } };

function loadSaved() {
  try {
    const d = JSON.parse(localStorage.getItem(LS_KEY) || 'null');
    if (!d || !Array.isArray(d.ids)) return null;
    const byId = new Map(S.bank.questions.map((q) => [q.id, q]));
    const qs = d.ids.map((id) => byId.get(id)).filter(Boolean);
    if (!qs.length) return null;
    const answers = {};
    for (const q of qs) {
      const a = d.answers && d.answers[q.id];
      if (!Array.isArray(a)) continue;
      if (q.kind === 'hotspot') {
        const norm = q.items.map((it, i) => (Number.isInteger(a[i]) && a[i] >= 0 && a[i] < it.choices.length ? a[i] : null));
        if (norm.some((x) => x != null)) answers[q.id] = norm;
      } else {
        const keys = a.filter((k) => q.options.some((o) => o.key === k));
        if (keys.length) answers[q.id] = keys;
      }
    }
    const keep = new Set(qs.map((q) => q.id));
    const answered = qs.filter((q) => stateOf(q, answers[q.id]) === 2).length;
    return {
      qs, answers, answered, flags: new Set((d.flags || []).filter((id) => keep.has(id))),
      idx: Math.min(Math.max(0, d.idx | 0), qs.length - 1), elapsed: Math.max(0, d.elapsed | 0),
      mode: ['all', 'weak', 'retry'].includes(d.mode) ? d.mode : 'all', sheets: Array.isArray(d.sheets) ? d.sheets : [],
      dropped: d.ids.length - qs.length,
    };
  } catch (e) { return null; }
}

/* ---------------------------------------------------------------- 开始 / 退出 / 交卷 */

function startSession(qs, mode) {
  S.session = {
    qs, answers: {}, flags: new Set(), idx: 0, base: 0, t0: Date.now(), mode,
    sheets: [...new Set(qs.map((q) => q.sheet))],
  };
  S.view = 'quiz';
  saveSession();
  render();
  window.scrollTo(0, 0);
}

function resumeSession() {
  const sv = loadSaved();
  if (!sv) { render(); return; }
  S.session = { qs: sv.qs, answers: sv.answers, flags: sv.flags, idx: sv.idx, base: sv.elapsed, t0: Date.now(), mode: sv.mode, sheets: sv.sheets };
  S.view = 'quiz';
  if (sv.dropped) toast(`有 ${sv.dropped} 道题已经不在题库里，已跳过`);
  render();
  window.scrollTo(0, 0);
}

function leaveQuiz() {
  saveSession();
  S.session = null;
  S.view = 'home';
  render();
  window.scrollTo(0, 0);
}

async function submit() {
  const se = S.session;
  const open = se.qs.filter((q) => stateOf(q, se.answers[q.id]) < 2).length;
  const go = await ask(open
    ? { title: `还有 ${open} 道题没答完`, body: '没答完的题按答错计分。现在交卷吗？', ok: '交卷', cancel: '回去继续' }
    : { title: '确认交卷？', body: '交卷后才会计分，并显示答案和解析。', ok: '交卷', cancel: '再检查一下' });
  if (!go) return;

  const results = se.qs.map((q) => {
    const ans = Array.isArray(se.answers[q.id]) ? se.answers[q.id] : null;
    return { q, ans, correct: grade(q, ans) };
  });
  const seconds = Math.floor(elapsed(se));
  let prev = null;
  try {
    S.progress = await post('/api/attempt', {
      results: results.map((r) => ({ id: r.q.id, correct: r.correct })), sheets: se.sheets, mode: se.mode, seconds,
    });
    clearSaved();
    const hist = S.progress.history || [];
    if (hist[1] && hist[1].mode === se.mode) prev = hist[1];
  } catch (e) {
    toast('成绩没能保存到学习记录（服务没连上，或登录已过期）。这一场的答案还留在浏览器里：刷新页面重新登录后，在首页「继续作答」里再交一次卷', 7000);
  }
  S.session = null;
  S.result = { results, seconds, mode: se.mode, prev };
  S.rvFilter = 'wrong';
  S.view = 'result';
  render();
  window.scrollTo(0, 0);
}

/* ---------------------------------------------------------------- 首页 */

function renderHome() {
  const b = S.bank, cfg = S.cfg;
  const saved = loadSaved();
  const nb = b.questions.filter((q) => inNotebook(q.id));
  if (cfg.mode === 'weak' && !nb.length) cfg.mode = 'all';

  const countInput = h('input', { type: 'number', min: '1', step: '1', inputmode: 'numeric', 'aria-label': '题数' });
  const shuffleBox = h('input', { type: 'checkbox', checked: cfg.shuffle });
  const startBtn = h('button', { class: 'btn primary big', type: 'button' }, '开始考试');
  const poolText = h('span', { class: 'dim' });

  const refreshSetup = () => {
    const n = pool().length;
    countInput.max = String(n);
    countInput.disabled = n === 0;
    countInput.value = n ? String(cfg.count == null ? n : Math.max(1, Math.min(cfg.count, n))) : '';
    startBtn.disabled = n === 0;
    poolText.textContent = n ? `当前范围共 ${n} 题` : (cfg.mode === 'weak' ? '所选工作表里没有待巩固的错题' : '先勾选一个工作表');
  };
  countInput.addEventListener('change', () => {
    const n = pool().length, v = parseInt(countInput.value, 10);
    cfg.count = !v || v >= n ? null : Math.max(1, v);
    refreshSetup();
  });
  startBtn.addEventListener('click', async () => {
    const p = pool();
    if (!p.length) return;
    if (loadSaved() && !(await ask({ title: '放弃没交卷的考试？', body: '有一场暂存的考试还没交卷。现在开始新的考试，会把它覆盖掉。', ok: '开始新考试', cancel: '回去继续', danger: true }))) return;
    let n = parseInt(countInput.value, 10);
    if (!(n >= 1)) n = p.length;
    n = Math.min(n, p.length);
    cfg.shuffle = shuffleBox.checked;
    startSession((cfg.shuffle ? shuffle(p) : p).slice(0, n), cfg.mode);
  });

  const sheetRows = b.sheets.map((s) => {
    const usable = s.usable > 0;
    const wrongN = b.questions.filter((q) => q.sheet === s.name && inNotebook(q.id)).length;
    const cb = h('input', {
      type: 'checkbox', checked: usable && cfg.selected.has(s.name), disabled: !usable,
      onchange: () => { if (cb.checked) cfg.selected.add(s.name); else cfg.selected.delete(s.name); refreshSetup(); },
    });
    const sub = usable
      ? `${s.usable} 题可用` + (s.inactive ? `，${s.inactive} 题已下架` : '')
      : '这个工作表里的题都已下架';
    return h('label', { class: 'row-opt' + (usable ? '' : ' off') }, cb,
      h('span', { class: 'grow' }, h('div', { class: 'name' }, s.name), h('div', { class: 'sub' }, sub)),
      wrongN ? h('span', { class: 'chipnum' }, `错题 ${wrongN}`) : null);
  });

  const modeRow = (val, title, sub, disabled) => {
    const r = h('input', {
      type: 'radio', name: 'mode', checked: cfg.mode === val, disabled,
      onchange: () => { cfg.mode = val; refreshSetup(); },
    });
    return h('label', { class: 'row-opt' + (disabled ? ' off' : '') }, r,
      h('span', { class: 'grow' }, h('div', { class: 'name' }, title), h('div', { class: 'sub' }, sub)));
  };

  const setup = h('section', { class: 'panel' },
    h('h2', null, '开始考试'),
    saved ? h('div', { class: 'resume' },
      h('p', null, `有一场没交卷的考试：已答 ${saved.answered} / ${saved.qs.length} 题，用时 ${fmtTime(saved.elapsed)}。`),
      h('div', { class: 'acts' },
        h('button', { class: 'btn primary', type: 'button', onclick: resumeSession }, '继续作答'),
        h('button', {
          class: 'btn', type: 'button',
          onclick: async () => { if (await ask({ title: '放弃这场考试？', body: '已经作答的内容不会保留。', ok: '放弃', cancel: '先留着', danger: true })) { clearSaved(); render(); } },
        }, '放弃这场'))) : null,
    b.error ? h('div', { class: 'err' }, h('p', null, b.error)) : null,
    b.questions.length ? [
      h('fieldset', null, h('legend', null, '考哪些工作表'), sheetRows),
      h('fieldset', null, h('legend', null, '从哪里出题'),
        modeRow('all', '全部题目', '按所选工作表出题', false),
        modeRow('weak', '错题本', nb.length ? `${nb.length} 题待巩固，连续答对 ${masterStreak()} 次后移出` : '还没有错题，先考一轮', !nb.length)),
      h('div', { class: 'inline' },
        h('label', null, '题数', countInput),
        h('label', null, shuffleBox, '打乱题目顺序')),
      h('div', { class: 'inline' }, startBtn, poolText),
    ] : (b.error ? null : h('div', { class: 'emptybank' },
      h('p', null, '题库还是空的。先上传你的 Excel 题库，再开始考试。'),
      h('button', { class: 'btn primary', type: 'button', onclick: openAdmin }, '去上传题库'))));

  refreshSetup();

  const inactive = b.sheets.reduce((n, s) => n + (s.inactive || 0), 0);
  const bankInfo = h('div', { class: 'bankinfo' },
    b.file ? h('span', null, b.file) : null,
    b.mtime ? h('span', { class: 'num' }, '更新于 ' + b.mtime) : null,
    h('span', null, `${b.questions.length} 题，${b.sheets.filter((s) => s.usable).length} 个工作表` + (inactive ? `（另有 ${inactive} 题已下架）` : '')),
    h('button', { class: 'btn', type: 'button', onclick: openAdmin }, '题库管理 / 上传 Excel'));

  return h('div', { class: 'wrap' },
    h('header', { class: 'top' }, brand(), bankInfo),
    h('div', { class: 'home-grid' }, setup, historyPanel()),
    h('details', { class: 'tr', style: 'margin-top:20px' },
      h('summary', null, '题库怎么写（往 Excel 加新题时看）'),
      h('div', { class: 'pre' },
        '每个工作表就是一组题，可以随时新增工作表。改好 Excel 后，点上面的「题库管理 / 上传 Excel」，把整份文件传上去。\n' +
        'A 列：题目。选择题把选项写成「A. 内容」，一行一个；热点题第一行写 ホットスポット，小项写成「1. 内容　はい / いいえ」。\n' +
        'B 列：中文翻译（可以不写）。\n' +
        'C 列：先写「【答案】C、E」（热点题写「1 是；2 否；…」），再写「【解析·怎么选】」和「【记忆口诀】」。\n' +
        '上传后会先显示「新增几题、更新几题、下架几题、哪几题读不出来」，你确认了才会真正导入。')),
    h('footer', { class: 'foot' },
      S.info.version ? h('span', { class: 'num' }, 'Xexam v' + S.info.version) : null,
      h('span', null, '题库里的答案是根据题意推理的，不是官方标准答案；把握度较低的题，会在答题后的备注里写明。'),
      S.info.canShutdown ? h('button', { class: 'btn quiet', type: 'button', onclick: quitProgram }, '关闭程序') : null));
}

function brand() {
  return h('div', { class: 'brand' },
    h('span', { class: 'logo', 'aria-hidden': 'true' }, h('i', { class: 'on' }), h('i'), h('i', { class: 'on' }), h('i')),
    h('h1', null, 'Xexam'));
}

function historyPanel() {
  const hist = (S.progress && S.progress.history) || [];
  const panel = h('section', { class: 'panel' }, h('h2', null, '学习记录'));
  if (!hist.length) {
    panel.append(h('p', { class: 'empty' }, '还没有考试记录。考完一场，成绩会出现在这里。'));
    return panel;
  }
  const best = Math.max(...hist.map((x) => x.percent));
  const recent = hist.slice(0, 12).reverse();
  panel.append(
    h('div', { class: 'stat' }, h('b', { class: 'num' }, hist[0].percent + '%'), h('span', { class: 'dim' }, '最近一场')),
    h('p', { class: 'dim' }, `一共考过 ${hist.length} 场，最高 ${best}%`),
    h('div', { class: 'bars', role: 'img', 'aria-label': '最近成绩：' + recent.map((x) => x.percent + '%').join('，') },
      recent.map((x) => h('i', { style: `height:${Math.max(x.percent, 3)}%`, title: `${x.at}  ${x.percent}%` }))),
    h('ul', { class: 'hist' }, hist.slice(0, 6).map((x) => h('li', null,
      h('span', { class: 'pct' }, x.percent + '%'),
      h('span', { class: 'meta' }, `${x.at}，${MODE_NAME[x.mode] || ''}，答对 ${x.correct} / ${x.total} 题，用时 ${fmtTime(x.seconds)}`,
        x.sheets && x.sheets.length ? h('div', { class: 'dim' }, x.sheets.join('、')) : null)))),
    h('div', { style: 'margin-top:12px' },
      h('button', {
        class: 'btn quiet danger', type: 'button',
        onclick: async () => {
          if (!(await ask({ title: '清空学习记录？', body: '成绩历史和错题本都会清掉，不能恢复。', ok: '清空', cancel: '不清空', danger: true }))) return;
          try { S.progress = await post('/api/reset'); render(); toast('学习记录已清空'); } catch (e) { toast('没能清空，本地服务没连上'); }
        },
      }, '清空学习记录')));
  return panel;
}

async function quitProgram() {
  if (!(await ask({ title: '关闭程序？', body: '关闭后这个页面就用不了了，下次要重新启动。学习记录已经保存好了。', ok: '关闭', cancel: '不关' }))) return;
  try { await post('/api/shutdown'); } catch (e) { /* 服务端关得快时会断开 */ }
  document.getElementById('app').replaceChildren(h('div', { class: 'wrap' }, h('div', { class: 'panel' }, h('h2', null, '程序已关闭'), h('p', null, '可以关掉这个页面了。'))));
}

/* ---------------------------------------------------------------- 题库管理（上传 Excel） */

async function openAdmin() {
  S.admin = { preview: null, busy: false, imports: [], done: null, error: '' };
  S.view = 'admin';
  render();
  window.scrollTo(0, 0);
  try { S.admin.imports = (await fetch('/api/imports').then((r) => r.json())).imports || []; } catch (e) { /* 没有也行 */ }
  if (S.view === 'admin') render();
}

function closeAdmin() {
  if (S.admin.preview) post('/api/import/cancel').catch(() => {});
  S.admin = { preview: null, busy: false, imports: [], done: null, error: '' };
  S.view = 'home';
  render();
  window.scrollTo(0, 0);
}

async function uploadFile(file) {
  const A = S.admin;
  if (!file) return;
  A.error = ''; A.done = null; A.preview = null;
  if (!/\.xlsx$/i.test(file.name)) {
    A.error = `「${file.name}」不是 .xlsx 文件。请在 Excel 里另存为「Excel 工作簿(.xlsx)」再上传。`;
    render();
    return;
  }
  A.busy = true; render();
  try {
    const r = await fetch('/api/upload', {
      method: 'POST', body: file,
      headers: { 'Content-Type': 'application/octet-stream', 'X-Filename': encodeURIComponent(file.name) },
    });
    const isJson = /json/i.test(r.headers.get('Content-Type') || '');
    const d = isJson ? await r.json().catch(() => ({})) : {};
    if (!r.ok) A.error = d.error || `上传失败（HTTP ${r.status}）。${isJson ? '' : '如果你是通过域名访问的，可能是登录已过期，请刷新页面重新登录。'}`;
    else if (!isJson || !d.token) A.error = '服务返回的不是预期的内容。如果你是通过域名访问的，可能是登录已过期，请刷新页面重新登录后再上传。';
    else A.preview = d;
  } catch (e) {
    A.error = '上传失败：连不上服务。请确认程序（或 Docker 容器）还在运行；如果你是通过域名访问的，也可能是登录已过期，请刷新页面重新登录。';
  }
  A.busy = false;
  if (S.view === 'admin') { render(); window.scrollTo(0, 0); }
}

async function confirmImport() {
  const A = S.admin, pv = A.preview;
  if (!pv) return;
  if (pv.removed > 0) {
    const ok = await ask({
      title: `这次会下架 ${pv.removed} 道题`,
      body: `这份文件里没有它们，所以会从题库里下架（不会再出现在考试里）。` +
            (pv.removed_stats ? `其中 ${pv.removed_stats} 道有答题记录，记录会保留，以后你把题传回来会自动恢复。` : '') +
            `\n确定要继续吗？如果传错了文件，请选「回去检查」。`,
      ok: '继续导入', cancel: '回去检查', danger: true,
    });
    if (!ok) return;
  }
  A.busy = true; render();
  try {
    const d = await post('/api/import/confirm', { token: pv.token });
    setBank(d.bank);
    A.done = { filename: pv.filename, ...d.result, total: d.bank.questions.length };
    A.preview = null;
    A.imports = (await fetch('/api/imports').then((r) => r.json()).catch(() => ({ imports: A.imports }))).imports;
  } catch (e) {
    let msg = '导入失败。如果你是通过域名访问的，可能是登录已过期：请刷新页面重新登录，再重新上传。（如果刚才网络断了，导入可能已经完成，请看看题库里的题数。）';
    try { const j = JSON.parse(String(e.message).replace(/^HTTP \d+:\s*/, '')); if (j.error) msg = j.error; } catch (x) { /* 用默认 */ }
    A.error = msg;
    A.preview = null;
  }
  A.busy = false;
  if (S.view === 'admin') { render(); window.scrollTo(0, 0); }
}

function pill(n, label, cls) {
  return h('div', { class: 'pill ' + (cls || '') + (n ? '' : ' zero') }, h('b', { class: 'num' }, String(n)), h('span', null, label));
}

function previewView(pv) {
  const A = S.admin;
  const nothing = pv.usable === 0;
  const same = !nothing && !pv.added && !pv.updated && !pv.removed && !pv.restored;
  return h('section', { class: 'panel' },
    h('h2', null, `检查结果：${pv.filename}`),
    nothing ? h('div', { class: 'err' }, h('p', null, '这份文件里一道可用的题都没读到，不能导入。' + (pv.skipped ? '看下面每个工作表里「没能读入」的原因。' : '请确认题目写在 A 列，答案写在 C 列。')))
            : h('p', { class: 'dim', style: 'margin-bottom:14px' }, '下面是导入后题库会发生的变化。现在还没有写入，确认后才会生效。'),
    h('div', { class: 'pills' },
      pill(pv.added, '新增', 'g'), pill(pv.updated, '更新', 'b'), pill(pv.restored, '恢复上架', 'b'),
      pill(pv.removed, '下架', pv.removed ? 'r' : ''), pill(pv.unchanged, '没变化'), pill(pv.skipped, '读不出来', pv.skipped ? 'r' : '')),
    pv.removed ? h('div', { class: 'warnbox' },
      h('p', null, `有 ${pv.removed} 道现有的题在这份文件里找不到，导入后会下架。` +
        (pv.removed_stats ? `其中 ${pv.removed_stats} 道有答题记录，记录会保留。` : '')),
      pv.removed_samples && pv.removed_samples.length ? h('ul', null, pv.removed_samples.map((t) => h('li', { lang: 'ja' }, t + '…'))) : null,
      h('p', { class: 'dim' }, '如果你只是想新增题，请上传包含全部题目（旧的加新的）的完整 Excel。')) : null,
    same ? h('p', { class: 'dim', style: 'margin-top:12px' }, '这份文件和现在的题库完全一样，不需要导入。') : null,
    h('h3', { class: 'sub-h' }, '各工作表'),
    h('div', { class: 'sheetlist' }, pv.sheets.map((s) => h('div', { class: 'sheetrow' },
      h('div', { class: 'name' }, s.name),
      h('div', { class: 'dim' },
        `${s.usable} 题可用` +
        (s.added ? `，新增 ${s.added}` : '') + (s.updated ? `，更新 ${s.updated}` : '') +
        (s.restored ? `，恢复 ${s.restored}` : '') + (s.removed ? `，下架 ${s.removed}` : '') +
        ((s.skippedCount || 0) ? `，${s.skippedCount} 题读不出来` : '')),
      (s.skippedCount || 0) ? h('details', { class: 'skipbox' },
        h('summary', null, `${s.skippedCount} 题没能读入，点开看原因`),
        h('ul', null, s.skipped.map((k) => h('li', null, `第 ${k.row} 行：${k.reason}`)),
          s.skippedMore ? h('li', { class: 'dim' }, `……还有 ${s.skippedMore} 条类似的`) : null)) : null))),
    h('div', { class: 'acts', style: 'margin-top:20px' },
      h('button', { class: 'btn primary', type: 'button', disabled: nothing || same || A.busy, onclick: confirmImport }, nothing ? '没有可导入的题' : same ? '无需导入' : '确认导入'),
      h('button', { class: 'btn', type: 'button', disabled: A.busy, onclick: async () => { try { await post('/api/import/cancel'); } catch (e) { /* 过期了也没关系 */ } A.preview = null; render(); } }, '取消，不导入')));
}

function renderAdmin() {
  const A = S.admin, b = S.bank;
  const input = h('input', { type: 'file', accept: '.xlsx,application/vnd.openxmlformats-officedocument.spreadsheetml.sheet', id: 'xlsx-input', hidden: true, onchange: (e) => { const f = e.target.files[0]; e.target.value = ''; uploadFile(f); } });
  const drop = h('div', { class: 'drop' + (A.busy ? ' busy' : ''), role: 'group', 'aria-label': '上传 Excel' },
    h('p', { class: 'drop-t' }, A.busy ? '正在读取和检查…' : '选择你更新后的 Excel 文件'),
    h('p', { class: 'dim' }, '需要 .xlsx 格式，最大 20 MB。请上传包含全部题目的完整文件。'),
    h('button', { class: 'btn primary big', type: 'button', disabled: A.busy, onclick: () => input.click() }, '选择 Excel 文件'),
    input);
  drop.addEventListener('dragover', (e) => { e.preventDefault(); drop.classList.add('over'); });
  drop.addEventListener('dragleave', () => drop.classList.remove('over'));
  drop.addEventListener('drop', (e) => { e.preventDefault(); drop.classList.remove('over'); if (!A.busy) uploadFile(e.dataTransfer.files[0]); });

  const done = A.done;
  return h('div', { class: 'wrap' },
    h('header', { class: 'top' }, brand(), h('button', { class: 'btn', type: 'button', onclick: closeAdmin }, '返回首页')),
    h('h2', { class: 'page-h' }, '题库管理'),
    A.error ? h('div', { class: 'err', role: 'alert' }, h('p', null, A.error)) : null,
    done ? h('div', { class: 'okbox', role: 'status' },
      h('p', { class: 'ok-t' }, `已导入 ${done.filename}`),
      h('p', null, `新增 ${done.added} 题，更新 ${done.updated} 题，恢复 ${done.restored} 题，下架 ${done.removed} 题。现在题库共 ${done.total} 题。`),
      h('div', { class: 'acts' }, h('button', { class: 'btn primary', type: 'button', onclick: closeAdmin }, '去考试'))) : null,
    A.preview ? previewView(A.preview) : h('section', { class: 'panel' },
      h('h2', null, '更新题库'),
      h('ol', { class: 'steps' },
        h('li', null, '在电脑上修改你的 Excel（新增工作表、新增或修改题目都可以）。'),
        h('li', null, '保存后，把整份 Excel 上传到这里。'),
        h('li', null, '先看检查结果，确认没问题再导入。')),
      drop),
    h('section', { class: 'panel' },
      h('h2', null, '现在的题库'),
      b.questions.length ? h('div', { class: 'sheetlist' }, b.sheets.map((s) => h('div', { class: 'sheetrow' },
        h('div', { class: 'name' }, s.name),
        h('div', { class: 'dim' }, `${s.usable} 题可用` + (s.inactive ? `，${s.inactive} 题已下架` : ''))))) : h('p', { class: 'empty' }, '题库是空的。'),
      A.imports.length ? [
        h('h3', { class: 'sub-h' }, '导入记录'),
        h('ul', { class: 'hist' }, A.imports.slice(0, 8).map((x) => h('li', null,
          h('span', { class: 'pct num' }, `${x.total}`),
          h('span', { class: 'meta' }, `${x.at}，${x.filename}`,
            h('div', { class: 'dim' }, `新增 ${x.added}，更新 ${x.updated}，恢复 ${x.restored}，下架 ${x.removed}` + (x.skipped ? `，${x.skipped} 题读不出来` : ''))))))] : null));
}

/* ---------------------------------------------------------------- 答题 */

function renderQuiz() {
  const se = S.session, q = se.qs[se.idx], n = se.qs.length;
  S.refresh = [];

  const answeredNow = () => se.qs.filter((x) => stateOf(x, se.answers[x.id]) === 2).length;
  const meterFill = h('i');
  const railCount = h('span', { class: 'num' });
  S.refresh.push(() => {
    const a = answeredNow();
    meterFill.style.width = (100 * a / n) + '%';
    railCount.textContent = `已答 ${a} / ${n} 题`;
  });

  // 当前题用哪种语言显示：这道题没有可用的中文翻译时，一律显示日语
  const hasZh = !!q.zh;
  const lang = S.lang === 'zh' && hasZh ? 'zh' : 'ja';
  const stemText = lang === 'zh' ? q.zh.stem : q.stem;

  let body;
  if (q.kind === 'hotspot') {
    body = h('div', { class: 'hs' }, q.items.map((it, i) => h('div', { class: 'hs-item' },
      h('span', { class: 'hs-no' }, i + 1),
      h('p', { lang, class: 'pre' }, lang === 'zh' ? q.zh.items[i] : it.text),
      h('div', { class: 'chips', role: 'group', 'aria-label': `第 ${i + 1} 小项` }, it.choices.map((ch, j) => {
        const btn = h('button', { class: 'chip', type: 'button', 'aria-pressed': 'false', onclick: () => pickItem(q, i, j) },
          h('span', { class: 'dot' }), h('span', { lang: 'ja' }, ch));     // 选项本身（はい / いいえ 等）是答案原文，不翻译
        S.refresh.push(() => btn.setAttribute('aria-pressed', String((se.answers[q.id] || [])[i] === j)));
        return btn;
      })))));
  } else {
    body = h('div', { class: 'opts' + (q.kind === 'multi' ? ' multi' : ''), role: 'group', 'aria-label': '选项' }, q.options.map((o, i) => {
      const btn = h('button', { class: 'opt', type: 'button', 'aria-pressed': 'false', onclick: () => pick(q, o.key) },
        h('span', { class: 'bubble' }, o.key), h('span', { class: 't', lang }, lang === 'zh' ? q.zh.options[i] : o.text));
      S.refresh.push(() => btn.setAttribute('aria-pressed', String((se.answers[q.id] || []).includes(o.key))));
      return btn;
    }));
  }

  const langBtn = (code, label) => h('button', {
    class: 'lang-btn', type: 'button', lang: code, 'aria-pressed': String(lang === code),
    disabled: code === 'zh' && !hasZh,
    title: code === 'zh' && !hasZh ? '这道题的 B 列没有中文翻译' : null,
    onclick: () => setLang(code),
  }, label);
  const langSwitch = h('div', { class: 'lang', role: 'group', 'aria-label': '题目语言' }, langBtn('ja', '日语'), langBtn('zh', '中文'));

  const flagBtn = h('button', { class: 'btn flagbtn', type: 'button', 'aria-pressed': 'false', onclick: () => toggleFlag(q) }, '标记这题');
  S.refresh.push(() => flagBtn.setAttribute('aria-pressed', String(se.flags.has(q.id))));

  const last = se.idx === n - 1;
  const paper = h('article', { class: 'paper' },
    h('div', { class: 'phead' }, h('span', { class: 'badge' }, kindLabel(q)), h('span', null, q.sheet), langSwitch),
    h('div', { class: 'stem' }, paras(stemText, null, lang)),
    body,
    h('div', { class: 'pfoot' },
      h('div', { class: 'grp' },
        h('button', { class: 'btn', type: 'button', disabled: se.idx === 0, onclick: () => go(se.idx - 1) }, '上一题'),
        last ? h('button', { class: 'btn primary', type: 'button', onclick: submit }, '交卷')
             : h('button', { class: 'btn primary', type: 'button', onclick: () => go(se.idx + 1) }, '下一题')),
      flagBtn));

  const tiles = h('div', { class: 'tiles' }, se.qs.map((x, i) => {
    const t = h('button', { class: 'tile num', type: 'button', onclick: () => go(i) }, String(i + 1));
    S.refresh.push(() => {
      const st = stateOf(x, se.answers[x.id]), fl = se.flags.has(x.id), cur = i === se.idx;
      t.className = 'tile num' + (st === 2 ? ' done' : st === 1 ? ' part' : '') + (cur ? ' cur' : '') + (fl ? ' flag' : '');
      t.setAttribute('aria-label', `第 ${i + 1} 题，${['未答', '只答了一部分', '已答'][st]}${fl ? '，已标记' : ''}${cur ? '，当前题' : ''}`);
      if (cur) t.setAttribute('aria-current', 'true'); else t.removeAttribute('aria-current');
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
    h('button', { class: 'btn primary', type: 'button', onclick: submit }, '交卷'),
    h('p', { class: 'keys' }, '← → 切换题目', h('br'), 'A–E 选答案　M 标记'));

  const timerEl = h('span', { class: 'timer num', id: 'timer' }, fmtTime(elapsed(se)));
  S.timer = setInterval(() => { timerEl.textContent = fmtTime(elapsed(se)); }, 1000);

  S.refresh.forEach((f) => f());

  return h('div', { class: 'wrap' },
    h('div', { class: 'qbar' },
      h('div', { class: 'l' }, h('b', { class: 'num' }, `第 ${se.idx + 1} 题`), h('span', { class: 'dim num' }, `共 ${n} 题`)),
      h('div', { class: 'r' }, timerEl,
        h('button', { class: 'btn quiet', type: 'button', onclick: leaveQuiz }, '暂存退出'))),
    h('div', { class: 'meter', 'aria-hidden': 'true' }, meterFill),
    h('div', { class: 'quiz-grid' }, paper, rail));
}

function setLang(code) {
  if (S.lang === code) return;
  S.lang = code;
  try { localStorage.setItem(LS_LANG, code); } catch (e) { /* 存不了就只在这次有效 */ }
  const y = window.scrollY;
  render();                 // 只换文字；已选的答案、标记、计时都存在 S.session 里，不受影响
  window.scrollTo(0, y);
}

function syncQuiz() {
  S.refresh.forEach((f) => f());
  saveSession();
}

function go(i) {
  const se = S.session;
  if (!se || i < 0 || i >= se.qs.length) return;
  se.idx = i;
  saveSession();
  render();
  window.scrollTo(0, 0);
}

function pick(q, key) {
  const se = S.session;
  let a = se.answers[q.id] || [];
  if (q.kind === 'single') a = a.includes(key) ? [] : [key];
  else a = a.includes(key) ? a.filter((k) => k !== key) : [...a, key].sort();
  if (a.length) se.answers[q.id] = a; else delete se.answers[q.id];
  syncQuiz();
}

function pickItem(q, i, j) {
  const se = S.session;
  const a = Array.isArray(se.answers[q.id]) ? se.answers[q.id].slice() : Array(q.items.length).fill(null);
  a[i] = a[i] === j ? null : j;
  if (a.every((x) => x == null)) delete se.answers[q.id]; else se.answers[q.id] = a;
  syncQuiz();
}

function toggleFlag(q) {
  const se = S.session;
  if (se.flags.has(q.id)) se.flags.delete(q.id); else se.flags.add(q.id);
  syncQuiz();
}

document.addEventListener('keydown', (e) => {
  if (S.view !== 'quiz' || !S.session || e.ctrlKey || e.metaKey || e.altKey || e.repeat) return;
  if (document.querySelector('dialog[open]')) return;
  if (e.target && /^(INPUT|TEXTAREA|SELECT)$/.test(e.target.tagName)) return;
  const se = S.session, q = se.qs[se.idx];
  if (e.key === 'ArrowRight' && se.idx < se.qs.length - 1) { e.preventDefault(); go(se.idx + 1); }
  else if (e.key === 'ArrowLeft' && se.idx > 0) { e.preventDefault(); go(se.idx - 1); }
  else if (e.key === 'm' || e.key === 'M') { toggleFlag(q); }
  else if (/^[a-hA-H]$/.test(e.key) && q.kind !== 'hotspot') {
    const k = e.key.toUpperCase();
    if (q.options.some((o) => o.key === k)) pick(q, k);
  }
});

/* ---------------------------------------------------------------- 成绩与解析 */

function renderResult() {
  const R = S.result, n = R.results.length;
  const good = R.results.filter((r) => r.correct).length, wrong = n - good;
  const pct = roundHalfEven(100 * good / n);

  const list = h('div', { class: 'rvlist' });
  const segW = h('button', { type: 'button', 'aria-pressed': 'true' }, `只看错题（${wrong}）`);
  const segA = h('button', { type: 'button', 'aria-pressed': 'false' }, `全部题目（${n}）`);
  const fill = () => {
    segW.setAttribute('aria-pressed', String(S.rvFilter === 'wrong'));
    segA.setAttribute('aria-pressed', String(S.rvFilter === 'all'));
    const items = R.results.map((r, i) => [r, i]).filter(([r]) => S.rvFilter === 'all' || !r.correct);
    list.replaceChildren(...(items.length
      ? items.map(([r, i]) => reviewItem(r, i))
      : [h('div', { class: 'allgood' }, '这一场全部答对，没有错题。想复习的话，切到「全部题目」。')]));
  };
  segW.addEventListener('click', () => { S.rvFilter = 'wrong'; fill(); });
  segA.addEventListener('click', () => { S.rvFilter = 'all'; fill(); });
  fill();

  const strip = h('div', { class: 'strip' }, R.results.map((r, i) => h('button', {
    class: r.correct ? 'ok' : 'ng', type: 'button', 'aria-label': `第 ${i + 1} 题，${r.correct ? '答对' : '答错'}，查看解析`,
    onclick: () => {
      if (S.rvFilter === 'wrong' && r.correct) { S.rvFilter = 'all'; fill(); }
      const el = document.getElementById('rv-' + i);
      if (el) el.scrollIntoView({ block: 'start' });
    },
  }, h('span', { class: 'n' }, i + 1), svgMark(r.correct))));

  const again = () => startSession(shuffle(R.results.map((r) => r.q)), R.mode);
  const retryWrong = () => startSession(shuffle(R.results.filter((r) => !r.correct).map((r) => r.q)), 'retry');
  const home = () => { S.view = 'home'; render(); window.scrollTo(0, 0); };

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

function reviewItem(r, i) {
  const q = r.q, st = qStat(q.id);
  const unanswered = stateOf(q, r.ans) === 0;
  let body, verdict;

  if (q.kind === 'hotspot') {
    const okN = q.items.filter((_, k) => r.ans && r.ans[k] === q.answer[k]).length;
    body = h('div', { class: 'hs' }, q.items.map((it, k) => {
      const mine = r.ans ? r.ans[k] : null, right = q.answer[k], ok = mine === right;
      return h('div', { class: 'hs-item ' + (ok ? 'rv-ok' : 'rv-ng') },
        h('span', { class: 'hs-no' }, k + 1),
        h('p', { lang: 'ja', class: 'pre' }, it.text),
        h('div', { class: 'hsa' },
          h('span', { class: 'tag ' + (ok ? 'g' : 'r') }, ok ? '答对' : '答错'),
          h('span', null, '你选了 ', h('b', { class: ok ? 'g' : 'r', lang: 'ja' }, mine == null ? '未作答' : it.choices[mine])),
          ok ? null : h('span', null, '正确答案 ', h('b', { class: 'g', lang: 'ja' }, it.choices[right]))));
    }));
    verdict = h('p', { class: 'verdict' }, `这题共 ${q.items.length} 个小项，答对 `, h('b', { class: r.correct ? 'r' : 'w' }, `${okN} 个`), '；每个小项都对才算这题答对。');
  } else {
    body = h('div', { class: 'opts' + (q.kind === 'multi' ? ' multi' : '') }, q.options.map((o) => {
      const isAns = q.answer.includes(o.key), picked = !!r.ans && r.ans.includes(o.key);
      const tags = [isAns ? h('span', { class: 'tag g' }, '正确答案') : null, picked ? h('span', { class: 'tag ' + (isAns ? 'k' : 'r') }, '你的选择') : null].filter(Boolean);
      return h('div', { class: 'opt review' + (isAns ? ' is-ans' : '') + (picked && !isAns ? ' is-bad' : '') },
        h('span', { class: 'bubble' }, o.key),
        h('div', { class: 'body' }, tags.length ? h('div', { class: 'tags' }, tags) : null, h('span', { class: 't', lang: 'ja' }, o.text)));
    }));
    verdict = h('p', { class: 'verdict' }, '你的答案 ', h('b', { class: r.correct ? 'r' : 'w' }, unanswered ? '未作答' : r.ans.join('、')),
      '　正确答案 ', h('b', { class: 'r' }, q.answer.join('、')));
  }

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
        (inNotebook(q.id) ? '，还在错题本里' : st.wrong ? '，已经移出错题本' : '')) : null));
}

/* ---------------------------------------------------------------- 总入口 */

function render() {
  clearInterval(S.timer);
  S.timer = null;
  let view;
  if (S.view === 'loading') view = h('div', { class: 'wrap' }, h('p', { class: 'dim' }, '正在读取题库…'));
  else if (S.view === 'fatal') view = h('div', { class: 'wrap' }, h('header', { class: 'top' }, brand()),
    h('div', { class: 'err' }, h('p', null, '连不上服务。请确认程序（或 Docker 容器）还在运行，然后刷新这个页面。如果你是通过域名访问的，也可能是登录已过期，刷新后重新登录即可。')));
  else if (S.view === 'admin') view = renderAdmin();
  else if (S.view === 'quiz' && S.session) view = renderQuiz();
  else if (S.view === 'result' && S.result) view = renderResult();
  else { S.view = 'home'; view = renderHome(); }
  document.getElementById('app').replaceChildren(view);
  document.title = S.view === 'quiz' ? `第 ${S.session.idx + 1} / ${S.session.qs.length} 题 - Xexam` : 'Xexam';
}

(async function init() {
  render();
  try {
    const [b, p, info] = await Promise.all([
      fetch('/api/bank').then((r) => r.json()),
      fetch('/api/progress').then((r) => r.json()),
      fetch('/api/info').then((r) => r.json()),
    ]);
    S.progress = p; S.info = info;
    setBank(b);
    S.view = 'home';
  } catch (e) {
    S.view = 'fatal';
  }
  render();
})();
