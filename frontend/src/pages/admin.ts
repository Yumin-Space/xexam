// 题库管理：上传 Excel → 预览（不写库）→ 确认 / 取消。
import { ApiError } from '../api/client.ts';
import { cancelImport, confirmImport as apiConfirm, fetchImports, uploadWorkbook } from '../api/import.ts';
import { brand } from '../components/common.ts';
import { S } from '../state/store.ts';
import { closeAdmin, refreshBank, render } from '../state/actions.ts';
import type { ImportPreview } from '../types/api.ts';
import { ask } from '../utils/dialog.ts';
import { h } from '../utils/dom.ts';

const MSG_NOT_XLSX = (name: string): string => `「${name}」不是 .xlsx 文件。请在 Excel 里另存为「Excel 工作簿(.xlsx)」再上传。`;

async function uploadFile(file: File | undefined): Promise<void> {
  const A = S.admin;
  if (!file || A.busy) return;
  A.error = '';
  A.done = null;
  A.preview = null;
  if (!/\.xlsx$/i.test(file.name)) {
    A.error = MSG_NOT_XLSX(file.name);
    render();
    return;
  }
  A.busy = true;
  render();
  try {
    const pv = await uploadWorkbook(file);
    if (!pv.token) throw new ApiError('unexpected', '服务返回的不是预期的内容。如果你是通过域名访问的，可能是登录已过期，请刷新页面重新登录后再上传。');
    A.preview = pv;
  } catch (e) {
    A.error = e instanceof ApiError ? e.message : '上传失败。请刷新页面后再试。';
  }
  A.busy = false;
  if (S.view === 'admin') {
    render();
    window.scrollTo(0, 0);
  }
}

async function confirmImport(): Promise<void> {
  const A = S.admin;
  const pv = A.preview;
  if (!pv) return;
  if (pv.removed > 0) {
    const ok = await ask({
      title: `这次会下架 ${pv.removed} 道题`,
      body: '这份文件里没有它们，所以会从题库里下架（不会再出现在考试里）。' +
        (pv.removed_stats ? `其中 ${pv.removed_stats} 道有答题记录，记录会保留，以后你把题传回来会自动恢复。` : '') +
        '\n确定要继续吗？如果传错了文件，请选「回去检查」。',
      ok: '继续导入', cancel: '回去检查', danger: true,
    });
    if (!ok) return;
  }
  A.busy = true;
  render();
  try {
    const d = await apiConfirm(pv.token);
    await refreshBank();
    A.done = { filename: pv.filename, ...d.result, total: d.bank.questions.length };
    A.preview = null;
    try {
      A.imports = await fetchImports();
    } catch {
      /* 保留旧的 */
    }
  } catch (e) {
    A.error = e instanceof ApiError && e.kind === 'http'
      ? e.message
      : '导入失败。如果你是通过域名访问的，可能是登录已过期：请刷新页面重新登录，再重新上传。（如果刚才网络断了，导入可能已经完成，请看看题库里的题数。）';
    A.preview = null;
  }
  A.busy = false;
  if (S.view === 'admin') {
    render();
    window.scrollTo(0, 0);
  }
}

function pill(n: number, label: string, cls?: string): HTMLElement {
  return h('div', { class: 'pill ' + (cls ?? '') + (n ? '' : ' zero') }, h('b', { class: 'num' }, String(n)), h('span', null, label));
}

function previewView(pv: ImportPreview): HTMLElement {
  const A = S.admin;
  const nothing = pv.usable === 0;
  const same = !nothing && !pv.added && !pv.updated && !pv.removed && !pv.restored;
  return h('section', { class: 'panel' },
    h('h2', null, `检查结果：${pv.filename}`),
    nothing
      ? h('div', { class: 'err' }, h('p', null, '这份文件里一道可用的题都没读到，不能导入。' + (pv.skipped ? '看下面每个工作表里「没能读入」的原因。' : '请确认题目写在 A 列，答案写在 C 列。')))
      : h('p', { class: 'dim', style: 'margin-bottom:14px' }, '下面是导入后题库会发生的变化。现在还没有写入，确认后才会生效。'),
    h('div', { class: 'pills' },
      pill(pv.added, '新增', 'g'), pill(pv.updated, '更新', 'b'), pill(pv.restored, '恢复上架', 'b'),
      pill(pv.removed, '下架', pv.removed ? 'r' : ''), pill(pv.unchanged, '没变化'), pill(pv.skipped, '读不出来', pv.skipped ? 'r' : '')),
    pv.removed ? h('div', { class: 'warnbox' },
      h('p', null, `有 ${pv.removed} 道现有的题在这份文件里找不到，导入后会下架。` + (pv.removed_stats ? `其中 ${pv.removed_stats} 道有答题记录，记录会保留。` : '')),
      pv.removed_samples?.length ? h('ul', null, pv.removed_samples.map((t) => h('li', { lang: 'ja' }, t + '…'))) : null,
      h('p', { class: 'dim' }, '如果你只是想新增题，请上传包含全部题目（旧的加新的）的完整 Excel。')) : null,
    same ? h('p', { class: 'dim', style: 'margin-top:12px' }, '这份文件和现在的题库完全一样，不需要导入。') : null,
    h('h3', { class: 'sub-h' }, '各工作表'),
    h('div', { class: 'sheetlist' }, pv.sheets.map((s) => h('div', { class: 'sheetrow' },
      h('div', { class: 'name' }, s.name),
      h('div', { class: 'dim' },
        `${s.usable} 题可用` +
        (s.added ? `，新增 ${s.added}` : '') + (s.updated ? `，更新 ${s.updated}` : '') +
        (s.restored ? `，恢复 ${s.restored}` : '') + (s.removed ? `，下架 ${s.removed}` : '') +
        ((s.skippedCount ?? 0) ? `，${s.skippedCount} 题读不出来` : '')),
      (s.skippedCount ?? 0) ? h('details', { class: 'skipbox' },
        h('summary', null, `${s.skippedCount} 题没能读入，点开看原因`),
        h('ul', null, s.skipped.map((k) => h('li', null, `第 ${k.row} 行：${k.reason}`)),
          s.skippedMore ? h('li', { class: 'dim' }, `……还有 ${s.skippedMore} 条类似的`) : null)) : null))),
    h('div', { class: 'acts', style: 'margin-top:20px' },
      h('button', { class: 'btn primary', type: 'button', disabled: nothing || same || A.busy, onclick: () => void confirmImport() }, nothing ? '没有可导入的题' : same ? '无需导入' : '确认导入'),
      h('button', {
        class: 'btn', type: 'button', disabled: A.busy,
        onclick: () => void (async () => {
          try {
            await cancelImport();
          } catch {
            /* 过期了也没关系 */
          }
          A.preview = null;
          render();
        })(),
      }, '取消，不导入')));
}

export function renderAdmin(): HTMLElement {
  const A = S.admin;
  const b = S.bank;
  const input = h('input', {
    type: 'file', accept: '.xlsx,application/vnd.openxmlformats-officedocument.spreadsheetml.sheet', id: 'xlsx-input', hidden: true,
    onchange: (e: Event) => {
      const el = e.target as HTMLInputElement;
      const f = el.files?.[0];
      el.value = '';
      void uploadFile(f);
    },
  });
  const drop = h('div', { class: 'drop' + (A.busy ? ' busy' : ''), role: 'group', 'aria-label': '上传 Excel' },
    h('p', { class: 'drop-t' }, A.busy ? '正在读取和检查…' : '选择你更新后的 Excel 文件'),
    h('p', { class: 'dim' }, '需要 .xlsx 格式，最大 20 MB。请上传包含全部题目的完整文件。'),
    h('button', { class: 'btn primary big', type: 'button', disabled: A.busy, onclick: () => input.click() }, '选择 Excel 文件'),
    input);
  drop.addEventListener('dragover', (e) => {
    e.preventDefault();
    drop.classList.add('over');
  });
  drop.addEventListener('dragleave', () => drop.classList.remove('over'));
  drop.addEventListener('drop', (e) => {
    e.preventDefault();
    drop.classList.remove('over');
    if (!A.busy) void uploadFile(e.dataTransfer?.files[0]);
  });

  const done = A.done;
  const hasQuestions = !!b && b.questions.length > 0;
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
      hasQuestions && b
        ? h('div', { class: 'sheetlist' }, b.sheets.map((s) => h('div', { class: 'sheetrow' },
          h('div', { class: 'name' }, s.name),
          h('div', { class: 'dim' }, `${s.usable} 题可用` + (s.inactive ? `，${s.inactive} 题已下架` : '')))))
        : h('p', { class: 'empty' }, '题库是空的。'),
      A.imports.length ? [
        h('h3', { class: 'sub-h' }, '导入记录'),
        h('ul', { class: 'hist' }, A.imports.slice(0, 8).map((x) => h('li', null,
          h('span', { class: 'pct num' }, `${x.total}`),
          h('span', { class: 'meta' }, `${x.at}，${x.filename}`,
            h('div', { class: 'dim' }, `新增 ${x.added}，更新 ${x.updated}，恢复 ${x.restored}，下架 ${x.removed}` + (x.skipped ? `，${x.skipped} 题读不出来` : ''))))))] : null));
}
