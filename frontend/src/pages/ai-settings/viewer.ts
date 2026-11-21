// AI settings page, the read-only viewer of one user's chats and memories (every view is written to the activity log by the AI
// service). v1.3: the viewer shows the branch the user is viewing; a message with other versions (a regenerated answer, an edited
// question) has a small version switcher that loads the branch through that version ("at"), which only changes this view and
// never what the user sees. Administrators can see which connection answered a message. Deleting works as before.
import { fetchAiUser } from '../../api/ai.ts';
import { adminChat, clearAiData } from '../../api/ai-admin.ts';
import { markdownEl } from '../../components/ai-render.ts';
import { iconButton } from '../../components/icon-button.ts';
import { miniIcon } from '../../components/mini-icons.ts';
import { footerOf } from '../../domain/ai-chat.ts';
import { focusAfterClear } from '../../domain/ai-settings.ts';
import { aiCodeText, errorText } from '../../i18n/errors.ts';
import { t } from '../../i18n/index.ts';
import type { AiAdminUserDetail, AiAdminUserRow, AiChatDetail, AiMessage } from '../../types/api.ts';
import { ask, dialogTitleId } from '../../utils/dialog.ts';
import { h, type Child } from '../../utils/dom.ts';
import { matches, PHONE_MQ } from '../../utils/media.ts';
import { toast } from '../../utils/toast.ts';
import { chatTitleEl, chatTitleText, reasoningEl, userBody, whoLine } from '../ai.ts';
import { jobTime } from '../../domain/ai-settings.ts';
import { aiDown } from './status.ts';
import { num, when } from './state.ts';

/** A message that has other versions (2 or more): previous / next, "Version 2 of 3"; only this view changes */
function versionSwitch(m: AiMessage, show: (at: number) => void): HTMLElement | null {
  const v = m.versions;
  if (!v || v.length < 2) return null;
  const i = v.indexOf(m.id);
  const prev = i > 0 ? v[i - 1] : undefined;
  const next = i >= 0 && i < v.length - 1 ? v[i + 1] : undefined;
  const btn = (label: string, to: number | undefined, icon: 'back' | 'fwd'): HTMLButtonElement => {
    const b = iconButton({ icon, label, onClick: () => to !== undefined && show(to) });
    b.disabled = to === undefined;
    return b;
  };
  return h('div', { class: 'ais-vsw', role: 'group', 'aria-label': t('ai.version.of', { n: i + 1, total: v.length }) },
    btn(t('ai.version.prev'), prev, 'back'),
    h('span', { class: 'ais-vsw-n' }, t('ai.version.of', { n: i + 1, total: v.length })),
    btn(t('ai.version.next'), next, 'fwd'));
}

function viewerMessage(m: AiMessage, show: (at: number) => void, open: Map<number, boolean>): HTMLElement {
  const user = m.role === 'user';
  const f = footerOf(m);
  const foot = !f ? null
    : f.kind === 'writing' ? t('ai.writing')
      : f.kind === 'stopped' ? t('ai.status.stopped')
        : f.kind === 'error' ? t('ai.status.error', { reason: aiCodeText(f.code) })
          : f.kind === 'noAnswer' ? t('ai.status.noAnswer', { reason: aiCodeText(f.code) })
            : t('ai.status.truncated');
  return h('li', { class: 'ai-msg ' + (user ? 'user' : 'assistant') },
    whoLine(m, true, jobTime),
    m.connection ? h('span', { class: 'tag k ais-vconn' }, m.connection) : null,
    versionSwitch(m, show),
    user ? null : reasoningEl(m, { idBase: 'ais-rs', userOpen: open.get(m.id), onToggle: (o) => open.set(m.id, o) }),
    ...(user ? userBody(m) : [markdownEl(m.content)]),
    foot ? h('p', { class: 'ai-foot' + (m.status === 'error' ? ' bad' : '') }, foot) : null);
}

/** The read-only viewer of a user's chats and memories (every view is written to the System log by the AI service) */
export async function openViewer(u: AiAdminUserRow, onChanged: () => void): Promise<void> {
  const titleId = dialogTitleId();
  let detail: AiAdminUserDetail | null = null;
  let err: unknown = null;
  let vtab: 'chats' | 'memories' = 'chats';
  let chat: AiChatDetail | null = null;
  let chatId: number | null = null;
  let chatErr: unknown = null;
  let busy = false;
  // v1.3: the reader's own open / closed choice of each reasoning block
  const reasoningOpen = new Map<number, boolean>();
  // PHONE: a full-screen sheet with a back chevron, the title and ×; the delete buttons become a trash icon (the open chat)
  // and a "Clear…" menu (all chats, all memories). Wider screens keep the dialog with its buttons at the bottom.
  let menuOpen = false;
  const phoneMq = typeof window.matchMedia === 'function' ? window.matchMedia(PHONE_MQ) : null;
  const d = h('dialog', { class: 'dlg-viewer', 'aria-labelledby': titleId });

  const loadDetail = async (): Promise<void> => {
    try {
      detail = await fetchAiUser(u.id);
      err = null;
      if (chatId != null && !detail.chats.some((c) => c.id === chatId)) {
        chatId = null;
        chat = null;
      }
    } catch (e) {
      if (aiDown(e)) {
        d.close();
        return;
      }
      err = e;
    }
    draw();
  };
  // `at`: the version whose branch is shown (null = the branch the user is viewing)
  const openChat = async (id: number, at: number | null = null): Promise<void> => {
    chatId = id;
    chat = null;
    chatErr = null;
    draw();
    try {
      chat = await adminChat(u.id, id, at ?? undefined);
    } catch (e) {
      if (aiDown(e)) {
        d.close();
        return;
      }
      chatErr = e;
    }
    draw();
    document.getElementById('vw-back')?.focus();
  };
  const clear = async (what: 'chat' | 'chats' | 'memories'): Promise<void> => {
    const dt = detail;
    if (!dt || busy) return;
    const picked = what === 'chat' ? dt.chats.find((c) => c.id === chatId) : undefined;
    const title = picked ? chatTitleText(picked) : what === 'chat' ? t('ai.new') : '';
    const body = what === 'chat'
      ? t('ai.set.user.clearChat.ask', { title, name: u.username })
      : what === 'chats'
        ? t('ai.set.user.clearChats.ask', { n: dt.chats.length, name: u.username })
        : t('ai.set.user.clearMemories.ask', { n: dt.memories.length, name: u.username });
    const heading = t(what === 'chat' ? 'ai.set.user.clearChat' : what === 'chats' ? 'ai.set.user.clearChats' : 'ai.set.user.clearMemories');
    if (!(await ask({ title: heading, body, ok: t('ai.set.user.clearOk'), danger: true }))) return;
    busy = true;
    draw();
    try {
      await clearAiData(what === 'chat' && chatId != null ? { userId: u.id, what: 'chat', chatId } : { userId: u.id, what: what === 'memories' ? 'memories' : 'chats' });
      toast(t('ai.set.user.cleared'));
      if (what !== 'memories') {
        chatId = null;
        chat = null;
      }
    } catch (e) {
      if (aiDown(e)) {
        d.close();
        return;
      }
      toast(errorText(e), 6000);
    }
    busy = false;
    await loadDetail();
    // The button that had the focus is gone (the dialog was drawn again): keep it inside the dialog
    document.getElementById(focusAfterClear(what, detail?.chats.map((c) => c.id) ?? []))?.focus();
    onChanged();
  };

  const backToList = (): void => {
    const was = chatId;
    chatId = null;
    chat = null;
    draw();
    if (was != null) document.getElementById(`vw-c-${was}`)?.focus();
  };

  function draw(): void {
    const dt = detail;
    const phone = matches(PHONE_MQ);
    const closeBtn = iconButton({ icon: 'x', label: t('common.close'), onClick: () => d.close() });
    closeBtn.classList.add('vw-close');
    const openChat1 = phone && dt != null && vtab === 'chats' && chatId != null;
    let back: HTMLButtonElement | null = null;
    if (openChat1) {
      back = iconButton({ icon: 'back', label: t('ai.back'), onClick: backToList });
      back.id = 'vw-back';
    }
    const tools: Child[] = [];
    if (phone && dt && (dt.chats.length || dt.memories.length)) {
      if (openChat1) {
        const trash = iconButton({ icon: 'trash', tone: 'danger', label: t('ai.set.user.clearChat'), onClick: () => void clear('chat') });
        trash.disabled = busy;
        tools.push(trash);
      }
      const more = iconButton({ icon: 'more', label: t('ai.set.user.clearMenu'), onClick: () => {
        menuOpen = !menuOpen;
        draw();
        document.getElementById('vw-more')?.focus();
      } });
      more.id = 'vw-more';
      more.classList.add('vw-more');
      more.setAttribute('aria-haspopup', 'menu');
      more.setAttribute('aria-expanded', String(menuOpen));
      more.disabled = busy;
      tools.push(more);
    }
    const head = h('div', { class: 'vw-head' },
      back,
      h('h3', { id: titleId }, t('ai.set.user.title', { name: u.username })),
      phone ? null : h('span', { class: 'tag k' }, t('ai.set.user.readOnly')),
      ...tools,
      closeBtn);
    if (!dt) {
      d.replaceChildren(head, err
        ? h('div', { class: 'err', role: 'alert' }, h('p', null, t('ai.set.user.err', { reason: errorText(err) })))
        : h('p', { class: 'dim', role: 'status' }, t('common.loading')));
      return;
    }
    const tabBtn = (which: 'chats' | 'memories', label: string): HTMLElement => h('button', {
      type: 'button', id: `vw-tab-${which}`, 'aria-pressed': String(vtab === which),
      onclick: () => {
        vtab = which;
        draw();
        document.getElementById(`vw-tab-${which}`)?.focus();
      },
    }, label);
    // The "Clear…" menu (PHONE): the two bulk deletes, in red text; each still asks first
    const menu = phone && menuOpen
      ? h('div', { class: 'vw-menu', role: 'menu', 'aria-label': t('ai.set.user.clearMenu') },
        dt.chats.length ? h('button', { type: 'button', role: 'menuitem', class: 'vw-menu-i', onclick: () => {
          menuOpen = false;
          void clear('chats');
        } }, t('ai.set.user.clearChats')) : null,
        dt.memories.length ? h('button', { type: 'button', role: 'menuitem', class: 'vw-menu-i', onclick: () => {
          menuOpen = false;
          void clear('memories');
        } }, t('ai.set.user.clearMemories')) : null)
      : null;
    let body: Child;
    if (!dt.chats.length && !dt.memories.length) {
      body = h('p', { class: 'ais-note' }, t('ai.set.user.noData'));
    } else if (vtab === 'memories') {
      body = dt.memories.length
        ? h('ul', { class: 'vw-mems' }, dt.memories.map((m) => h('li', null,
          h('span', { class: 'ai-mem-src', role: 'img', 'aria-label': t(m.source === 'chat' ? 'ai.mem.source.chat' : 'ai.mem.source.manual') },
            miniIcon(m.source === 'chat' ? 'chat' : 'pencil', 'ai-mi')),
          h('span', null, m.content))))
        : h('p', { class: 'ais-note' }, t('ai.mem.empty'));
    } else {
      const list = h('ul', { class: 'vw-chats' }, dt.chats.map((c) => h('li', null, h('button', {
        type: 'button', class: 'ai-chat-open' + (c.id === chatId ? ' on' : ''), id: `vw-c-${c.id}`, 'aria-current': c.id === chatId ? 'true' : null,
        onclick: () => void openChat(c.id),
      }, chatTitleEl(c), h('span', { class: 'ai-chat-at' }, when(c.updatedAt))))));
      const pane = chatId == null
        ? h('p', { class: 'ais-note vw-pick' }, t('ai.set.user.pick'))
        : chatErr
          ? h('div', { class: 'err', role: 'alert' }, h('p', null, t('ai.set.user.err', { reason: errorText(chatErr) })))
          : !chat
            ? h('p', { class: 'dim', role: 'status' }, t('common.loading'))
            : h('div', null,
              chat.messages.some((m) => (m.versions?.length ?? 0) > 1) ? h('p', { class: 'ais-note ais-vnote' }, t('ai.set.viewer.branch')) : null,
              h('ol', { class: 'ai-msgs' }, chat.messages.map((m) => viewerMessage(m, (at) => void openChat(chatId ?? 0, at), reasoningOpen))));
      body = h('div', { class: 'vw-panes ' + (chatId == null ? 'on-list' : 'on-chat') },
        h('div', { class: 'vw-list' }, dt.chats.length ? list : h('p', { class: 'ais-note' }, t('ai.empty'))),
        h('div', { class: 'vw-chat' },
          // The back control is in the header on PHONE; below 720 px (not PHONE) it stays above the chat
          phone ? null : h('button', { class: 'btn quiet vw-back', id: 'vw-back', type: 'button', onclick: backToList }, miniIcon('back', 'ai-mi'), t('ai.back')),
          pane));
    }
    d.replaceChildren(head,
      ...(menu ? [menu] : []),
      h('div', { class: 'seg vw-tabs', role: 'group' },
        tabBtn('chats', `${t('ai.set.user.chats')} (${num(dt.chats.length)})`),
        tabBtn('memories', `${t('ai.set.user.memories')} (${num(dt.memories.length)})`)),
      h('div', { class: 'vw-body' }, body),
      // PHONE has no bottom row (the duplicate Close and the red buttons moved to the header)
      ...(phone ? [] : [h('div', { class: 'acts vw-acts' },
        vtab === 'chats' && chatId != null
          ? h('button', { class: 'btn danger', type: 'button', disabled: busy, onclick: () => void clear('chat') }, t('ai.set.user.clearChat'))
          : null,
        dt.chats.length ? h('button', { class: 'btn danger', type: 'button', disabled: busy, onclick: () => void clear('chats') }, t('ai.set.user.clearChats')) : null,
        dt.memories.length ? h('button', { class: 'btn danger', type: 'button', disabled: busy, onclick: () => void clear('memories') }, t('ai.set.user.clearMemories')) : null)]));
  }

  // A turn of the phone between portrait and landscape (or a resize across 600 px) draws the other layout
  const onMq = (): void => {
    menuOpen = false;
    draw();
  };
  phoneMq?.addEventListener('change', onMq);
  // A tap outside the menu closes it; so does Escape (before it closes the sheet)
  d.addEventListener('click', (e) => {
    if (menuOpen && !(e.target as Element | null)?.closest('.vw-menu, .vw-more')) {
      menuOpen = false;
      draw();
    }
  });
  d.addEventListener('cancel', (e) => {
    if (!menuOpen) return;
    e.preventDefault();
    menuOpen = false;
    draw();
    document.getElementById('vw-more')?.focus();
  });
  d.addEventListener('close', () => {
    phoneMq?.removeEventListener('change', onMq);
    d.remove();
    document.getElementById(`ais-view-${u.id}`)?.focus();
  });
  draw();
  document.body.append(d);
  if (typeof d.showModal === 'function') d.showModal();
  else d.setAttribute('open', '');
  d.querySelector<HTMLElement>('.vw-close')?.focus();
  await loadDetail();
}
