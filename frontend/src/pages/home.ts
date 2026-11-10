// 首页：选范围、开始考试、学习记录。
import { inNotebook, masterStreakOf, shuffle } from '../domain/grading.ts';
import { clearSaved, loadSaved } from '../state/session-storage.ts';
import { S, pool } from '../state/store.ts';
import { clearProgress, openAdmin, render, resumeSession, startSession } from '../state/actions.ts';
import { brand, modeName } from '../components/common.ts';
import { shutdownApp } from '../api/import.ts';
import { ask } from '../utils/dialog.ts';
import { h } from '../utils/dom.ts';
import { fmtTime } from '../utils/format.ts';
import type { Mode } from '../types/api.ts';

function historyPanel(): HTMLElement {
  const hist = S.progress?.history ?? [];
  const panel = h('section', { class: 'panel' }, h('h2', null, '学习记录'));
  const first = hist[0];
  if (!first) {
    panel.append(h('p', { class: 'empty' }, '还没有考试记录。考完一场，成绩会出现在这里。'));
    return panel;
  }
  const best = Math.max(...hist.map((x) => x.percent));
  const recent = hist.slice(0, 12).reverse();
  panel.append(
    h('div', { class: 'stat' }, h('b', { class: 'num' }, first.percent + '%'), h('span', { class: 'dim' }, '最近一场')),
    h('p', { class: 'dim' }, `一共考过 ${hist.length} 场，最高 ${best}%`),
    h('div', { class: 'bars', role: 'img', 'aria-label': '最近成绩：' + recent.map((x) => x.percent + '%').join('，') },
      recent.map((x) => h('i', { style: `height:${Math.max(x.percent, 3)}%`, title: `${x.at}  ${x.percent}%` }))),
    h('ul', { class: 'hist' }, hist.slice(0, 6).map((x) => h('li', null,
      h('span', { class: 'pct' }, x.percent + '%'),
      h('span', { class: 'meta' }, `${x.at}，${modeName(x.mode)}，答对 ${x.correct} / ${x.total} 题，用时 ${fmtTime(x.seconds)}`,
        x.sheets?.length ? h('div', { class: 'dim' }, x.sheets.join('、')) : null)))),
    h('div', { style: 'margin-top:12px' },
      h('button', { class: 'btn quiet danger', type: 'button', onclick: () => void clearProgress() }, '清空学习记录')));
  return panel;
}

async function quitProgram(): Promise<void> {
  if (!(await ask({ title: '关闭程序？', body: '关闭后这个页面就用不了了，下次要重新启动。学习记录已经保存好了。', ok: '关闭', cancel: '不关' }))) return;
  try {
    await shutdownApp();
  } catch {
    /* 服务端关得快时会断开 */
  }
  document.getElementById('app')?.replaceChildren(h('div', { class: 'wrap' }, h('div', { class: 'panel' }, h('h2', null, '程序已关闭'), h('p', null, '可以关掉这个页面了。'))));
}

export function renderHome(): HTMLElement {
  const b = S.bank;
  if (!b) return h('div', { class: 'wrap' });
  const cfg = S.cfg;
  const selected = cfg.selected ?? new Set<string>();
  const ms = masterStreakOf(S.progress);
  const stat = (id: string) => S.progress?.questions?.[id];
  const saved = loadSaved(b);
  const nb = b.questions.filter((q) => inNotebook(stat(q.id), ms));
  if (cfg.mode === 'weak' && !nb.length) cfg.mode = 'all';

  const countInput = h('input', { type: 'number', min: '1', step: '1', inputmode: 'numeric', 'aria-label': '题数' });
  const shuffleBox = h('input', { type: 'checkbox', checked: cfg.shuffle });
  const startBtn = h('button', { class: 'btn primary big', type: 'button' }, '开始考试');
  const poolText = h('span', { class: 'dim' });

  const refreshSetup = (): void => {
    const n = pool().length;
    countInput.max = String(n);
    countInput.disabled = n === 0;
    countInput.value = n ? String(cfg.count == null ? n : Math.max(1, Math.min(cfg.count, n))) : '';
    startBtn.disabled = n === 0;
    poolText.textContent = n ? `当前范围共 ${n} 题` : (cfg.mode === 'weak' ? '所选工作表里没有待巩固的错题' : '先勾选一个工作表');
  };
  countInput.addEventListener('change', () => {
    const n = pool().length;
    const v = parseInt(countInput.value, 10);
    cfg.count = !v || v >= n ? null : Math.max(1, v);
    refreshSetup();
  });
  startBtn.addEventListener('click', () => {
    void (async () => {
      const p = pool();
      if (!p.length) return;
      if (loadSaved(b) && !(await ask({ title: '放弃没交卷的考试？', body: '有一场暂存的考试还没交卷。现在开始新的考试，会把它覆盖掉。', ok: '开始新考试', cancel: '回去继续', danger: true }))) return;
      let n = parseInt(countInput.value, 10);
      if (!(n >= 1)) n = p.length;
      n = Math.min(n, p.length);
      cfg.shuffle = shuffleBox.checked;
      startSession((cfg.shuffle ? shuffle(p) : p).slice(0, n), cfg.mode);
    })();
  });

  const sheetRows = b.sheets.map((s) => {
    const usable = s.usable > 0;
    const wrongN = b.questions.filter((q) => q.sheet === s.name && inNotebook(stat(q.id), ms)).length;
    const cb = h('input', {
      type: 'checkbox', checked: usable && selected.has(s.name), disabled: !usable,
      onchange: () => {
        if (cb.checked) selected.add(s.name);
        else selected.delete(s.name);
        refreshSetup();
      },
    });
    const sub = usable ? `${s.usable} 题可用` + (s.inactive ? `，${s.inactive} 题已下架` : '') : '这个工作表里的题都已下架';
    return h('label', { class: 'row-opt' + (usable ? '' : ' off') }, cb,
      h('span', { class: 'grow' }, h('div', { class: 'name' }, s.name), h('div', { class: 'sub' }, sub)),
      wrongN ? h('span', { class: 'chipnum' }, `错题 ${wrongN}`) : null);
  });

  const modeRow = (val: Mode, title: string, sub: string, disabled: boolean): HTMLElement => {
    const r = h('input', {
      type: 'radio', name: 'mode', checked: cfg.mode === val, disabled,
      onchange: () => {
        cfg.mode = val;
        refreshSetup();
      },
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
          onclick: () => void (async () => {
            if (await ask({ title: '放弃这场考试？', body: '已经作答的内容不会保留。', ok: '放弃', cancel: '先留着', danger: true })) {
              clearSaved();
              render();
            }
          })(),
        }, '放弃这场'))) : null,
    b.error ? h('div', { class: 'err' }, h('p', null, b.error)) : null,
    b.questions.length ? [
      h('fieldset', null, h('legend', null, '考哪些工作表'), sheetRows),
      h('fieldset', null, h('legend', null, '从哪里出题'),
        modeRow('all', '全部题目', '按所选工作表出题', false),
        modeRow('weak', '错题本', nb.length ? `${nb.length} 题待巩固，连续答对 ${ms} 次后移出` : '还没有错题，先考一轮', !nb.length)),
      h('div', { class: 'inline' },
        h('label', null, '题数', countInput),
        h('label', null, shuffleBox, '打乱题目顺序')),
      h('div', { class: 'inline' }, startBtn, poolText),
    ] : (b.error ? null : h('div', { class: 'emptybank' },
      h('p', null, '题库还是空的。先上传你的 Excel 题库，再开始考试。'),
      h('button', { class: 'btn primary', type: 'button', onclick: () => void openAdmin() }, '去上传题库'))));

  refreshSetup();

  const inactive = b.sheets.reduce((n, s) => n + (s.inactive || 0), 0);
  const bankInfo = h('div', { class: 'bankinfo' },
    b.file ? h('span', null, b.file) : null,
    b.mtime ? h('span', { class: 'num' }, '更新于 ' + b.mtime) : null,
    h('span', null, `${b.questions.length} 题，${b.sheets.filter((s) => s.usable).length} 个工作表` + (inactive ? `（另有 ${inactive} 题已下架）` : '')),
    h('button', { class: 'btn', type: 'button', onclick: () => void openAdmin() }, '题库管理 / 上传 Excel'));

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
      S.info.canShutdown ? h('button', { class: 'btn quiet', type: 'button', onclick: () => void quitProgram() }, '关闭程序') : null));
}
