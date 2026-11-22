// AI Chat (#/ai, v0.8). main.ts opens it in mode 'full' when the user may chat now, 'readOnly' when AI+ or AI is off but
// the user has chats or memories (reading and every delete stay; nothing new can be written), else "Not allowed".
//   · An always-visible line under the title says that administrators can view and delete the user's AI chats and
//     memories and see their usage (no pop-up, never dismissible).
//   · v2.3: entering AI Chat opens a NEW chat on every width (a chat named in the address, or a running one,
//     wins; a read-only user opens the latest chat). The sidebar (search, New chat, Memory, the history grouped by day, Delete
//     all) sits left of the conversation from 720 px and is a drawer under 720 px (the ☰ button of the chat header). The
//     Memory row opens the memory view (notes the AI uses in every chat) in place of the conversation.
//     Wider than 900 px the owner's fixed-height box; up to 900 px (v1.0) a full-height chat screen: the document is locked
//     (html.no-doc-scroll), only the lists scroll, the composer is a fixed footer lifted by the on-screen keyboard (--kb),
//     and the page never slides up when the keyboard opens (CH-13); the messages stay still too, the list only gets shorter
//     from the bottom (v1.2, A-OWNER-88, watchList).
//   · Answers stream in (server-sent events over a POST, api/client.ts postStream). The page draws itself in place while an
//     answer streams (only the answer being written is redrawn, at most every 100 ms); render() of the whole page happens
//     only on navigation and menu changes. The composer element is kept across those, so the text, the caret and an input
//     method's composition survive; Enter never sends while an input method is composing (domain/ai-chat.ts shouldSend).
//   · Answers keep being written on the server when the page is left or the connection drops: the page reads the chat again
//     every 2 s until it is finished (a lost stream, the tab coming back, a reload while an answer runs). Stop needs no id.
//   · Assistant text is a small Markdown subset drawn with text nodes only (components/ai-render.ts).
//   · Round 2: every send, retry and memory suggestion carries the interface language of Settings (the AI replies in it);
//     a reasoning model's thinking shows "Thinking…" until its first text
//     (the free mode is a Settings choice for administrators); a suspended AI+ function makes the page read only.
//   · v1.3: the conversation is the path of a branch tree. Under an answer: Copy, Regenerate (a new version, still named Retry
//     for a failed or stopped last answer) and the switcher `< n/total >`; under a question: Copy, Remember, Edit (the text goes
//     into the composer under a banner, Send starts a new branch) and the switcher. Switching versions calls no model, so it also
//     works read only. Administrators get a model menu in the composer (stored per chat) and see the connection of each answer;
//     everyone else sees only "AI" and the time, never a model or connection (v1.3 sprint 2).
import { githubLine } from '../components/github-line.ts';
import '../styles/ai.css';
import {
  addMemory, deleteAllChats, deleteAllMemories, deleteChat, deleteMemory, editMessage, fetchChat, fetchChats, fetchMemories, regenerate,
  sendMessage, setChatModel, stopAnswer, suggestMemory, switchBranch, updateMemory,
} from '../api/ai.ts';
import { fetchAiSettings, fetchModels } from '../api/ai-admin.ts';
import { modelRefText, parseModelRef } from '../api/model-ref.ts';
import { ApiError, isAbortError, type StreamEvent, type StreamOptions } from '../api/client.ts';
import { badgeReasonText } from '../components/ai-badge.ts';
import { fillMarkdown, markdownEl } from '../components/ai-render.ts';
import { iconButton } from '../components/icon-button.ts';
import { miniIcon, type MiniIcon } from '../components/mini-icons.ts';
import { holdTip } from '../components/tooltip.ts';
import { EFFORT_PARAM } from '../components/provider-icons.ts';
import {
  drawerBack, entryTarget, filterChats, groupChats, type ChatGroup, localDayKey, latestChat, chatAddress, DRAWER_ENTRY, staleDrawerEntry, applyEvent, atEnd, beginEdit, canEdit, canRegenerate, chatOf, clearsComposer, composerLayout, composerLines, composerStatus, editValid, emptyChat, END_PX, fmtAiTime,
  footerOf, reasoningView, ignoreSendClick, isTerminal, localEdit, localFail, localRegenerate, chatShowsModel, chatMenu, chatShown, chooseValue, modelArg, pickerView, effortArg, keepEffort, strengthView, EFFORT_DEFAULT, type StrengthView, regenerateLabelKey, startOf, versionInfo, whoParts,
  isBusyElsewhere, isThinking, localSend, localStop, LOCK_MAX_WIDTH, locksDocument, MIN_MSGS_PX, noAccessKey, pollResult, readOnlyReason, questionOf,
  sendBody, shouldSend, showsEmptyHint, showsTwoPanes, TWO_PANES_MIN_WIDTH, type ChatDefault, type ChatMenu, type ChatView, type EditState, type ViewMessage,
} from '../domain/ai-chat.ts';
import { chatDisplayTitle, popTitleKey } from '../domain/ai-plus.ts';
import { memoryBoxChars, memoryLength, memoryTokenCount, wantsMemory } from '../domain/ai-memory.ts';
import { featureOffKey } from '../domain/ai-off.ts';
import { shownCode } from '../domain/ai-settings.ts';
import { aiCodeText, callLimitText, errorText } from '../i18n/errors.ts';
import { localeTag, t, tn, uiLang } from '../i18n/index.ts';
import { clockText } from '../i18n/wait.ts';
import { parseHash } from '../router.ts';
import { onRouteChange, onSignedIn, refreshAi, render } from '../state/actions.ts';
import { aiFreeMode, aiStatus, callBlock, noteCallBlock, onCallBlockChange, recheckCallBlock } from '../state/ai.ts';
import { S, isAdmin, onClearUserData } from '../state/store.ts';
import type { AiCallBlock, AiChatKind, AiChatSummary, AiMemory, AiMessage, AiModelList, AiSettings, ModelRef } from '../types/api.ts';
import { forgetStudyChats as forgetPanelChats } from '../components/ai-panel.ts';
import { forgetStudyChats as forgetSummaryChats } from '../components/ai-summary.ts';
import { copyText } from '../utils/clipboard.ts';
import { ask } from '../utils/dialog.ts';
import { h, keepFocus, undoViewportPan, type Child } from '../utils/dom.ts';
import { liftToastAbove, toast } from '../utils/toast.ts';

export type ChatMode = 'full' | 'readOnly';

const POLL_MS = 2000;
const PAINT_MS = 100;
const ico = (name: MiniIcon, cls = 'ai-mi'): SVGSVGElement => miniIcon(name, cls);

// ---------------------------------------------------------------- page-local state (kept while the app runs)

let mode: ChatMode = 'full';
let tab: 'chats' | 'memory' = 'chats';
/** The page was set up since it was opened (the chats are read, a running answer re-attached) */
let entered = false;

let chats: AiChatSummary[] | null = null;
let chatsError: unknown = null;
let chatsLoading = false;
let chatsSeq = 0;

let view: ChatView = emptyChat();
/** A chat is being opened */
let opening = false;
let openError: unknown = null;
/** v2.3 phones: the history drawer is open (the sidebar slides in from the left) */
let drawerOpen = false;
/** The drawer pushed one history entry (so the system back gesture closes it instead of leaving AI Chat) */
let drawerPushed = false;
/** A history.back() made by this page to drop that entry is under way: its popstate is not a user's back */
let drawerPopping = false;
let drawerAfter: (() => void) | null = null;
let drawerTimer = 0;
/** A read-only user cannot start a chat: once the list is read, the most recent chat opens (view only); none: an empty state */
let enterLatest = false;
/** The height (px) of the conversation card on a wide screen, taken when the memory view opens, so that view keeps the layout */
let memH = 0;

/** Why the last send did not start (shown above the composer; the text stays) */
let sendError: string | null = null;
/** The running stream of this page (null: none, or detached) */
let stream: AbortController | null = null;
let pollTimer: number | null = null;
let polling = false;
let paintTimer: number | null = null;
let lastPaint = 0;
/** The conversation follows the newest text (it is scrolled to the end) */
let stick = true;
/** An input method is composing in the composer (cleared one tick after compositionend) */
let composing = false;
/** When the Send button last turned into Stop (Date.now()): a click right after it is part of the click that sent */
let stopSince = 0;
/** v1.3: the question being edited in the composer (the draft the composer held is kept for Cancel) */
let edit: EditState | null = null;
/** v1.3, administrators: the models of the working connections (the composer's picker), and the picked value (`<conn>:<model>`, '' = follow the default) */
let modelList: AiModelList | null = null;
let modelsLoading = false;
let modelSel = '';
/** v2.1: the picked thinking-strength level ('' = none picked), and the AI settings the strength step reads (levels, routes) */
let effortSel = '';
/** v2.2: the strength menu (the button beside the brain button) is open */
let effortPop = false;
let aiSetAt = 0;
let aiSet: AiSettings | null = null;
/** v1.9: the picker's popover is open; the connection chosen in its first list before a model is chosen (null = none); when the
 *  model list was last asked for (Date.now(), 0 = never) */
let modelPop = false;
let pendingConn: number | null = null;
let modelsAt = 0;
/** The last model-list read failed (cleared when a read starts or succeeds) */
let modelsFailed = false;
/** A version switch is under way */
let switching = false;
/** v2.3 phones: the ⋯ menu of the chat header (Delete this chat) is open */
let headMenu = false;
/** v2.3: what the sidebar's search box holds (kept while the page lives; the box itself is never redrawn while typing) */
let searchQ = '';
/** v2.3: the open ⋯ menu of a history row: the chat and where the (fixed) menu sits, set from the button when it opens */
let rowMenu: { id: number; top: number | null; bottom: number | null; right: number } | null = null;

interface Composer {
  form: HTMLFormElement;
  input: HTMLTextAreaElement;
  label: HTMLLabelElement;
  count: HTMLElement;
  btn: HTMLButtonElement;
  /** The status slot above the input (CH-7): a send error, the request-cap callout, or "another chat is busy" */
  status: HTMLElement;
  latest: HTMLButtonElement;
  latestText: HTMLElement;
  /** v1.3: the banner above the box while a question is edited */
  banner: HTMLElement;
  /** The one rounded box: the input on top, one bar under it (the model button on the left; the counter and Send on the right) */
  box: HTMLElement;
  /** The administrators' model picker (hidden for everyone else): the brain button and, while open, its popover */
  models: HTMLElement;
  /** v1.9: the brain button that opens the picker's popover, and the dot on it (a model other than the default is chosen; amber when its connection is off) */
  modelBtn: HTMLButtonElement;
  modelDot: HTMLElement;
  /** v2.2: the strength button beside the brain button (its text is the current choice) and the menu wrapper it opens */
  effortBtn: HTMLButtonElement;
  effortLbl: HTMLElement;
  effortHost: HTMLElement;
  /** The fixed footer (CH-13): the form, and the administrator notice line under it */
  dock: HTMLElement;
  notice: HTMLElement;
}
/** Kept across renders of the page, so its text, caret, undo history and composition survive */
let composer: Composer | null = null;
/** What the status slot shows now (it is redrawn only when this changes, so an alert is announced once) */
let statusSig = '';
/** Copy feedback on the icon (CH-5): message id -> 'ok' | 'fail' for 1.5 s */
const copyState = new Map<number, 'ok' | 'fail'>();
/** v1.3: the reader's own open / closed choice of each reasoning block (by message id); cleared on a chat switch */
const reasoningOpen = new Map<number, boolean>();
const COPY_MS = 1500;

let memories: AiMemory[] | null = null;
let memError: unknown = null;
let memLoading = false;
let memWanted = true;
let memDraft = '';
let memBusy = false;
let memFormError: string | null = null;
let memEdit: { id: number; text: string } | null = null;

/** "Save to memory?" under an answer: the question it is for, the proposed note, and its state */
let suggestion: { messageId: number; text: string; busy: boolean; error: string | null } | null = null;

/** The open chat is in the address (#/ai/<id>; a new chat not sent yet is #/ai/new), so a reload opens it again
 *  (owner's report); replaceState, so the back button is not filled with chats and no route change runs. v2.3: always a
 *  replace (a phone's back button leaves AI Chat like any page; there is no list step to go back to) */
function setChatInUrl(id: number | 'new' | null): void {
  const want = chatAddress(id, full());
  if (location.hash === want) return;
  history.replaceState(history.state, '', want);
}

/** One column on a phone (up to 719 px): the sidebar is a drawer there; from 720 px it is the left column */
const onePane = (): boolean => phoneScreen() && !showsTwoPanes(window.innerWidth);

// ---------------------------------------------------------------- the phone drawer (v2.3)

const drawerEl = (): HTMLElement | null => document.getElementById('ai-side');

/** Draws the drawer's state: the sidebar's class, the backdrop, the menu button's aria-expanded */
function applyDrawer(): void {
  const open = drawerOpen && onePane();
  const side = drawerEl();
  side?.classList.toggle('open', open);
  const back = document.getElementById('ai-backdrop');
  if (back) back.hidden = !open;
  document.getElementById('ai-drawer-btn')?.setAttribute('aria-expanded', String(open));
}

function openDrawer(): void {
  if (drawerOpen || !onePane()) return;
  drawerOpen = true;
  headMenu = false;
  // One history entry (the same address, marked as the drawer's): the system back gesture closes the drawer (drawerBack)
  history.pushState(DRAWER_ENTRY, '', location.hash);
  drawerPushed = true;
  paint();
  applyDrawer();
  // The focus goes to the drawer itself, never to the search box (no keyboard pops up on a phone)
  drawerEl()?.focus({ preventScroll: true });
}

/**
 * Closes the drawer; `after` runs once the pushed history entry is gone (a picked chat changes the address, which must not
 * race the history.back() that drops the entry). The focus returns to the menu button first, so an action may move it on.
 */
function closeDrawer(after?: () => void, focusBack = true): void {
  const wasOpen = drawerOpen;
  drawerOpen = false;
  applyDrawer();
  if (wasOpen && focusBack) document.getElementById('ai-drawer-btn')?.focus({ preventScroll: true });
  if (!drawerPushed) {
    after?.();
    return;
  }
  drawerPushed = false;
  drawerPopping = true;
  drawerAfter = after ?? null;
  // popstate normally finishes this; the timer covers a browser that never fires it
  drawerTimer = window.setTimeout(finishDrawerPop, 600);
  history.back();
}

/** The entry under the drawer's may carry an older address than the open chat now has (a new chat got its id meanwhile, or
 *  the open chat was deleted while the drawer was open and the screen is a new draft): the address follows the screen again,
 *  so dropping the drawer's entry never switches chats nor brings a deleted one back */
function reassertUrl(): void {
  if (tab === 'chats') setChatInUrl(view.chatId ?? 'new');
}

function finishDrawerPop(): void {
  window.clearTimeout(drawerTimer);
  if (!drawerPopping) return;
  drawerPopping = false;
  reassertUrl();
  const after = drawerAfter;
  drawerAfter = null;
  after?.();
}

/** The drawer's page state is dropped without touching the history (the page is left or the screen got wide) */
function resetDrawer(): void {
  window.clearTimeout(drawerTimer);
  drawerOpen = false;
  drawerPushed = false;
  drawerPopping = false;
  drawerAfter = null;
}

/** A left swipe of more than 60 px on the drawer closes it */
let swipeX = 0;
let swipeY = 0;
let swiping = false;
function onDrawerTouchStart(e: TouchEvent): void {
  const t0 = e.touches.length === 1 ? e.touches[0] : undefined;
  swiping = !!t0;
  if (t0) {
    swipeX = t0.clientX;
    swipeY = t0.clientY;
  }
}
function onDrawerTouchEnd(e: TouchEvent): void {
  const t0 = swiping ? e.changedTouches[0] : undefined;
  swiping = false;
  if (!t0 || !drawerOpen) return;
  const dx = t0.clientX - swipeX;
  const dy = t0.clientY - swipeY;
  if (dx < -60 && Math.abs(dx) > Math.abs(dy)) closeDrawer();
}

/** Tab stays inside the open drawer */
function trapTab(e: KeyboardEvent): void {
  const side = drawerEl();
  if (!side) return;
  const items = Array.from(side.querySelectorAll<HTMLElement>('button, input, [href], [tabindex]:not([tabindex="-1"])'))
    .filter((el) => !el.hasAttribute('disabled') && el.offsetParent !== null);
  if (!items.length) return;
  const first = items[0]!;
  const last = items[items.length - 1]!;
  const at = document.activeElement;
  if (e.shiftKey && (at === first || at === side)) {
    e.preventDefault();
    last.focus();
  } else if (!e.shiftKey && at === last) {
    e.preventDefault();
    first.focus();
  } else if (!side.contains(at)) {
    e.preventDefault();
    first.focus();
  }
}

/** The chat the address asks for: an id, 'new' for a new chat not sent yet, or null when it names none (plain #/ai) */
function chatInUrl(): number | 'new' | null {
  const r = parseHash(location.hash);
  if (r.name !== 'ai') return null;
  if (r.chatId != null) return r.chatId;
  return r.fresh ? 'new' : null;
}

/** Codes after which the page reads the AI status again: AI+ switched off, taken away or the AI+ function suspended, or
 *  ai-service no longer connected to the main service (the page then follows: read only) */
const ACCESS_CODES: readonly string[] = ['ai.disabled', 'ai.notAllowed', 'ai.suspended', 'ai.backendDown'];

const onPage = (): boolean => S.route.name === 'ai' && !S.authView && S.phase === 'ready';
const full = (): boolean => mode === 'full';
const sending = (): boolean => stream != null || view.pending != null || view.streamingId != null || view.running;

/** The user's limits from the AI status (the service's current safety limits) */
function limits(): { messageChars: number; memoryTokens: number; memories: number } {
  const st = aiStatus();
  if (st.state !== 'ready') return { messageChars: 16000, memoryTokens: 1000, memories: 50 };
  return { messageChars: st.messageChars ?? st.maxMessageChars, memoryTokens: st.memoryTokens ?? st.memoryMaxTokens, memories: st.memoryLimit };
}

function resetAll(): void {
  detachStream();
  stopPolling();
  watchBusy(false);
  tab = 'chats';
  enterLatest = false;
  memH = 0;
  entered = false;
  chats = null;
  chatsError = null;
  chatsLoading = false;
  chatsSeq += 1;
  view = emptyChat();
  opening = false;
  openError = null;
  sendError = null;
  stick = true;
  composer = null;
  statusSig = '';
  edit = null;
  modelList = null;
  modelsLoading = false;
  modelSel = '';
  effortSel = '';
  aiSet = null;
  modelPop = false;
  effortPop = false;
  pendingConn = null;
  modelsAt = 0;
  modelsFailed = false;
  switching = false;
  modelSig = '';
  copyState.clear();
  reasoningOpen.clear();
  dockObserver?.disconnect();
  dockObserver = null;
  lockDocument(false);
  headMenu = false;
  resetDrawer();
  memories = null;
  memError = null;
  memLoading = false;
  memWanted = true;
  memDraft = '';
  memBusy = false;
  memFormError = null;
  memEdit = null;
  suggestion = null;
}

onRouteChange((from, to) => {
  // #/ai/<id> typed or followed while AI Chat is open: open that chat
  if (from.name === 'ai' && to.name === 'ai' && to.chatId != null && to.chatId !== view.chatId) void openChat(to.chatId);
  if (from.name === 'ai' && to.name === 'ai' && to.fresh && view.chatId != null) newChat();
  if (from.name === 'ai' && to.name !== 'ai') {
    lockDocument(false);
    headMenu = false;
    resetDrawer();
    // Coming back enters as usual (a new chat), also from the memory view
    tab = 'chats';
    enterLatest = false;
    // The answer keeps being written on the server; coming back re-attaches to it
    detachStream();
    stopPolling();
    watchBusy(false);
    entered = false;
    modelPop = false;
    effortPop = false;
    pendingConn = null;
    sendError = null;
    memEdit = null;
    memFormError = null;
  }
});
// Signing out (or another user): nothing of this user stays in memory
onClearUserData(resetAll);
onSignedIn(() => {
  entered = false;
  chatsError = null;
  memError = null;
});

let installed = false;
function installOnce(): void {
  if (installed) return;
  installed = true;
  // Back to the tab while an answer was being written: the stream may have died while the tab was hidden
  document.addEventListener('visibilitychange', () => {
    if (document.visibilityState === 'visible' && onPage() && (view.streamingId != null || view.running) && !polling) void checkAttached();
    // The day groups of the history follow the date (the list signature carries it)
    if (document.visibilityState === 'visible' && onPage()) paint();
  });
  // Leaving #/ai: the document scrolls again (also covers a route change that did not go through onRouteChange)
  // v2.3: the phone's back gesture with the drawer open closes the drawer (the entry it pushed); otherwise it leaves AI Chat
  // like any page (the router follows the hash). A back made by this page to drop the entry is not the user's
  window.addEventListener('popstate', () => {
    if (drawerPopping) {
      finishDrawerPop();
      return;
    }
    if (!onPage() || parseHash(location.hash).name !== 'ai') {
      resetDrawer();
      return;
    }
    // Forward reached the entry a closed drawer left behind: step back over it (the entry does nothing)
    if (staleDrawerEntry({ drawerOpen, ownMove: false, state: history.state })) {
      drawerPopping = true;
      drawerAfter = null;
      drawerTimer = window.setTimeout(finishDrawerPop, 600);
      history.back();
      return;
    }
    if (drawerBack({ drawerOpen }) === 'close-drawer') {
      drawerPushed = false;
      drawerOpen = false;
      reassertUrl();
      applyDrawer();
      document.getElementById('ai-drawer-btn')?.focus({ preventScroll: true });
    }
  });
  window.addEventListener('hashchange', () => {
    if (parseHash(location.hash).name !== 'ai') lockDocument(false);
  });
  // Crossing 900 px (a tablet turned): the page is drawn again with or without the lock; crossing 720 px it is drawn again
  // with the sidebar as a drawer or as the left column (the drawer is dropped, history untouched but for its entry)
  if (typeof window.matchMedia === 'function') {
    window.matchMedia(LOCK_MQ).addEventListener('change', () => {
      if (onPage()) render();
      else lockDocument(false);
    });
    window.matchMedia(`(max-width: ${TWO_PANES_MIN_WIDTH - 0.02}px)`).addEventListener('change', () => {
      if (!onPage()) return;
      if (drawerOpen && !onePane()) {
        const had = drawerPushed;
        resetDrawer();
        if (had) {
          drawerPopping = true;
          drawerTimer = window.setTimeout(finishDrawerPop, 600);
          history.back();
        }
      }
      render();
    });
  }
  // A-OWNER-83: the title row is measured again when the width changes and once the web fonts are in
  window.addEventListener('resize', queueFitHead);
  void document.fonts?.ready.then(queueFitHead);
  // The request cap came or went (C-6): the callout, Send, Retry and Remember follow without a reload
  onCallBlockChange(() => {
    if (!onPage()) return;
    if (tab === 'chats') paintMsgs();
    syncComposer();
  });
  // The chat header's ⋯ menu: a tap outside it or Esc closes it
  document.addEventListener('pointerdown', (e) => {
    if (!headMenu || !(e.target instanceof Node)) return;
    if (e.target instanceof Element && e.target.closest('.ai-more-wrap')) return;
    headMenu = false;
    if (onPage()) paint();
  });
  // A history row's ⋯ menu: a press outside it, Esc, or a resize closes it
  document.addEventListener('pointerdown', (e) => {
    if (!rowMenu || !(e.target instanceof Node)) return;
    if (e.target instanceof Element && e.target.closest('.ai-row-menu, .ai-chat-more')) return;
    closeRowMenu(false);
  });
  window.addEventListener('resize', () => closeRowMenu(false));
  document.addEventListener('keydown', (e) => {
    if (e.key !== 'Escape' || !rowMenu || !onPage()) return;
    // The Esc that closes the menu is not the drawer's (the drawer's handler runs after this one)
    e.stopImmediatePropagation();
    closeRowMenu(true);
  });
  // The model picker's popover: a press outside it or Esc closes it (the focus returns to the brain button)
  document.addEventListener('pointerdown', (e) => {
    if (!modelPop || !(e.target instanceof Node)) return;
    if (e.target instanceof Element && e.target.closest('#ai-model-pick')) return;
    closeModelPop(true);
  });
  // The strength menu: a press outside it or Esc closes it (Esc returns the focus to its button)
  document.addEventListener('pointerdown', (e) => {
    if (!effortPop || !(e.target instanceof Node)) return;
    if (e.target instanceof Element && e.target.closest('#ai-effort-wrap')) return;
    closeEffortPop(false);
  });
  document.addEventListener('keydown', (e) => {
    if (e.key !== 'Escape' || !effortPop || !onPage() || e.isComposing) return;
    closeEffortPop(true);
  });
  document.addEventListener('keydown', (e) => {
    if (e.key !== 'Escape' || !modelPop || !onPage() || e.isComposing) return;
    closeModelPop(true);
  });
  document.addEventListener('keydown', (e) => {
    if (e.key !== 'Escape' || !headMenu || !onPage()) return;
    headMenu = false;
    paint();
    document.getElementById('ai-chat-more')?.focus();
  });
  // The drawer: Esc closes it (unless a popover of the composer is open: that Esc is theirs); Tab stays inside it
  document.addEventListener('keydown', (e) => {
    if (!drawerOpen || !onPage() || e.isComposing) return;
    // A confirm dialog opened from the drawer is modal: its keys (Tab, Esc) are its own
    if (document.querySelector('dialog[open]')) return;
    if (e.key === 'Escape') {
      e.preventDefault();
      closeDrawer();
    } else if (e.key === 'Tab') {
      trapTab(e);
    }
  });
}

// ---------------------------------------------------------------- the phone screen (CH-1, CH-13)

/** The document is locked up to this width: the one media query of the full-height chat screen */
const LOCK_MQ = `(max-width: ${LOCK_MAX_WIDTH}px)`;
const phoneScreen = (): boolean => locksDocument(window.innerWidth);

/** Observes the composer's footer: its height is what the conversation pane reserves (--composer-h) */
let dockObserver: ResizeObserver | null = null;
/** Observes the message list: when its height changes (keyboard, composer growth) and it was at the end, it stays at the end */
let listObserver: ResizeObserver | null = null;
let panWatching = false;
let panFrame = 0;

/**
 * A box with less than this to scroll (px) counts as fixed. v1.2 (A-OWNER-82): on touch screens the GitHub line's invisible
 * tap area (shell.css .gh-link::after) stuck 2 px out of the bottom of the history list, so a list of a few chats was a real
 * scroller with 2 px of travel: the guard saw "can scroll", let the drag through, and iOS Safari dragged and bounced the whole
 * box (rows, empty area, GitHub line) with the finger. ai.css now keeps that tap area inside the scrollers; this keeps any
 * such sliver (rounding, a font) from turning a fixed box into a scroller again.
 */
const MIN_TRAVEL = 4;

/** True when `el` scrolls on `axis` with real travel and can still move in the direction of the drag (d > 0: content follows down / right) */
function canScroll(el: Element, axis: 'x' | 'y', d: number): boolean {
  const cs = getComputedStyle(el);
  const o = axis === 'y' ? cs.overflowY : cs.overflowX;
  if (o !== 'auto' && o !== 'scroll') return false;
  const [pos, view, size] = axis === 'y' ? [el.scrollTop, el.clientHeight, el.scrollHeight] : [el.scrollLeft, el.clientWidth, el.scrollWidth];
  if (size < view + MIN_TRAVEL) return false;
  return d === 0 || (d > 0 ? pos > 0 : pos + view < size - 1);
}

/** True when a scroller from `target` up to the document can follow the drag (`d` 0: has travel at all, whatever its position) */
function followable(target: EventTarget | null, axis: 'x' | 'y', d: number): boolean {
  for (let n = target instanceof Element ? target : null; n && n !== document.documentElement; n = n.parentElement) {
    if (canScroll(n, axis, d)) return true;
  }
  return false;
}

let touchY = 0;
let touchX = 0;
let startY = 0;
let startX = 0;
let touchGuarding = false;
/** v1.2 (A-OWNER-82): the gesture started outside the conversation view (memory view, title row, top bar there); phones are
 *  always in the conversation (`.conv-on`) except in the memory view, and the drawer follows the conversation's rule */
let frameGesture = false;

function onGuardStart(e: TouchEvent): void {
  const t = e.touches.length === 1 ? e.touches[0] : undefined;
  if (!t) return;
  touchY = startY = t.clientY;
  touchX = startX = t.clientX;
  // The conversation view (phones: the whole screen except the memory view, drawer included; tablets: the conversation pane) keeps the v1.1 rule
  const inConv = e.target instanceof Element && !!e.target.closest('.ai-conv');
  // The sidebar (the phone drawer, the tablet / desktop column) is a fixed frame like the memory view: strict rule
  const inSide = e.target instanceof Element && !!e.target.closest('#ai-side');
  frameGesture = !inConv && (inSide || !document.querySelector('.page-ai.conv-on'));
}

/**
 * While the document is locked (phones), a drag may only scroll a row list; the screen never drags, bounces or pans (iOS
 * Safari rubber-bands even under overflow: hidden). Pinch (two fingers) and taps are left alone.
 *
 * The conversation view keeps the v1.1 rule unchanged (owner, A-OWNER-82 constraint): a mostly vertical move that no
 * scroller under the finger can still follow is cancelled.
 *
 * The memory screen and, from 720 px, the sidebar (v1.2, A-OWNER-82; before v2.3 the phone's history list) are fixed frames: v1.1 let through every move with no vertical
 * part or a bigger sideways part, and so the first move of most swipes; once Safari starts its own pan on a move nobody
 * cancels, the later moves of that gesture are not cancelable. Here every move, the first one included, is judged by the
 * whole path from the touch start and cancelled unless a scroller can follow it. Sideways jitter at the start of a swipe on
 * a list with rows to scroll counts as vertical, so it never blocks that list. ai.css adds touch-action: pinch-zoom there.
 */
function onGuardMove(e: TouchEvent): void {
  const t = e.touches.length === 1 ? e.touches[0] : undefined;
  if (!t || !e.cancelable) return;
  const dy = t.clientY - touchY;
  const dx = t.clientX - touchX;
  touchY = t.clientY;
  touchX = t.clientX;
  if (!frameGesture) {
    if (Math.abs(dx) > Math.abs(dy) || dy === 0) return;
    if (!followable(e.target, 'y', dy)) e.preventDefault();
    return;
  }
  const cx = t.clientX - startX;
  const cy = t.clientY - startY;
  const listUnder = followable(e.target, 'y', 0);
  const side = Math.abs(cx) > Math.abs(cy) && (!listUnder || (Math.abs(cx) >= 6 && Math.abs(cx) > 2 * Math.abs(cy)));
  if (side ? followable(e.target, 'x', dx || cx) : cy === 0 ? listUnder : followable(e.target, 'y', dy || cy)) return;
  e.preventDefault();
}

function setTouchGuard(on: boolean): void {
  if (on === touchGuarding) return;
  touchGuarding = on;
  if (on) {
    document.addEventListener('touchstart', onGuardStart, { passive: true });
    document.addEventListener('touchmove', onGuardMove, { passive: false });
  } else {
    document.removeEventListener('touchstart', onGuardStart);
    document.removeEventListener('touchmove', onGuardMove);
  }
}

/**
 * Locks the document (html.no-doc-scroll: only the lists scroll) or frees it. Every AI Chat render at up to 900 px locks it;
 * leaving the page, or growing wider than 900 px, frees it.
 */
function lockDocument(on: boolean): void {
  const el = document.documentElement;
  el.classList.toggle('no-doc-scroll', on);
  setTouchGuard(on);
  if (on) return;
  el.style.removeProperty('--composer-h');
  listObserver?.disconnect();
  listObserver = null;
  stopPanWatch();
}

function syncComposerHeight(): void {
  const el = document.documentElement;
  if (!composer || !el.classList.contains('no-doc-scroll')) return;
  el.style.setProperty('--composer-h', `${Math.ceil(composer.dock.offsetHeight)}px`);
}

/**
 * Keeps the newest message in view when the list's size changes and the list was at its end (no smooth scroll).
 * v1.2 (A-OWNER-88): on the full-height phone screen (html.no-doc-scroll) a change of the list's HEIGHT alone (the iOS
 * keyboard opening or closing, the composer growing while typing, its safe-area padding switching) never moves the text:
 * the list keeps its scrollTop and top edge and only its visible height changes at the bottom, so the bottom part is
 * covered until the user swipes (the list still scrolls to its very end, right above the composer). A new message or a
 * streaming reply still follows the end (paintMsgs / paintStreaming / toBottom). A change of WIDTH (rotation, the side
 * pane) re-flows the text, so there the end is kept as before; wider than 900 px every change keeps the end (v1.1).
 */
function watchList(): void {
  listObserver?.disconnect();
  listObserver = null;
  const el = scroller();
  if (!el || typeof ResizeObserver === 'undefined') return;
  let width = el.clientWidth;
  listObserver = new ResizeObserver(() => {
    const w = el.clientWidth;
    const reflow = w !== width;
    width = w;
    if (!stick) return;
    if (reflow || !document.documentElement.classList.contains('no-doc-scroll')) el.scrollTop = el.scrollHeight;
  });
  listObserver.observe(el);
}

function onViewportMove(): void {
  if (panFrame) return;
  panFrame = window.requestAnimationFrame(() => {
    panFrame = 0;
    undoViewportPan();
    if (composer) autoGrow(composer.input);
  });
}

/** While the composer has the focus: undo the pan at once, on every viewport change and 300 ms after the focus */
function startPanWatch(): void {
  undoViewportPan();
  window.setTimeout(undoViewportPan, 300);
  if (panWatching) return;
  panWatching = true;
  window.visualViewport?.addEventListener('resize', onViewportMove);
  window.visualViewport?.addEventListener('scroll', onViewportMove);
}

function stopPanWatch(): void {
  if (panWatching) {
    panWatching = false;
    window.visualViewport?.removeEventListener('resize', onViewportMove);
    window.visualViewport?.removeEventListener('scroll', onViewportMove);
  }
  // Once the keyboard has gone
  window.setTimeout(undoViewportPan, 300);
}

/**
 * v1.2 (A-OWNER-83): the title row of the 720-900 px screen (the h1 and the free-mode tag) never overlaps. The row is measured
 * after it is drawn: when the h1 is cut the tag shows its short form; the title ellipsises as the last resort (CSS). (Phones
 * under 720 px have no title row: the chat header takes its place, v2.3.)
 */
function fitHead(): void {
  const head = document.querySelector<HTMLElement>('.page-ai .ai-head');
  if (!head) return;
  head.classList.remove('fit-short');
  if (!document.documentElement.classList.contains('no-doc-scroll') || head.offsetParent === null) return;
  // scrollWidth rounds up and clientWidth down: 1 px is the rounding of a box that fits exactly
  const over = (e: Element | null): boolean => !!e && e.scrollWidth > e.clientWidth + 1;
  const cut = (): boolean => over(head.querySelector('.ai-title')) || over(head.querySelector('.ai-title h1'));
  if (!cut()) return;
  head.classList.add('fit-short');
}

let fitFrame = 0;
function queueFitHead(): void {
  if (fitFrame || !onPage()) return;
  fitFrame = window.requestAnimationFrame(() => {
    fitFrame = 0;
    fitHead();
  });
}

// ---------------------------------------------------------------- loading

async function loadChats(): Promise<void> {
  chatsLoading = true;
  const mine = ++chatsSeq;
  try {
    const list = await fetchChats();
    if (mine !== chatsSeq) return;
    chats = list;
    chatsError = null;
  } catch (e) {
    if (mine !== chatsSeq) return;
    chatsError = e;
  } finally {
    if (mine === chatsSeq) chatsLoading = false;
  }
  if (onPage()) paint();
  // A read-only user entered with no chat open: the most recent chat opens now (view only)
  if (enterLatest && mine === chatsSeq && chats) {
    enterLatest = false;
    const latest = latestChat(chats);
    if (onPage() && !full() && view.chatId == null && latest && !stream && chatInUrl() !== 'new') void openChat(latest.id);
  }
}

async function loadMemories(): Promise<void> {
  memWanted = false;
  memLoading = true;
  try {
    const r = await fetchMemories();
    memories = r.memories;
    memError = null;
  } catch (e) {
    memError = e;
  } finally {
    memLoading = false;
  }
  if (onPage() && tab === 'memory') render();
}

/** First draw after the page was opened: read the chats, and re-attach to an answer that is still being written */
function enter(): void {
  entered = true;
  if (!chatsLoading) void loadChats();
  // coming back to the page: the AI settings (the Chat row's strength) may have changed meanwhile
  void loadModels(modelList != null);
  memWanted = true;
  const st = aiStatus();
  const running = st.state === 'ready' ? st.running : null;
  const wanted = chatInUrl();
  // v2.3: a chat named in the address, else the running chat, else a NEW chat on every width (never a list)
  const target = entryTarget({ running: running ? running.chatId : null, hash: wanted == null ? 'none' : wanted });
  if (stream) return;
  if (target.kind === 'chat') {
    if (view.chatId !== target.id) void openChat(target.id);
    else if (view.running || view.streamingId != null) startPolling();
  } else if (!full()) {
    // Read only (paused, or the feature is off): nothing new can be written, so no new-chat draft. The latest chat opens once
    // the list is read (view only); with none the page shows its paused line and an empty state
    if (view.chatId != null) setChatInUrl(view.chatId);
    else {
      enterLatest = true;
      setChatInUrl(null);
    }
  } else if (view.chatId != null) {
    queueMicrotask(() => newChat());
  } else {
    // A fresh draft (or one kept while the app ran): the address says so, no route change runs
    setChatInUrl('new');
  }
}

// ---------------------------------------------------------------- the conversation

function detachStream(): void {
  const s = stream;
  stream = null;
  s?.abort();
  // The answer keeps running on the server: read the status again so "another chat is answering" shows at once
  if (s) void refreshAi(true).then(() => {
    if (onPage()) syncComposer();
  });
}

function stopPolling(): void {
  polling = false;
  if (pollTimer != null) window.clearTimeout(pollTimer);
  pollTimer = null;
}

/** Reads the chat every 2 s until its answer is finished (the stream was lost, or the answer runs elsewhere) */
function startPolling(): void {
  if (polling || view.chatId == null) return;
  polling = true;
  void pollTick();
}

async function pollTick(): Promise<void> {
  pollTimer = null;
  const id = view.chatId;
  if (!polling || id == null || !onPage()) {
    polling = false;
    return;
  }
  try {
    const d = await fetchChat(id, true);
    if (!polling || view.chatId !== id) return;
    const wasRunning = view.running || view.streamingId != null;
    view = pollResult(view, d);
    paint();
    if (!view.running) {
      polling = false;
      if (wasRunning) announceEnd(view.messages[view.messages.length - 1]);
      void loadChats();
      return;
    }
  } catch (e) {
    if (e instanceof ApiError && e.status === 404) {
      // The chat was deleted meanwhile (another tab, an administrator, the retention)
      polling = false;
      goneChat();
      paint();
      return;
    }
    if (e instanceof ApiError && (e.status === 401 || e.status === 403)) {
      polling = false;
      return;
    }
  }
  if (polling && onPage()) pollTimer = window.setTimeout(() => void pollTick(), POLL_MS);
  else polling = false;
}

/** The tab came back while an answer was being written: is the stream still alive? */
async function checkAttached(): Promise<void> {
  const id = view.chatId;
  if (id == null) return;
  try {
    const d = await fetchChat(id, true);
    if (view.chatId !== id) return;
    if (!d.chat.running) {
      // Finished while the tab was hidden; a stream that is still open has nothing more to say
      detachStream();
      view = pollResult(view, d);
      paint();
      announceEnd(view.messages[view.messages.length - 1]);
      void loadChats();
    } else if (!stream) {
      startPolling();
    }
  } catch {
    /* the next look tries again */
  }
}

const pickText = (p: ModelRef | null): string => (p ? modelRefText(p) : '');

/** Ends an edit: the composer gets its earlier draft back (not when the edited message was sent) */
function cancelEdit(restore = true): void {
  const e = edit;
  if (!e) return;
  edit = null;
  if (restore && composer) {
    composer.input.value = e.draft;
    autoGrow(composer.input);
  }
  if (onPage()) paintMsgs();
  syncComposer();
}

/** Back to the conversation from the memory view (the sidebar's rows and New chat are shown beside both) */
function showChats(): void {
  if (tab === 'chats') return;
  tab = 'chats';
  memEdit = null;
  memFormError = null;
  render();
}

async function openChat(id: number): Promise<void> {
  showChats();
  detachStream();
  stopPolling();
  cancelEdit();
  closeModelPop(false);
  suggestion = null;
  sendError = null;
  openError = null;
  const summary = chats?.find((c) => c.id === id);
  view = { ...emptyChat(), chatId: id, title: summary?.title ?? '', kind: summary?.kind ?? 'chat' };
  setChatInUrl(id);
  opening = true;
  stick = true;
  paint();
  try {
    const d = await fetchChat(id);
    if (view.chatId !== id) return;
    view = chatOf(d);
    modelSel = pickText(view.pick);
    effortSel = view.pickEffort ?? '';
    if (view.running) startPolling();
  } catch (e) {
    if (view.chatId !== id) return;
    openError = e;
    if (e instanceof ApiError && e.status === 404) {
      // The chat is gone (deleted, another device, an administrator, the retention): a real new-chat draft, so sending works
      goneChat();
    }
  } finally {
    if (view.chatId === id) opening = false;
  }
  if (!onPage()) return;
  stick = true;
  paint();
  toBottom();
}

/** The open chat no longer exists (a 404): the screen becomes a clean new-chat draft (nothing points at the deleted id any
 *  more, sending works) with a short toast; read-only users get the empty state */
function goneChat(): void {
  detachStream();
  stopPolling();
  view = emptyChat();
  modelSel = '';
  effortSel = '';
  openError = null;
  opening = false;
  suggestion = null;
  sendError = null;
  setChatInUrl('new');
  toast(t('ai.chat.gone'));
  void loadChats();
}

function newChat(): void {
  showChats();
  detachStream();
  stopPolling();
  cancelEdit();
  closeModelPop(false);
  view = emptyChat();
  modelSel = '';
  effortSel = '';
  setChatInUrl('new');
  opening = false;
  openError = null;
  suggestion = null;
  sendError = null;
  stick = true;
  paint();
  announce(t('ai.new'));
  // New chat keeps the focus on the composer (CH-9), but not on a touch phone: no keyboard popping up by itself (v2.3)
  if (window.matchMedia('(pointer: fine)').matches) composer?.input.focus();
}

function announce(text: string): void {
  const sr = document.getElementById('ai-sr');
  if (!sr) return;
  sr.textContent = '';
  window.setTimeout(() => {
    sr.textContent = text;
  }, 60);
}

function reasonText(code: string): string {
  return aiCodeText(shownCode(code, isAdmin()));
}

/** What the screen reader hears when an answer ends */
function announceEnd(m: ViewMessage | undefined): void {
  if (!m || m.role !== 'assistant') return;
  const f = footerOf(m);
  if (!f) announce(t('ai.status.done'));
  else announce(footerText(m) ?? t('ai.status.done'));
}

function onStreamEvent(ev: StreamEvent): void {
  const before = view;
  view = applyEvent(view, ev);
  switch (ev.event) {
    case 'start': {
      // The message is saved: only now the composer is cleared. Not for a retry: its question is already in the chat, and
      // the composer may hold the next question the user is writing
      if (clearsComposer(before, ev)) clearComposer();
      const sd = startOf(ev.data);
      // A sent edit is done with its banner (the earlier draft is dropped with the text that replaced it)
      if (sd?.mode === 'edit') cancelEdit(false);
      modelSel = pickText(view.pick);
    effortSel = view.pickEffort ?? '';
      if (sd?.modelFallback) toast(t('ai.model.fallback'));
      sendError = null;
      // A new chat now has its id: put it in the address, so a reload keeps it open
      setChatInUrl(view.chatId);
      stick = true;
      announce(t('ai.writing'));
      paint();
      toBottom();
      void refreshAi(true);
      void loadChats();
      return;
    }
    case 'delta':
      schedulePaint();
      return;
    case 'reasoning':
      schedulePaint();
      return;
    case 'reasoningDone':
      // The block collapses as the answer starts (the reader's own toggle of the live block is dropped)
      if (view.streamingId != null) reasoningOpen.delete(view.streamingId);
      paintStreaming();
      return;
    case 'thinking': {
      // Once per answer, before its first text: "Thinking…" under it (and for screen readers)
      const id = view.streamingId;
      const was = before.messages.find((m) => m.id === id)?.thinking === true;
      const now = view.messages.find((m) => m.id === id)?.thinking === true;
      if (now && !was) {
        paintStreaming();
        announce(t('ai.thinking'));
      }
      return;
    }
    default:
      if (!isTerminal(ev.event)) return;
  }
  const done = before.streamingId;
  paint();
  const msg = done != null ? view.messages.find((m) => m.id === done) : undefined;
  announceEnd(msg);
  void loadChats();
  // AI switched off, AI+ taken away or the AI+ function suspended meanwhile: the page follows (read only, or "Not allowed")
  if (msg?.status === 'error' && msg.error != null && ACCESS_CODES.includes(msg.error)) void refreshAi(true);
  // The question asked to remember something: offer a note (saved only when the user saves it)
  if (ev.event === 'done' && full() && done != null) {
    const q = questionOf(view, done);
    if (q && wantsMemory(q.content) && !callBlock()) void suggest(q.id);
  }
  // The first chat makes the user one "with AI data" (the menu keeps AI Chat even if AI+ is switched off later)
  void refreshAi(true);
}

async function runStream(call: (o: StreamOptions) => Promise<void>, begin: (v: ChatView) => ChatView): Promise<void> {
  if (sending()) return;
  sendError = null;
  view = begin(view);
  if (suggestion && view.pending?.retry !== true) suggestion = null;
  const ctrl = new AbortController();
  stream = ctrl;
  let started = false;
  let ended = false;
  stick = true;
  syncComposer();
  try {
    await call({
      signal: ctrl.signal,
      onEvent: (ev) => {
        if (stream !== ctrl) return;
        // Signed out meanwhile: stop listening (the answer goes on on the server)
        if (S.authView) {
          detachStream();
          return;
        }
        if (ev.event === 'start') started = true;
        if (isTerminal(ev.event)) ended = true;
        onStreamEvent(ev);
      },
    });
    if (stream === ctrl && started && !ended) startPolling();
  } catch (e) {
    if (stream !== ctrl || isAbortError(e)) return;
    if (!started) {
      // Nothing was saved: the text stays in the composer, the reason above it
      view = localFail(view);
      if (e instanceof ApiError && e.code === 'ai.busy') {
        // Another answer runs: read the status, so "another chat is writing [Stop it]" shows (watchBusy clears it when it
        // ends). Running in this very chat (another tab or device): follow it here, with Stop in the Send slot
        sendError = null;
        void refreshAi(true).then(() => {
          if (!onPage()) return;
          const st = aiStatus();
          if (st.state === 'ready' && st.running && st.running.chatId === view.chatId && !stream) startPolling();
          syncComposer();
        });
      } else if (e instanceof ApiError && e.quota) {
        // A request cap: the callout in the status slot says it (not a red send error); the text stays in the composer
        sendError = null;
        noteCallBlock(e.quota);
        announce(errorText(e));
      } else {
        sendError = errorText(e);
      }
      if (e instanceof ApiError && e.code != null && ACCESS_CODES.includes(e.code)) void refreshAi(true);
      if (e instanceof ApiError && e.code === 'ai.branchMoved') {
        // Another tab switched the branch: this page shows the chat as the server has it now
        sendError = null;
        void reloadChat(true);
      }
      // The pick stays (an off connection is the administrator's to fix); the list is read again so the picker tells the truth
      if (e instanceof ApiError && (e.code === 'ai.badModel' || e.code === 'ai.connectionOff')) void loadModels(true);
      if (e instanceof ApiError && e.code === 'ai.notFound' && view.chatId != null) {
        view = emptyChat();
        void loadChats();
      }
      paint();
    } else {
      // The stream was lost (a phone switching apps, a tunnel blip): the answer goes on; read the chat until it is done
      startPolling();
    }
  } finally {
    if (stream === ctrl) stream = null;
    // Remember / Retry follow the end of the send
    if (onPage() && tab === 'chats') paintMsgs();
    syncComposer();
  }
}

/** The text of the cap that blocks AI calls now (for the tip, the toast and the callout) */
const capText = (q: AiCallBlock): string => callLimitText(q, isAdmin(), Date.now() / 1000);

/** The cap callout in the composer: one short line pair (CH-7 allows 2 lines); capText() is its tip and what a reader hears */
function capShort(q: AiCallBlock): string {
  const cap = t(q.cap === 'minute' ? 'ai.cap.minute' : q.cap === 'hour' ? 'ai.cap.hour' : 'ai.cap.day', { n: q.limit.toLocaleString(localeTag()) });
  const left = Math.max(1, Math.ceil(q.retryAt - Date.now() / 1000));
  const when = q.cap === 'day' ? t('ai.cap.shortDay') : left < 60 ? t('ai.cap.shortSoon') : t('ai.cap.shortWhen', { time: clockText(q.retryAt, Date.now() / 1000) });
  return t(isAdmin() ? 'ai.cap.shortAdmin' : 'ai.cap.short', { cap, when });
}

/**
 * Runs `action` unless the request cap blocks AI calls (C-6): then the tap first asks the server again (an administrator may
 * have raised the limits); still blocked, the cap text is the answer (a toast), free again, the action runs once.
 */
function guarded(action: () => void): () => void {
  return () => {
    if (!callBlock()) {
      action();
      return;
    }
    void recheckCallBlock().then((still) => {
      const q = callBlock();
      if (still && q) toast(capText(q), 6000);
      else if (onPage()) action();
    });
  };
}

/** A control that needs a model call, while the cap blocks calls: aria-disabled (never `disabled`), the cap text as its tip */
function applyCap(btn: HTMLButtonElement): void {
  const q = callBlock();
  if (!q || !full()) return;
  btn.setAttribute('aria-disabled', 'true');
  btn.setAttribute('aria-describedby', 'ai-cap-note');
  holdTip(btn, capText(q));
}

function send(): void {
  if (!full() || !composer || sending()) return;
  const text = composer.input.value.trim();
  if (!text) return;
  if (callBlock()) {
    guarded(send)();
    return;
  }
  const max = limits().messageChars;
  if (memoryLength(text) > max) {
    sendError = aiCodeText('ai.tooLong', max);
    syncComposer();
    return;
  }
  // The reply language is the interface language of Settings; free mode is the administrator's state on the server
  const lang = uiLang();
  const chatId = view.chatId;
  const model = modelToSend();
  const effort = effortToSend();
  const ed = edit;
  // v2.4 (owner): on a touch screen sending closes the keyboard, so the conversation gets the whole screen back; the box
  // shrinks to its one-row form when the sent text leaves it (the "start" event). A mouse / keyboard user keeps the focus.
  if (window.matchMedia('(pointer: coarse)').matches) composer.input.blur();
  if (ed && chatId != null && editValid(view, ed)) {
    void runStream((o) => withModel(chatId, model, effort, (m) => editMessage(chatId, ed.messageId, text, lang, o, m, effort)),
      (v) => localEdit(v, ed.messageId, text, model, undefined, effort));
    return;
  }
  const body = sendBody({ chatId, text, lang, model, effort });
  void runStream((o) => sendMessage(body, o), (v) => localSend(v, text, false, model, effort));
}

/** The model this request carries (administrators only): undefined = unchanged, null = back to the default */
function modelToSend(): ModelRef | null | undefined {
  return isAdmin() && full() && aiFreeMode() ? modelArg(modelSel, view.pick, parseModelRef) : undefined;
}

/** The strength level this request carries (administrators only): undefined = unchanged, null = no pick */
function effortToSend(): string | null | undefined {
  return isAdmin() && full() && aiFreeMode() ? effortArg(effortSel, view.pickEffort) : undefined;
}

/** Runs a call that carries the model; going back to the default is a call of its own first (the regenerate / edit bodies
 *  only carry a model reference) */
async function withModel(chatId: number, model: ModelRef | null | undefined, effort: string | null | undefined, go: (m?: ModelRef) => Promise<void>): Promise<void> {
  if (model === null) await setChatModel(chatId, null, effort);
  return go(model ?? undefined);
}

/** Regenerate: a new version of this answer (a failed or stopped last answer is the old Retry) */
function regen(m: ViewMessage): void {
  const chatId = view.chatId;
  if (!full() || chatId == null || !canRegenerate(view, m)) return;
  const lang = uiLang();
  const model = modelToSend();
  const effort = effortToSend();
  // The button goes away when the answer starts: keep the focus on the composer (it stays focused while an answer streams)
  if (document.activeElement instanceof HTMLElement && document.activeElement.classList.contains('ai-regen')) composer?.input.focus({ preventScroll: true });
  cancelEdit();
  void runStream((o) => withModel(chatId, model, effort, (mm) => regenerate(chatId, m.id, lang, o, mm, effort)), (v) => localRegenerate(v, m.id, model, effort));
}

/** Edit: the question goes into the composer under a banner; Send then starts a new branch from it */
function startEdit(m: ViewMessage): void {
  if (!full() || !composer || sending()) return;
  const e = beginEdit(view, m, edit ? edit.draft : composer.input.value);
  if (!e) return;
  edit = e;
  sendError = null;
  composer.input.value = m.content;
  autoGrow(composer.input);
  paintMsgs();
  syncComposer();
  composer.input.focus();
  const end = composer.input.value.length;
  try {
    composer.input.setSelectionRange(end, end);
  } catch {
    // no caret to move
  }
}

/** Reads the chat again (another tab changed the branch) and says so */
async function reloadChat(said: boolean): Promise<void> {
  const id = view.chatId;
  if (id == null) return;
  try {
    const d = await fetchChat(id);
    if (view.chatId !== id || sending()) return;
    cancelEdit();
    view = chatOf(d);
    modelSel = pickText(view.pick);
    effortSel = view.pickEffort ?? '';
    stick = true;
    paint();
    toBottom();
    if (view.running) startPolling();
    if (said) toast(t('ai.branch.moved'));
  } catch {
    // the page keeps what it shows; the next poll or open reads again
  }
}

/** The version switcher: shows the branch that ends at version `toId`; no model call; the message stays where it was on screen */
async function switchTo(fromId: number, toId: number): Promise<void> {
  const chatId = view.chatId;
  if (chatId == null || sending() || switching) return;
  switching = true;
  const sc = scroller();
  const el = document.getElementById(`ai-m-${fromId}`);
  const anchor = el && sc ? el.getBoundingClientRect().top - sc.getBoundingClientRect().top : null;
  try {
    const d = await switchBranch(chatId, toId);
    if (view.chatId !== chatId || sending()) return;
    cancelEdit();
    suggestion = null;
    view = chatOf(d);
    modelSel = pickText(view.pick);
    effortSel = view.pickEffort ?? '';
    stick = false;
    paint();
    const sc2 = scroller();
    const el2 = document.getElementById(`ai-m-${toId}`);
    if (anchor != null && sc2 && el2) sc2.scrollTop += el2.getBoundingClientRect().top - sc2.getBoundingClientRect().top - anchor;
    syncLatest();
    void loadChats();
  } catch (e) {
    if (e instanceof ApiError && e.status === 404) {
      void reloadChat(false);
    } else {
      toast(errorText(e), 5000);
    }
  } finally {
    switching = false;
    if (onPage() && tab === 'chats') paintMsgs();
  }
}

/** The models of the working connections for the administrators' menu (read from the settings; the page shows none for others) */
async function loadModels(force = false): Promise<void> {
  if (!isAdmin() || !full() || !aiFreeMode() || modelsLoading || (modelList && !force)) return;
  modelsLoading = true;
  modelsFailed = false;
  try {
    // The strength step needs the families and levels; without them the picker still works (no strength step)
    const [list, set] = await Promise.all([fetchModels(false), fetchAiSettings().catch(() => null)]);
    modelList = list;
    aiSet = set;
    aiSetAt = Date.now();
  } catch {
    modelList = null;
    modelsFailed = true;
  } finally {
    modelsAt = Date.now();
    modelsLoading = false;
    if (onPage()) syncComposer();
  }
}

async function stop(): Promise<void> {
  const hadStart = view.streamingId != null;
  const chatId = view.chatId;
  detachStream();
  view = localStop(view);
  paint();
  announce(t('ai.status.stopped'));
  try {
    await stopAnswer(chatId);
  } catch {
    /* the answer may have just ended; the chat is read again below */
  }
  if (!onPage()) return;
  if (!hadStart) void loadChats();
  // The server saves the text written up to the stop: read it
  if (view.chatId != null) startPolling();
}

/** "Stop it" in the status slot: stops the answer that runs in another chat (no id), then reads the status again */
async function stopElsewhere(): Promise<void> {
  try {
    await stopAnswer(null);
  } catch {
    /* the answer may have just ended */
  }
  await refreshAi(true);
  if (onPage()) syncComposer();
}

/** Brings the memory suggestion bar into the free area of the conversation scroller (only its scrollTop moves, never the page) */
function revealSuggest(): void {
  const sc = scroller();
  const bar = sc?.querySelector<HTMLElement>('.ai-suggest');
  if (!sc || !bar) return;
  const s = sc.getBoundingClientRect();
  const b = bar.getBoundingClientRect();
  // The fixed composer covers the bottom of the scroller on phones: keep clear of it
  const dock = document.getElementById('ai-composer');
  const dockTop = dock && getComputedStyle(dock).position === 'fixed' ? dock.getBoundingClientRect().top : Infinity;
  const bottom = Math.min(s.bottom, dockTop);
  if (b.bottom > bottom) sc.scrollTop += b.bottom - bottom + 8;
  if (b.top < s.top) sc.scrollTop -= s.top - b.top + 8;
}

async function suggest(messageId: number): Promise<void> {
  suggestion = { messageId, text: '', busy: true, error: null };
  paintMsgs();
  revealSuggest();
  try {
    const text = await suggestMemory(messageId, uiLang());
    if (suggestion?.messageId !== messageId) return;
    suggestion = { messageId, text, busy: false, error: null };
    paintMsgs();
    revealSuggest();
    return;
  } catch (e) {
    if (suggestion?.messageId !== messageId) return;
    if (e instanceof ApiError && e.quota) {
      // A request cap: the callout in the composer's status slot says it; no suggestion bar
      suggestion = null;
      noteCallBlock(e.quota);
      announce(errorText(e));
      if (onPage()) paintMsgs();
      return;
    }
    suggestion = { messageId, text: '', busy: false, error: errorText(e) };
    if (e instanceof ApiError && e.code != null && ACCESS_CODES.includes(e.code)) void refreshAi(true);
  }
  if (onPage()) {
    paintMsgs();
    revealSuggest();
  }
}

async function saveSuggestion(): Promise<void> {
  const s = suggestion;
  if (!s || s.busy) return;
  const text = s.text.trim();
  if (!text) return;
  suggestion = { ...s, busy: true, error: null };
  paintMsgs();
  try {
    const m = await addMemory(text, 'chat');
    if (memories) memories = [m, ...memories];
    suggestion = null;
    toast(t('ai.mem.saved'));
    void refreshAi(true);
  } catch (e) {
    if (suggestion) suggestion = { ...s, busy: false, error: errorText(e) };
  }
  if (onPage()) paintMsgs();
}

/** The deleted chats' saved hints / explanations / summaries must not be offered as saved any more (null = every chat) */
function forgetStudyChats(chatId: number | null): void {
  forgetPanelChats(chatId);
  forgetSummaryChats(chatId);
}

// v2.4 (owner): whether the last input was a tap / click (true) or a key (false). After a delete the focus moves to the next
// row only for a keyboard user; a tap or click would get a blue focus ring around a row it never touched.
let pointerInput = true;
if (typeof document !== 'undefined') {
  document.addEventListener('pointerdown', () => { pointerInput = true; }, true);
  document.addEventListener('keydown', () => { pointerInput = false; }, true);
}

/** After a delete: a keyboard user's focus goes to `target`; after a tap or click the drawer keeps it (no ring) or, on a
 *  wide screen, nothing is focused */
function focusAfterDelete(target: HTMLElement | null): void {
  if (!pointerInput) {
    target?.focus({ preventScroll: true });
    return;
  }
  if (drawerOpen) drawerEl()?.focus({ preventScroll: true });
  else if (document.activeElement instanceof HTMLElement && document.activeElement !== document.body) document.activeElement.blur();
}

/** A confirm dialog was cancelled: the focus goes back to the control that started it when that is gone (its menu closed) */
function focusAfterCancel(...ids: string[]): void {
  window.setTimeout(() => {
    if (!onPage() || (document.activeElement && document.activeElement !== document.body)) return;
    for (const id of ids) {
      const el = document.getElementById(id);
      if (el && el.offsetParent !== null) {
        el.focus({ preventScroll: true });
        return;
      }
    }
    if (drawerOpen) drawerEl()?.focus({ preventScroll: true });
  }, 0);
}

async function removeChat(c: Pick<AiChatSummary, 'id' | 'title' | 'kind'>): Promise<void> {
  const title = chatTitleText(c);
  if (!(await ask({ title: t('ai.delete.title'), body: t('ai.delete.ask', { title }), ok: t('ai.delete'), danger: true }))) {
    focusAfterCancel(`ai-cmenu-${c.id}`, 'ai-chat-more', 'ai-del');
    return;
  }
  const idx = chats?.findIndex((x) => x.id === c.id) ?? -1;
  let deleted = false;
  try {
    await deleteChat(c.id);
    deleted = true;
    forgetStudyChats(c.id);
    toast(t('ai.delete.done'));
    if (view.chatId === c.id) {
      detachStream();
      stopPolling();
      view = emptyChat();
      suggestion = null;
      setChatInUrl('new');
    }
  } catch (e) {
    toast(errorText(e), 6000);
  }
  await loadChats();
  void refreshAi(true);
  if (deleted && onPage()) {
    // The row is gone: the focus goes to the next row (else the previous one, else New chat) and the result is announced
    const rows = Array.from(document.querySelectorAll<HTMLElement>('.ai-chats .ai-chat-open'));
    const target = rows[Math.min(Math.max(idx, 0), rows.length - 1)] ?? (drawerOpen ? document.getElementById('ai-side-new') : null) ?? document.getElementById('ai-new');
    focusAfterDelete(target);
    announce(t('ai.delete.done'));
  }
}

async function removeAllChats(): Promise<void> {
  const n = chats?.length ?? 0;
  if (!n) return;
  if (!(await ask({ title: t('ai.deleteAll.title'), body: tn('ai.deleteAll.ask', n), ok: t('ai.deleteAll'), danger: true }))) {
    focusAfterCancel('ai-delall');
    return;
  }
  try {
    await deleteAllChats();
    forgetStudyChats(null);
    detachStream();
    stopPolling();
    view = emptyChat();
    suggestion = null;
    setChatInUrl('new');
    toast(t('ai.delete.done'));
  } catch (e) {
    toast(errorText(e), 6000);
  }
  await loadChats();
  void refreshAi(true);
  // The foot button is gone: the focus goes to New chat (in the drawer when it is open)
  if (onPage()) focusAfterDelete(document.getElementById(drawerOpen ? 'ai-side-new' : 'ai-new') ?? drawerEl());
}

// ---------------------------------------------------------------- scrolling

/** The element that scrolls the conversation: its own box (a fixed-height box on desktop, the free space of the full-height
 *  screen on phones); null only while the conversation pane is not drawn */
function scroller(): HTMLElement | null {
  return document.getElementById('ai-scroll');
}

function nearEnd(): boolean {
  const el = scroller();
  return el ? atEnd(el, END_PX) : true;
}

function toBottom(): void {
  const el = scroller();
  if (el) el.scrollTop = el.scrollHeight;
  stick = true;
  syncLatest();
}

function syncLatest(): void {
  if (!composer) return;
  composer.latest.hidden = stick || view.messages.length === 0 || tab !== 'chats';
}

function onScroll(): void {
  if (!onPage() || tab !== 'chats') return;
  stick = nearEnd();
  syncLatest();
}

// ---------------------------------------------------------------- drawing in place

/** The nodes of a list of children (as h() would append them) */
function nodes(kids: readonly Child[]): Node[] {
  const out: Node[] = [];
  const add = (k: Child): void => {
    if (k == null || k === false) return;
    if (Array.isArray(k)) k.forEach(add);
    else out.push(k instanceof Node ? k : document.createTextNode(String(k)));
  };
  kids.forEach(add);
  return out;
}

/** What the chat list shows: the list is redrawn only when this changes (a poll repaints every 2 s while an answer runs;
 *  redrawing the rows each time would stop a fling and swap the row under a finger) */
let listSig = '';
function listSignature(): string {
  const st = aiStatus();
  return JSON.stringify([uiLang(), full(), tab, onePane(), localDayKey(new Date()), rowMenu, chatsError != null ? errorText(chatsError) : null, view.chatId, isThinking(view), st.state,
    st.state === 'ready' ? st.retentionDays : null, chats?.map((c) => [c.id, c.title, c.kind, c.running, c.updatedAt]) ?? null]);
}

function paint(): void {
  const panes = document.getElementById('ai-panes');
  if (!panes) return;
  keepFocus(() => {
    const lb = document.getElementById('ai-list-body');
    const sig = listSignature();
    if (lb && sig !== listSig) {
      listSig = sig;
      // Redrawing the list must not throw the reader back to the top
      const keep = lb.scrollTop;
      const keepUl = lb.querySelector('.ai-chats')?.scrollTop ?? 0;
      refreshListHead();
      lb.replaceChildren(...nodes(listBody()));
      if (keep) lb.scrollTop = keep;
      const ul = lb.querySelector('.ai-chats');
      if (ul && keepUl) ul.scrollTop = keepUl;
    }
    // The memory view has no conversation to draw
    if (tab !== 'chats') return;
    document.getElementById('ai-conv-head')?.replaceChildren(...nodes(convHead()));
    paintMsgs();
  });
  if (tab === 'chats') syncComposer();
}

/**
 * The title of a chat as shown (v0.9): the stored title, plus the translated suffix of a study chat ("<time>-练习"); an
 * AI Chat conversation without a title is "New chat".
 */
export function chatTitleText(c: { title: string; kind?: AiChatKind | null }): string {
  const { text, suffixKey } = chatDisplayTitle(c);
  return suffixKey ? `${text}-${t(suffixKey)}` : text || t('ai.new');
}

/** The title of a chat in a list */
export function chatTitleEl(c: { title: string; kind?: AiChatKind | null }): HTMLElement {
  return h('span', { class: 'ai-chat-t' }, chatTitleText(c));
}

/** v2.5: a user message that asked for the AI study assessment (the wire kind `assess`) */
const isAssessMessage = (m: AiMessage): boolean => m.role === 'user' && m.kind === 'assess';

/**
 * What a user message shows (v0.9): an AI Chat message is its own bubble; a study question (hint / explanation) is a label
 * line ("AI hint · question 3") above the bubble; a summary request is only its label ("AI summary · 8 of 9 wrong answers"),
 * never the machine text the model was given.
 */
export function userBody(m: AiMessage): Child[] {
  // v2.5: the study assessment request is only its title, never the machine line the model was given
  if (isAssessMessage(m)) return [h('p', { class: 'ai-study-label' }, t('dash.ai.title'))];
  if (m.kind === 'summary') {
    const sm = m.sample;
    return [sm ? h('p', { class: 'ai-study-label' }, t('aiplus.chat.label.summary', { used: sm.used, wrong: sm.wrong })) : null];
  }
  const bubble = h('div', { class: 'ai-bubble' }, h('div', { class: 'ai-text' }, m.content));
  if (m.kind === 'hint' || m.kind === 'explain') {
    const label = t(popTitleKey(m.kind, m.source ?? 'practice'), { n: m.questionNo ?? '' });
    return [h('p', { class: 'ai-study-label' }, label), bubble];
  }
  return [bubble];
}

function paintMsgs(): void {
  const box = document.getElementById('ai-msgs-wrap');
  if (!box) return;
  const follow = stick;
  const sc = scroller();
  const top = sc?.scrollTop ?? 0;
  keepFocus(() => box.replaceChildren(...nodes(msgsBody())));
  lastPaint = Date.now();
  if (follow) toBottom();
  else if (sc) sc.scrollTop = top;
  syncLatest();
}

function schedulePaint(): void {
  if (paintTimer != null) return;
  const wait = Math.max(0, PAINT_MS - (Date.now() - lastPaint));
  paintTimer = window.setTimeout(() => {
    paintTimer = null;
    paintStreaming();
  }, wait);
}

/** Redraws only the answer being written (its text and status line), then follows it when the view is at the end */
function paintStreaming(): void {
  lastPaint = Date.now();
  const id = view.streamingId;
  const m = id != null ? view.messages.find((x) => x.id === id) : undefined;
  if (!m) return;
  const follow = stick;
  const md = document.getElementById(`ai-md-${m.id}`);
  const foot = document.getElementById(`ai-foot-${m.id}`);
  if (!md || !foot) {
    paintMsgs();
    return;
  }
  const rv = reasoningView(m, reasoningOpen.get(m.id));
  const rs = document.getElementById(`ai-rs-${m.id}`);
  if (rv && !rs) {
    paintMsgs();
    return;
  }
  if (rs && !rv) {
    rs.remove();
  } else if (rs && rv) {
    const tog = rs.querySelector('.ai-rs-tog');
    const body = rs.querySelector<HTMLElement>('.ai-rs-body');
    const same = rs.dataset.live === String(rv.live) && tog?.getAttribute('aria-expanded') === String(rv.open)
      && (rs.querySelector('.ai-rs-cut') != null) === rv.truncated
      && rs.querySelector('.ai-rs-label')?.textContent === reasoningLabel(rv);
    if (!same) {
      const fresh = reasoningEl(m, reasoningOpts(m));
      if (fresh) keepFocus(() => rs.replaceWith(fresh));
    } else {
      const box = document.getElementById(`ai-rs-${m.id}-text`);
      if (box && body && box.textContent !== rv.text) {
        const wasAtEnd = atEnd(body);
        box.textContent = rv.text;
        if (wasAtEnd) body.scrollTop = body.scrollHeight;
      }
    }
  }
  fillMarkdown(md, m.content);
  // "Thinking…" turns into "Writing…" with the first text
  const text = footerText(m);
  if (text != null && foot.textContent !== text) foot.textContent = text;
  if (foot.hidden !== (text == null)) foot.hidden = text == null;
  paintListRunning();
  if (follow) toBottom();
}

/** The open chat's row in the list says the same as its answer: "Thinking…" while a reasoning model thinks, then
 *  "Writing…" (the list is otherwise drawn only when it is loaded, which may happen before the first "thinking" event) */
function paintListRunning(): void {
  const id = view.chatId;
  const c = id != null ? chats?.find((x) => x.id === id) : undefined;
  const at = c?.running ? document.querySelector(`#ai-c-${c.id} .ai-chat-at`) : null;
  if (!c || !at) return;
  const want = t(listThinking(c) ? 'ai.thinking' : 'ai.writing');
  if (at.textContent !== want) at.textContent = want;
}

// ---------------------------------------------------------------- the chat list

/** The chat's running answer is a reasoning model still thinking (no text yet): the list says what the message says */

function listThinking(c: AiChatSummary): boolean {
  return c.id === view.chatId && isThinking(view);
}

/** The sidebar's scroller: the rows, the foot (Delete all, the retention note), then the GitHub line as its last item (the
 *  header rows stay above it) */
function listBody(): Child[] {
  return [...listRows(), listFoot(), githubLine()];
}

/** An action of the sidebar on a phone first closes the drawer (once its history entry is gone), then runs */
function viaDrawer(run: () => void): void {
  if (drawerOpen) closeDrawer(run);
  else run();
}

/** The memory view (the sidebar's Memory row; v2.3 replaces the Chats / Memory tabs) */
function openMemory(): void {
  const go = (): void => {
    if (tab === 'memory') return;
    // Wide screens: the memory card takes the height the conversation card has now, so the layout does not jump
    const conv = document.querySelector('.ai-conv');
    memH = conv && !phoneScreen() ? Math.round(conv.getBoundingClientRect().height) : 0;
    tab = 'memory';
    memEdit = null;
    render();
    document.getElementById('ai-mem-h')?.focus({ preventScroll: true });
  };
  if (drawerOpen) closeDrawer(go, false);
  else go();
}

/** The sidebar's search box: filters the history's titles live. Only the list body is redrawn while typing, so the box keeps
 *  its focus, caret and input method state */
function searchBox(): HTMLElement {
  const label = t('ai.search.ph');
  const input = h('input', {
    type: 'search', class: 'ai-search-in', id: 'ai-search', placeholder: label, 'aria-label': label, autocomplete: 'off', autocapitalize: 'off',
    spellcheck: 'false', enterkeyhint: 'search',
    oninput: () => {
      searchQ = input.value;
      rowMenu = null;
      const lb = document.getElementById('ai-list-body');
      if (!lb) return;
      lb.replaceChildren(...nodes(listBody()));
      lb.scrollTop = 0;
    },
  });
  input.value = searchQ;
  return h('div', { class: 'ai-search', id: 'ai-search-wrap' }, ico('search'), input);
}

/** The top of the sidebar, above its scroller (so the scroller's edge never cuts its buttons' tap margins): the search box,
 *  New chat, the Memory row */
function listHead(): HTMLElement {
  // On a phone the chat header owns #ai-new; the sidebar's button is then #ai-side-new
  const newBtn = full()
    ? h('button', {
      class: 'btn ai-new', id: onePane() ? 'ai-side-new' : 'ai-new', type: 'button', 'aria-label': t('ai.new'),
      onclick: () => viaDrawer(() => newChat()),
    }, ico('plus'), h('span', { class: 'ai-new-l', 'aria-hidden': 'true' }, t('ai.new')))
    : h('span', { class: 'ai-list-title' }, t('ai.tab.chats'));
  const on = tab === 'memory';
  const mem = h('button', {
    class: 'ai-side-row' + (on ? ' on' : ''), id: 'ai-mem-open', type: 'button', 'aria-current': on ? 'page' : null,
    onclick: () => openMemory(),
  }, ico('memory'), h('span', null, t('ai.tab.memory')));
  return h('div', { class: 'ai-list-top', id: 'ai-list-head' }, searchBox(), h('div', { class: 'ai-list-head' }, newBtn), mem);
}

/** Redraws the sidebar's top except the search box, which stays in the document (the focus and the caret survive) */
function refreshListHead(): void {
  const top = document.getElementById('ai-list-head');
  if (!top) return;
  const fresh = listHead();
  const keep = top.querySelector('#ai-search-wrap');
  const search = keep?.querySelector<HTMLInputElement>('#ai-search');
  if (search) {
    const label = t('ai.search.ph');
    search.placeholder = label;
    search.setAttribute('aria-label', label);
  }
  if (!keep) {
    top.replaceChildren(...Array.from(fresh.children));
    return;
  }
  const rest = Array.from(fresh.children).filter((c) => c.id !== 'ai-search-wrap');
  Array.from(top.children).filter((c) => c !== keep).forEach((c) => c.remove());
  top.append(...rest);
}

const GROUP_KEY = {
  today: 'ai.group.today', yesterday: 'ai.group.yesterday', week: 'ai.group.week', older: 'ai.group.older',
} as const satisfies Record<ChatGroup, string>;

function listRows(): Child[] {
  const out: Child[] = [];
  if (!chats) {
    out.push(chatsError
      ? h('div', { class: 'err', role: 'alert' }, h('p', null, t('ai.err.load', { reason: errorText(chatsError) })),
        h('div', { class: 'acts' }, h('button', { class: 'btn', type: 'button', onclick: () => void loadChats() }, t('common.retry'))))
      : h('p', { class: 'dim', role: 'status' }, t('common.loading')));
    return out;
  }
  if (!chats.length) {
    out.push(h('p', { class: 'ai-list-note' }, t('ai.empty')));
    return out;
  }
  const shown = filterChats(chats, searchQ, chatTitleText);
  if (!shown.length) {
    out.push(h('p', { class: 'ai-list-note', id: 'ai-search-none', role: 'status' }, t('ai.search.none')));
    return out;
  }
  out.push(h('div', { class: 'ai-chats-wrap' }, groupChats(shown, new Date()).map((g) => [
    h('h3', { class: 'ai-group-h', id: `ai-g-${g.group}` }, t(GROUP_KEY[g.group])),
    h('ul', { class: 'ai-chats', 'aria-labelledby': `ai-g-${g.group}` }, g.chats.map((c) => {
      const title = chatTitleText(c);
      const on = c.id === view.chatId && tab === 'chats';
      return h('li', { class: 'ai-chat' + (on ? ' on' : '') },
        h('button', {
          class: 'ai-chat-open', id: `ai-c-${c.id}`, type: 'button', 'aria-current': on ? 'true' : null,
          onclick: () => viaDrawer(() => void openChat(c.id)),
        },
        chatTitleEl(c),
        h('span', { class: 'ai-chat-at' }, c.running ? t(listThinking(c) ? 'ai.thinking' : 'ai.writing') : fmtAiTime(c.updatedAt, localeTag()))),
        chatMoreButton(c, title),
        rowMenu?.id === c.id ? chatRowMenu(c, title) : null);
    })),
  ])));
  return out;
}

/** The ⋯ button of a history row (44 px hit area on touch, tap-and-hold tip with the name) */
function chatMoreButton(c: AiChatSummary, title: string): HTMLButtonElement {
  const label = t('ai.row.options', { title });
  const open = rowMenu?.id === c.id;
  const btn = iconButton({ icon: 'more', label, onClick: () => toggleRowMenu(c.id) });
  btn.id = `ai-cmenu-${c.id}`;
  btn.classList.add('ai-chat-more');
  btn.setAttribute('aria-haspopup', 'true');
  btn.setAttribute('aria-expanded', String(open));
  if (open) btn.setAttribute('aria-controls', 'ai-row-menu');
  return btn;
}

/** The row's menu: Delete (the same confirm flow as before). It is fixed to the screen, placed from the button when it
 *  opened, so the list's scroller never clips it */
function chatRowMenu(c: AiChatSummary, title: string): HTMLElement {
  const pos = rowMenu;
  const style = pos ? `right:${pos.right}px;${pos.top != null ? `top:${pos.top}px;` : ''}${pos.bottom != null ? `bottom:${pos.bottom}px;` : ''}` : '';
  return h('div', { class: 'ai-menu ai-row-menu', id: 'ai-row-menu', role: 'group', 'aria-label': t('ai.row.options', { title }), style },
    h('button', {
      class: 'btn danger ai-delthis', id: `ai-cdel-${c.id}`, type: 'button', 'aria-label': t('ai.deleteOne.aria', { title }),
      onclick: () => {
        rowMenu = null;
        paint();
        void removeChat(c);
      },
    }, ico('trash'), t('ai.delete')));
}

function toggleRowMenu(id: number): void {
  if (rowMenu?.id === id) {
    closeRowMenu(true);
    return;
  }
  const btn = document.getElementById(`ai-cmenu-${id}`);
  if (!btn) return;
  const r = btn.getBoundingClientRect();
  const room = window.innerHeight - r.bottom >= 76;
  headMenu = false;
  rowMenu = {
    id, right: Math.max(8, Math.round(window.innerWidth - r.right)),
    top: room ? Math.round(r.bottom + 4) : null, bottom: room ? null : Math.round(window.innerHeight - r.top + 4),
  };
  paint();
  document.getElementById(`ai-cdel-${id}`)?.focus({ preventScroll: true });
}

function closeRowMenu(focusBack: boolean): void {
  const id = rowMenu?.id;
  if (id == null) return;
  rowMenu = null;
  if (!onPage()) return;
  paint();
  if (focusBack) document.getElementById(`ai-cmenu-${id}`)?.focus({ preventScroll: true });
}

/** The foot of the sidebar's scroller: Delete all chats (red text, confirm kept) and the retention note */
function listFoot(): HTMLElement {
  const st = aiStatus();
  return h('div', { class: 'ai-side-foot' },
    chats?.length
      ? h('button', { class: 'btn danger ai-delall', id: 'ai-delall', type: 'button', onclick: () => void removeAllChats() }, t('ai.deleteAll'))
      : null,
    st.state === 'ready' ? h('p', { class: 'ai-retention' }, t('ai.retention', { n: st.retentionDays })) : null);
}

/** The sidebar: one function for the phone drawer and the tablet / desktop left column (v2.3). On a phone it is a dialog that
 *  slides in from the left (CSS), on wider screens it is always visible */
function sidebar(): HTMLElement {
  const phone = onePane();
  const side = h('aside', {
    class: 'ai-list' + (phone && drawerOpen ? ' open' : ''), id: 'ai-side', 'aria-label': phone ? t('ai.drawer.open') : t('ai.tab.chats'),
    role: phone ? 'dialog' : null, 'aria-modal': phone ? 'true' : null, tabindex: phone ? '-1' : null,
  },
  listHead(),
  // The footer line is the last item of the list's scroller (listBody), never inside a conversation
  // (The administrator notice is not here: it shows under the composer, and in the memory view)
  h('div', { class: 'ai-list-body', id: 'ai-list-body', onscroll: () => closeRowMenu(false) }, listBody()));
  if (phone) {
    side.addEventListener('touchstart', onDrawerTouchStart, { passive: true });
    side.addEventListener('touchend', onDrawerTouchEnd, { passive: true });
  }
  return side;
}

/** The sidebar and the pane beside it (the conversation or the memory view), and the phone drawer's backdrop */
function panesView(pane: HTMLElement): HTMLElement {
  return h('div', { class: 'ai-panes', id: 'ai-panes' },
    sidebar(),
    pane,
    h('div', { class: 'ai-backdrop', id: 'ai-backdrop', hidden: !(drawerOpen && onePane()), 'aria-hidden': 'true', onclick: () => closeDrawer() }));
}

// ---------------------------------------------------------------- the conversation pane

/** The phone's button that opens the history drawer (chat header and memory header). Its glyph is two lines of unequal
 *  length, never the three lines of the top bar's app menu */
function drawerButton(): HTMLButtonElement {
  const label = t('ai.drawer.open');
  const btn = iconButton({ icon: 'sideLines', label, onClick: () => openDrawer() });
  btn.id = 'ai-drawer-btn';
  btn.classList.add('ai-drawer-btn');
  btn.setAttribute('aria-expanded', String(drawerOpen && onePane()));
  btn.setAttribute('aria-controls', 'ai-side');
  return btn;
}

function convHead(): Child[] {
  // A read-only user cannot start a chat: with none open the title is the page's, not "New chat"
  const title = view.chatId == null ? t(full() ? 'ai.new' : 'ai.title') : chatTitleText(view);
  const c = view.chatId != null ? { id: view.chatId, title: view.title, kind: view.kind } : null;
  const heading = h('h2', { class: 'ai-conv-t', id: 'ai-conv-t', tabindex: '-1', title }, title);
  if (!onePane()) {
    // Tablet / desktop: [title] [red trash icon]; the sidebar beside it has the list
    let del: HTMLButtonElement | null = null;
    if (c) {
      del = iconButton({ icon: 'trash', tone: 'danger', label: t('ai.deleteOne.aria', { title }), onClick: () => void removeChat(c) });
      del.id = 'ai-del';
      del.classList.add('ai-del');
    }
    return [heading, del];
  }
  // Phone, one 44 px row: [menu] [title + the free-mode tag] [new chat] [more]
  const tag = isAdmin() && aiFreeMode()
    ? h('span', { class: 'ai-mode-tag ai-head-tag', role: 'status', title: t('ai.freeTag') },
      h('span', { class: 'sr-only' }, t('ai.freeTag')), h('span', { class: 'ai-mode-s', 'aria-hidden': 'true' }, t('ai.freeTagShort')))
    : null;
  let add: HTMLButtonElement | null = null;
  if (full()) {
    add = iconButton({ icon: 'plus', label: t('ai.new'), onClick: () => newChat() });
    add.id = 'ai-new';
    add.classList.add('ai-new-ic');
  }
  let more: HTMLElement | null = null;
  if (c) {
    const btn = iconButton({
      icon: 'more', label: t('ai.chat.options'),
      onClick: () => {
        headMenu = !headMenu;
        paint();
      },
    });
    btn.id = 'ai-chat-more';
    btn.setAttribute('aria-haspopup', 'true');
    btn.setAttribute('aria-expanded', String(headMenu));
    btn.setAttribute('aria-controls', 'ai-chat-menu');
    const menu = headMenu
      ? h('div', { class: 'ai-menu', id: 'ai-chat-menu', role: 'group', 'aria-label': t('ai.chat.options') },
        h('button', {
          class: 'btn danger ai-delthis', id: 'ai-del', type: 'button',
          onclick: () => {
            headMenu = false;
            paint();
            void removeChat(c);
          },
        }, ico('trash'), t('ai.deleteThis')))
      : null;
    more = h('div', { class: 'ai-more-wrap' }, btn, menu);
  }
  // The spacer pushes [new chat] [more] to the right edge; the title and its tag stay together at the left
  return [drawerButton(), heading, tag, h('span', { class: 'ai-head-sp', 'aria-hidden': 'true' }), add, more];
}

function footerText(m: ViewMessage): string | null {
  const f = footerOf(m);
  if (!f) return null;
  switch (f.kind) {
    case 'thinking':
      return t('ai.thinking');
    case 'writing':
      return t('ai.writing');
    case 'stopped':
      return t('ai.status.stopped');
    case 'error':
      return t('ai.status.error', { reason: reasonText(f.code) });
    case 'noAnswer':
      return t('ai.status.noAnswer', { reason: reasonText(f.code) });
    default:
      return t('ai.status.truncated');
  }
}

function suggestionBar(): HTMLElement | null {
  const s = suggestion;
  if (!s) return null;
  const max = limits().memoryTokens;
  if (s.busy && !s.text) return h('div', { class: 'ai-suggest', role: 'status' }, h('span', { class: 'dim' }, t('ai.mem.suggestBusy')));
  const input = h('input', {
    type: 'text', id: 'ai-suggest-input', class: 'ai-suggest-in', value: s.text, maxlength: String(max), disabled: s.busy,
    'aria-labelledby': 'ai-suggest-l',
    oninput: () => {
      if (suggestion) suggestion = { ...suggestion, text: input.value };
    },
    onkeydown: (e: KeyboardEvent) => {
      if (e.key === 'Enter' && !e.isComposing && e.keyCode !== 229) {
        e.preventDefault();
        void saveSuggestion();
      }
    },
  });
  // The proposed note can be edited (or written by hand when the proposal failed); nothing is saved without Save
  return h('div', { class: 'ai-suggest', role: 'group', 'aria-labelledby': 'ai-suggest-l' },
    h('span', { class: 'ai-suggest-l', id: 'ai-suggest-l' }, miniIcon('sparkle', 'ai-mi'), t('ai.mem.suggest')),
    input,
    s.error ? h('p', { class: 'ai-suggest-err', role: 'alert' }, s.error) : null,
    h('div', { class: 'ai-suggest-acts' },
      h('button', { class: 'btn primary ai-mini', id: 'ai-suggest-save', type: 'button', disabled: s.busy, onclick: () => void saveSuggestion() }, t('ai.mem.save')),
      h('button', {
        class: 'btn quiet ai-mini', id: 'ai-suggest-no', type: 'button', disabled: s.busy,
        onclick: () => {
          suggestion = null;
          paintMsgs();
          composer?.input.focus();
        },
      }, t('ai.mem.dismiss'))));
}

function showCopyState(btn: HTMLElement, st: 'ok' | 'fail' | null): void {
  btn.classList.remove('ok', 'fail');
  if (st) btn.classList.add(st);
  btn.replaceChildren(ico(st === 'ok' ? 'check' : st === 'fail' ? 'xCircle' : 'copy', 'ibtn-ic'));
}

/** Copy: an icon button; the feedback is on the icon (it turns into a check for 1.5 s), no toast; the live region says it (CH-5) */
function copyButton(m: AiMessage, text: string): HTMLElement {
  const state = copyState.get(m.id);
  const btn = iconButton({
    icon: state === 'ok' ? 'check' : state === 'fail' ? 'xCircle' : 'copy', label: t('ai.copyLabel'),
    onClick: async () => {
      const ok = await copyText(text);
      copyState.set(m.id, ok ? 'ok' : 'fail');
      announce(t(ok ? 'ai.copied' : 'ai.copyFailed'));
      // Only this button changes (in place): redrawing every message would replace a button another finger is holding
      showCopyState(btn, ok ? 'ok' : 'fail');
      window.setTimeout(() => {
        copyState.delete(m.id);
        showCopyState(btn, null);
      }, COPY_MS);
    },
  });
  btn.id = `ai-copy-${m.id}`;
  btn.classList.add('ai-copy');
  if (state) btn.classList.add(state);
  return btn;
}

/** What Copy copies: the text shown. A summary request shows only its label, so Copy copies that label, not the
 *  machine line the model received (v0.9). */
function copySource(m: AiMessage): string {
  if (isAssessMessage(m)) return t('dash.ai.title');
  if (m.role !== 'user' || m.kind !== 'summary') return m.content;
  const sm = m.sample;
  return sm ? t('aiplus.chat.label.summary', { used: sm.used, wrong: sm.wrong }) : '';
}

/** The line right above a message (owner's request): "You" or "AI" and the time; "connection · model" only when `showModel`
 *  (an administrator in the free mode, `chatShowsModel`, or the administrators' chat viewer), everyone else only "AI" and the
 *  time (owner, v1.3 sprint 2 / v1.4); right-aligned over a sent message's bubble. The buttons go on one line right under
 *  the message (`actsRow`). */
export function whoLine(m: AiMessage, showModel: boolean, fmt: (sec: number) => string = (s) => fmtAiTime(s, localeTag())): HTMLElement {
  const who = whoParts(m, showModel);
  // One line, always: the connection and the time keep their width; only the model name gives way and is cut at its end
  // with an ellipsis (CSS). The full "connection · model" stays in the text (screen readers) and in `title`.
  const modelText = who.model ? (who.connection ? `${who.connection} · ${who.model}` : who.model) : null;
  const model = modelText && who.model
    ? h('span', { class: 'ai-model', title: modelText },
      // The dot is its own item: its spaces stay in the text (screen readers) but take no width (CSS), so it sits close
      ...(who.connection ? [h('span', { class: 'ai-model-conn' }, who.connection), h('span', { class: 'ai-model-sep' }, ' · ')] : []),
      h('span', { class: 'ai-model-name' }, who.model))
    : null;
  const mark = who.mark === 'you' ? t('ai.you') : who.mark === 'ai' ? t('ai.assistant') : null;
  return h('div', { class: 'ai-who' }, mark ? h('span', { class: 'ai-who-mark' }, mark) : null, model,
    h('time', { class: 'ai-time', datetime: new Date(m.createdAt * 1000).toISOString() }, fmt(m.createdAt)));
}

function reasoningLabel(v: { live: boolean; seconds: number | null }): string {
  return v.live ? t('ai.reasoning.live') : v.seconds != null ? t('ai.reasoning.done', { n: v.seconds }) : t('ai.reasoning.label');
}

function reasoningOpts(m: AiMessage): { idBase: string; userOpen: boolean | undefined; onToggle: (open: boolean) => void } {
  return { idBase: 'ai-rs', userOpen: reasoningOpen.get(m.id), onToggle: (o) => reasoningOpen.set(m.id, o) };
}

/**
 * v1.3: the model's reasoning above an answer (the page and the administrators' viewer share it): a grey toggle row and a
 * box of plain text (never HTML or markdown). Opening and closing flips `aria-expanded` and `hidden` in place; `onToggle`
 * keeps the reader's choice. null = no block (no reasoning, user messages, older answers).
 */
export function reasoningEl(
  m: ViewMessage, o: { idBase: string; userOpen: boolean | undefined; onToggle: (open: boolean) => void },
): HTMLElement | null {
  const v = reasoningView(m, o.userOpen);
  if (!v) return null;
  const id = `${o.idBase}-${m.id}`;
  const body = h('div', { class: 'ai-rs-body', id: `${id}-body`, role: 'region', 'aria-labelledby': `${id}-tog`, tabindex: '0', hidden: !v.open },
    h('p', { class: 'ai-rs-text', id: `${id}-text` }, v.text),
    v.truncated ? h('p', { class: 'ai-rs-cut' }, t('ai.reasoning.truncated')) : null);
  const tog = h('button', { type: 'button', class: 'ai-rs-tog', id: `${id}-tog`, 'aria-expanded': String(v.open), 'aria-controls': `${id}-body` },
    h('span', { class: 'ai-rs-label' }, reasoningLabel(v)), h('span', { class: 'ai-rs-chev', 'aria-hidden': 'true' }, '▸'));
  tog.addEventListener('click', () => {
    const open = tog.getAttribute('aria-expanded') !== 'true';
    tog.setAttribute('aria-expanded', String(open));
    body.hidden = !open;
    o.onToggle(open);
  });
  return h('div', { class: 'ai-rs', id, 'data-live': String(v.live) }, tog, body);
}

function actsRow(buttons: Child[]): HTMLElement | null {
  return buttons.some((b) => b != null && b !== false) ? h('div', { class: 'ai-meta' }, buttons) : null;
}

function messageEl(m: ViewMessage, pos: number): HTMLElement {
  const user = m.role === 'user';
  const busy = m.status === 'streaming';
  const foot = user ? null : footerText(m);
  const bad = m.status === 'error';
  // The suggestion sits under the answer to the question it is for (or under the question while there is no answer yet)
  const next = view.messages[view.messages.indexOf(m) + 1];
  const showSuggest = suggestion != null && (
    (!user && questionOf(view, m.id)?.id === suggestion.messageId) || (user && m.id === suggestion.messageId && (!next || next.role !== 'assistant')));
  // Every message, sent or answered, has a Copy button (owner's request); not while the answer is still being written
  const chatKind = (m.kind ?? 'chat') === 'chat';
  const buttons: Child[] = [
    busy || copySource(m).trim() === '' ? null : copyButton(m, copySource(m)),
    user && full() && chatKind && m.id > 0 ? rememberButton(m) : null,
    user && full() && chatKind && m.id > 0 ? editButton(m) : null,
    !user && full() && chatKind && m.status !== 'streaming' ? regenerateButton(m) : null,
    versionSwitch(m, pos),
  ];
  const editing = edit != null && edit.messageId === m.id;
  return h('li', { class: 'ai-msg ' + (user ? 'user' : 'assistant') + (editing ? ' editing' : ''), id: `ai-m-${m.id}`, 'aria-busy': busy ? 'true' : null },
    whoLine(m, chatShowsModel(isAdmin(), aiFreeMode())),
    user ? null : reasoningEl(m, reasoningOpts(m)),
    ...(user ? userBody(m) : [markdownEl(m.content, `ai-md-${m.id}`)]),
    // A streaming answer always has its foot (empty and hidden while there is no text), so paintStreaming need not repaint the list
    foot || (!user && busy) ? h('p', { class: 'ai-foot' + (bad ? ' bad' : ''), id: `ai-foot-${m.id}`, hidden: !foot }, foot ?? '') : null,
    actsRow(buttons),
    showSuggest ? suggestionBar() : null);
}

/** Remember (user messages): asks the model for a note, so it follows the request cap (C-6) */
function rememberButton(m: AiMessage): HTMLElement {
  const btn = iconButton({ icon: 'memory', label: t('ai.mem.remember'), onClick: guarded(() => void suggest(m.id)) });
  btn.id = `ai-rem-${m.id}`;
  btn.classList.add('ai-remember');
  btn.disabled = sending() || suggestion?.busy === true;
  applyCap(btn);
  return btn;
}

/** Regenerate: another model call, so it follows the request cap (C-6); the last answer of a failure keeps the name Retry */
function regenerateButton(m: ViewMessage): HTMLElement {
  const key = regenerateLabelKey(view, m);
  const btn = iconButton({ icon: 'refresh', label: t(key), onClick: guarded(() => regen(m)) });
  btn.id = key === 'ai.retry' ? 'ai-retry' : `ai-regen-${m.id}`;
  btn.classList.add('ai-regen', 'ai-retry');
  if (!canRegenerate(view, m)) btn.disabled = true;
  applyCap(btn);
  return btn;
}

/** Edit (a question): puts it into the composer; the model is called only when the edited text is sent */
function editButton(m: ViewMessage): HTMLElement {
  const btn = iconButton({ icon: 'pencil', label: t('ai.edit'), onClick: () => startEdit(m) });
  btn.id = `ai-edit-${m.id}`;
  btn.classList.add('ai-edit');
  btn.disabled = !canEdit(view, m);
  return btn;
}

/** `< n/total >` under a message with 2+ versions: no model call, so it is there in read-only mode too. The ids carry the
 *  position in the list (not the message id), so the focus stays on the same arrow after a switch. */
function versionSwitch(m: ViewMessage, pos: number): HTMLElement | null {
  const v = versionInfo(m);
  if (!v) return null;
  const off = sending() || switching;
  const arrow = (dir: 'prev' | 'next', to: number | null): HTMLElement => h('button', {
    type: 'button', class: 'ai-ver-btn', id: `ai-ver-${dir}-${pos}`, disabled: off || to == null,
    'aria-label': t(dir === 'prev' ? 'ai.version.prev' : 'ai.version.next'),
    onclick: () => {
      if (to != null) void switchTo(m.id, to);
    },
  }, dir === 'prev' ? '\u2039' : '\u203a');
  return h('span', { class: 'ai-ver', role: 'group', 'aria-label': t('ai.version.of', { n: v.index, total: v.total }) },
    arrow('prev', v.prev), h('span', { class: 'ai-ver-n', 'aria-hidden': 'true' }, `${v.index}/${v.total}`), arrow('next', v.next));
}

function msgsBody(): Child[] {
  if (opening && !view.messages.length) return [h('p', { class: 'dim', role: 'status' }, t('common.loading'))];
  if (openError) {
    return [h('div', { class: 'err', role: 'alert' }, h('p', null, t('ai.err.load', { reason: errorText(openError) })))];
  }
  const out: Child[] = [];
  if (view.running && !stream) out.push(h('p', { class: 'ai-conv-note ai-running' }, t('ai.running')));
  // A read-only user with no chat to show: say so (nothing can be started)
  if (!full() && showsEmptyHint(view)) {
    out.push(h('div', { class: 'ai-welcome' }, h('p', { class: 'ai-empty-hint', id: 'ai-readonly-empty' }, t('ai.readOnly.empty'))));
  }
  // A new chat with nothing in it yet: the greeting only (the composer and the admin notice are the footer's)
  if (full() && showsEmptyHint(view)) {
    out.push(h('div', { class: 'ai-welcome' }, h('p', { class: 'ai-greeting', id: 'ai-greeting' }, t('ai.greeting'))));
  }
  out.push(h('ol', { class: 'ai-msgs', id: 'ai-msgs', tabindex: '-1' }, view.messages.map((m, i) => messageEl(m, i))));
  return out;
}

// ---------------------------------------------------------------- the composer

/**
 * The full-height screen (CH-1 / CH-13): the px the input's text may take so that MIN_MSGS_PX of the conversation stay
 * visible between its head and the composer; Infinity elsewhere (desktop box, list pane shown). Read while the input has its
 * one-line height (autoGrow has just set it to auto).
 */
function textRoom(input: HTMLTextAreaElement, chrome: number): number {
  if (!composer || !document.documentElement.classList.contains('no-doc-scroll')) return Number.POSITIVE_INFINITY;
  const head = document.getElementById('ai-conv-head');
  if (!head || !head.getClientRects().length || !composer.dock.getClientRects().length) return Number.POSITIVE_INFINITY;
  const free = composer.dock.getBoundingClientRect().top - head.getBoundingClientRect().bottom;
  return free + input.offsetHeight - chrome - MIN_MSGS_PX;
}

/**
 * v2.3: compact (one row: brain, input, send) or expanded (text on top at the full width, controls under it). The text is tried
 * in the compact row first (no newline): if it wraps there the box goes expanded. Only classes change: the textarea is the same
 * element, so the caret, the focus and an input method's composition are kept; it all runs before the next paint.
 */
function layoutBox(input: HTMLTextAreaElement): void {
  const box = input.parentElement;
  if (!box || !box.classList.contains('ai-box')) return;
  box.classList.add('compact');
  input.style.height = 'auto';
  input.style.overflowY = 'hidden';
  const cs = window.getComputedStyle(input);
  const pad = (parseFloat(cs.paddingTop) || 0) + (parseFloat(cs.paddingBottom) || 0);
  const kind = composerLayout(input.value, input.scrollHeight, parseFloat(cs.lineHeight) || 22, pad);
  if (kind === 'expanded') box.classList.remove('compact');
  // The strength choice lives in the brain picker while the box is compact on a phone (and on the button while expanded): the
  // layout switched, so the open picker / strength menu is drawn again consistently
  const was = box.dataset.layout;
  const now = box.classList.contains('compact') ? 'compact' : 'expanded';
  if (was !== now) {
    box.dataset.layout = now;
    if (composer && composer.box === box && was !== undefined && window.matchMedia('(max-width: 719.98px)').matches) {
      if (now === 'compact' && effortPop) effortPop = false;
      syncModels(composer);
    }
  }
}

function autoGrow(input: HTMLTextAreaElement): void {
  layoutBox(input);
  input.style.height = 'auto';
  const cs = window.getComputedStyle(input);
  const line = parseFloat(cs.lineHeight) || 22;
  const pad = (parseFloat(cs.paddingTop) || 0) + (parseFloat(cs.paddingBottom) || 0) + (parseFloat(cs.borderTopWidth) || 0) + (parseFloat(cs.borderBottomWidth) || 0);
  // CH-7: 4 lines on touch or in a window under 700 px tall, 8 on desktop, then the input scrolls inside; never more than
  // ~22% of the visual viewport, and on the full-height screen never so tall that less than MIN_MSGS_PX of the conversation
  // stays visible (the open keyboard, a short landscape screen)
  const room = window.visualViewport?.height ?? window.innerHeight;
  const lines = composerLines(window.matchMedia('(pointer: coarse)').matches, window.innerHeight, room, line, textRoom(input, pad));
  const max = line * lines + pad;
  const want = input.scrollHeight + (parseFloat(cs.borderTopWidth) || 0) + (parseFloat(cs.borderBottomWidth) || 0);
  input.style.height = `${Math.ceil(Math.min(want, max))}px`;
  input.style.overflowY = want > max ? 'auto' : 'hidden';
}

function clearComposer(): void {
  if (!composer) return;
  composer.input.value = '';
  autoGrow(composer.input);
  syncComposer();
}

function makeComposer(): Composer {
  const input = h('textarea', {
    id: 'ai-input', class: 'ai-input', rows: '1', autocomplete: 'off', spellcheck: 'true', 'aria-describedby': 'ai-count',
    oninput: () => {
      autoGrow(input);
      syncCount();
    },
    // CH-13: iOS pans the page to the focused field; put it back (while the field has the focus and once after it)
    onfocus: () => startPanWatch(),
    onblur: () => stopPanWatch(),
    oncompositionstart: () => {
      composing = true;
    },
    oncompositionend: () => {
      // Safari fires compositionend before the keydown of the Enter that committed the word: clear the flag a tick later
      window.setTimeout(() => {
        composing = false;
      }, 0);
    },
    onkeydown: (e: KeyboardEvent) => {
      // Escape ends an edit (the earlier draft comes back); not while an input method composes
      if (e.key === 'Escape' && edit && !composing && !e.isComposing) {
        e.preventDefault();
        cancelEdit();
        return;
      }
      const fine = window.matchMedia('(pointer: fine)').matches;
      if (!shouldSend(e, { composing, finePointer: fine })) return;
      e.preventDefault();
      // While an answer is written the text can be typed, but not sent
      if (!sending()) send();
    },
  });
  const label = h('label', { class: 'sr-only', for: 'ai-input' });
  const count = h('span', { class: 'ai-count', id: 'ai-count', hidden: true });
  // Send = the send icon; while an answer is written the same slot is Stop (syncComposer swaps icon, name and tip)
  const btn = iconButton({
    icon: 'send', label: t('ai.send'), tone: 'ai',
    onClick: (e: MouseEvent) => {
      // The first click of a double-click sends and turns this same button into Stop: the second click (and any click
      // within a moment of the switch) must not stop the answer it just started. Enter / Space on the focused button
      // (click count 0) still stops.
      if (ignoreSendClick(e.detail, btn.type === 'button', Date.now() - stopSince)) {
        e.preventDefault();
        return;
      }
      if (btn.type === 'button') {
        e.preventDefault();
        void stop();
      }
    },
  });
  // Pressing Send / Stop must not take the focus from the input (the keyboard would close after every send)
  let keptFocus = false;
  const holdInput = (e: Event): void => {
    keptFocus = document.activeElement === input;
    if (keptFocus) e.preventDefault();
  };
  btn.addEventListener('pointerdown', holdInput);
  btn.addEventListener('mousedown', holdInput);
  btn.addEventListener('click', () => {
    if (keptFocus && document.activeElement !== input) input.focus({ preventScroll: true });
    keptFocus = false;
  });
  btn.id = 'ai-send';
  btn.classList.add('ai-send');
  btn.type = 'submit';
  const status = h('div', { class: 'ai-status', id: 'ai-status', hidden: true });
  const banner = h('div', { class: 'ai-edit-banner', id: 'ai-edit-banner', hidden: true });
  // The model picker (v1.9): the brain button opens a small popover with two lists (API, then model); a dot marks a choice
  // other than the default. The popover is drawn by syncModels as a child of the wrapper, which sits inside .ai-box.
  const modelDot = h('span', { class: 'ai-model-dot', hidden: true });
  const modelBtn = iconButton({ icon: 'brain', label: t('ai.model.label'), onClick: () => (modelPop ? closeModelPop(true) : openModelPop()) });
  modelBtn.id = 'ai-model-btn';
  modelBtn.classList.add('ai-model-face');
  modelBtn.setAttribute('aria-haspopup', 'dialog');
  modelBtn.setAttribute('aria-expanded', 'false');
  modelBtn.setAttribute('aria-controls', 'ai-model-pop');
  modelBtn.append(modelDot);
  // v2.2: the thinking strength is its own button beside the brain button; a small menu opens above it
  const effortLbl = h('span', { class: 'ai-effort-t' });
  const effortBtn = h('button', {
    class: 'ai-effort-btn', id: 'ai-effort-btn', type: 'button', hidden: true, 'aria-haspopup': 'menu', 'aria-expanded': 'false', 'aria-controls': 'ai-effort-pop',
    onclick: () => (effortPop ? closeEffortPop(true) : openEffortPop()),
  }, ico('strength', 'ai-effort-ic'), effortLbl);
  const effortHost = h('span', { class: 'ai-effort-wrap', id: 'ai-effort-wrap' }, effortBtn);
  const models = h('span', { class: 'ai-model-wrap', id: 'ai-model-pick', hidden: true }, modelBtn, effortHost);
  // One rounded box: the input on top; under it one bar with the model button (left) and the counter + Send (right)
  const box = h('div', {
    class: 'ai-box',
    // A click on the box's own padding or on the empty part of the bar goes to the input
    onclick: (e: MouseEvent) => {
      const at = e.target as Element | null;
      if (at === box || at?.classList.contains('ai-bar') || at?.classList.contains('ai-bar-r')) input.focus({ preventScroll: true });
    },
  }, input, h('div', { class: 'ai-bar' }, models, h('div', { class: 'ai-bar-r' }, count, btn)));
  // A new width (rotation, window resize, the sidebar) can change whether the text fits the compact row
  if (typeof ResizeObserver !== 'undefined') {
    let lastW = 0;
    new ResizeObserver(() => {
      const w = box.clientWidth;
      if (w && w !== lastW) {
        lastW = w;
        autoGrow(input);
      }
    }).observe(box);
  }
  const latestText = h('span', null);
  const latest = h('button', {
    class: 'btn ai-latest', id: 'ai-latest', type: 'button', hidden: true,
    // A tap never moves the focus: an input that has it keeps it (the keyboard stays up, the caret stays), an input that has not
    // gets none (no keyboard pops up on a phone)
    onpointerdown: (e: PointerEvent) => e.preventDefault(),
    onmousedown: (e: MouseEvent) => e.preventDefault(),
    onclick: () => {
      const onButton = document.activeElement === latest;
      toBottom();
      // Only when the button itself had the focus (Tab / Enter) and is about to hide, the focus goes to the message list
      if (onButton) document.getElementById('ai-msgs')?.focus({ preventScroll: true });
    },
  }, ico('down'), latestText);
  const form = h('form', {
    class: 'ai-form', id: 'ai-form', novalidate: true,
    onsubmit: (e: SubmitEvent) => {
      e.preventDefault();
      if (sending()) return;
      send();
    },
  }, latest, status, banner, label, box);
  // The fixed footer of CH-13: the form (full mode only) and the administrator notice line under it (CH-3)
  const notice = noticeButton('ai-notice-dock');
  const dock = h('div', { class: 'ai-composer', id: 'ai-composer' }, form, notice);
  dockObserver?.disconnect();
  dockObserver = null;
  if (typeof ResizeObserver !== 'undefined') {
    dockObserver = new ResizeObserver(syncComposerHeight);
    // The border box: html.kb-open changes only the footer's bottom padding (safe area -> 8 px), which the content box misses
    dockObserver.observe(dock, { box: 'border-box' });
  }
  return { form, input, label, count, btn, status, banner, box, models, modelBtn, modelDot, effortBtn, effortLbl, effortHost, latest, latestText, dock, notice };
}

/** The administrator notice: one muted line with the info icon that wraps to show the whole text (owner, v2.2: never cut
 *  off with an ellipsis on phones; before v2.2 a tap opened the cut text) */
function noticeButton(extra: string): HTMLElement {
  return h('p', { class: `ai-notice-btn ${extra}` }, ico('info'), h('span', null, t('ai.adminNotice')));
}

function syncCount(): void {
  if (!composer) return;
  const max = limits().messageChars;
  const n = memoryLength(composer.input.value);
  const show = n >= max - Math.min(2000, Math.floor(max / 8));
  composer.count.hidden = !show;
  composer.count.textContent = show ? t('ai.input.count', { n: n.toLocaleString(localeTag()), max: max.toLocaleString(localeTag()) }) : '';
  composer.count.classList.toggle('over', n > max);
}

/** An answer is being written in another chat (CH-7): the status slot says so and offers Stop it */
function busyElsewhere(): boolean {
  const st = aiStatus();
  return st.state === 'ready' && isBusyElsewhere(st.running, view.chatId, sending());
}

/** While "another chat is busy" shows: the AI status is read again every few seconds, so the line goes when that answer ends */
const BUSY_POLL_MS = 3000;
let busyTimer: number | null = null;

function watchBusy(on: boolean): void {
  if (!on) {
    if (busyTimer != null) window.clearTimeout(busyTimer);
    busyTimer = null;
    return;
  }
  if (busyTimer != null) return;
  busyTimer = window.setTimeout(() => {
    busyTimer = null;
    if (!onPage() || !busyElsewhere()) return;
    void refreshAi(true).then(() => {
      if (onPage()) syncComposer();
    });
  }, BUSY_POLL_MS);
}

/** Redraws the status slot when what it shows changed: an error, the cap callout, or "another chat is busy" + Stop it */
function syncStatus(c: Composer): void {
  const block = callBlock();
  const kind = composerStatus({ sendError, capped: block != null, busyElsewhere: busyElsewhere() });
  watchBusy(kind === 'busy' && onPage());
  const text = kind === 'error' ? (sendError ?? '') : kind === 'cap' && block ? capShort(block) : kind === 'busy' ? t('ai.busyElsewhere') : '';
  const sig = `${kind}|${text}|${uiLang()}`;
  if (sig === statusSig) return;
  statusSig = sig;
  // The slot's height changed: the input may now take more or fewer lines (the conversation keeps its room)
  if (composer && document.activeElement === composer.input) queueMicrotask(() => composer && autoGrow(composer.input));
  c.status.hidden = kind == null;
  if (kind === 'error') {
    c.status.replaceChildren(h('div', { class: 'err ai-send-err', id: 'ai-send-err', role: 'alert' }, h('p', null, text)));
  } else if (kind === 'cap') {
    // Warn callout with the clock; no alert role (it only stands there while the cap blocks calls)
    const longText = block ? capText(block) : text;
    const note = h('div', { class: 'callout warn ai-cap', id: 'ai-cap-note' }, ico('clock'),
      h('span', { class: 'ai-cap-t', 'aria-hidden': 'true' }, text), h('span', { class: 'sr-only' }, longText));
    holdTip(note, longText);
    c.status.replaceChildren(note);
  } else if (kind === 'busy') {
    c.status.replaceChildren(h('div', { class: 'ai-busy', id: 'ai-busy' },
      h('span', null, text),
      h('button', { class: 'btn ai-mini', id: 'ai-stop-other', type: 'button', onclick: () => void stopElsewhere() }, t('ai.stopIt'))));
  } else {
    c.status.replaceChildren();
  }
}

/** What the model picker shows now (it is redrawn only when this changes, so an open list is not closed under a finger) */
let modelSig = '';

/** The AI Chat default model of the status (null = none, or the status is not ready) */
function chatDefault(): ChatDefault {
  const st = aiStatus();
  return st.state === 'ready' ? st.chatDefault ?? null : null;
}

/** Opens the popover; the model list is read again when it is missing or older than a minute */
function openModelPop(): void {
  const c = composer;
  if (!c || modelPop) return;
  effortPop = false;
  modelPop = true;
  pendingConn = null;
  if (!modelList || Date.now() - modelsAt > 60_000) void loadModels(true);
  syncModels(c);
  // Touch: the heading takes the focus (no ring on the first select), as the dialogs do
  if (window.matchMedia('(pointer: coarse)').matches) document.getElementById('ai-model-pop-t')?.focus({ preventScroll: true });
  else document.getElementById('ai-model-conn')?.focus();
}

/** Closes the popover (`focusBtn`: the focus returns to the brain button; false when the page moves on anyway) */
function closeModelPop(focusBtn: boolean): void {
  if (!modelPop) return;
  modelPop = false;
  pendingConn = null;
  const c = composer;
  if (c) syncModels(c);
  if (focusBtn) c?.modelBtn.focus();
}

/** v2.2: opens the strength menu (the brain popover closes first) */
function openEffortPop(): void {
  const c = composer;
  if (!c || effortPop) return;
  if (modelPop) closeModelPop(false);
  // the shown level comes from the AI settings: read them again when stale, so it matches what the server applies
  if (!modelList || Date.now() - aiSetAt > 60_000) void loadModels(true);
  effortPop = true;
  syncModels(c);
  (document.querySelector('#ai-effort-pop [aria-checked="true"]') as HTMLElement | null)?.focus();
}

/** Closes the strength menu (`focusBtn`: the focus returns to its button) */
function closeEffortPop(focusBtn: boolean): void {
  if (!effortPop) return;
  effortPop = false;
  const c = composer;
  if (c) syncModels(c);
  if (focusBtn) c?.effortBtn.focus();
}

const FOLLOW_VALUE = 'follow:inherit';

/** v2.3: the compact one-row box on a phone has no room for the strength button: its choice is a section of the brain picker */
function inlineStrength(c: Composer): boolean {
  return c.box.classList.contains('compact') && window.matchMedia('(max-width: 719.98px)').matches;
}

/**
 * The strength choices as radio items (Default first, the shown one checked, "follow the AI settings" last while the chat has its
 * own choice).
 */
function effortItems(c: Composer, sv: StrengthView, route: ModelRef | null): HTMLButtonElement[] {
  const after = (v: string): void => {
    effortSel = v;
    effortPop = false;
    syncModels(c);
    c.effortBtn.focus();
  };
  const items: HTMLButtonElement[] = sv.options.map((o) => {
    const isDef = o === EFFORT_DEFAULT;
    const b = h('button', {
      class: 'ai-effort-item', type: 'button', role: 'menuitemradio', 'aria-checked': String(sv.selected === o), tabindex: sv.selected === o ? '0' : '-1',
      'data-v': o,
      onclick: () => {
        // the shown item is the inherited one while the chat follows the AI settings: nothing changes, the chat keeps following
        after(effortSel === '' && sv.selected === o ? effortSel : o);
      },
    }, h('span', { class: 'ai-effort-chk', 'aria-hidden': 'true' }, sv.selected === o ? ico('check') : null),
    h('span', { class: 'ai-effort-lbl' }, isDef ? t('ai.effort.defaultOption') : o));
    return b;
  });
  // last item: drop the chat's own choice and follow the Chat row of the AI settings again (sends null)
  if (effortSel !== '') {
    const inherited = strengthView(aiSet, route?.connectionId ?? null, route?.model ?? null, '')?.selected ?? EFFORT_DEFAULT;
    items.push(h('button', {
      class: 'ai-effort-item', type: 'button', role: 'menuitemradio', 'aria-checked': 'false', tabindex: '-1', 'data-v': '',
      onclick: () => after(''),
    }, h('span', { class: 'ai-effort-chk', 'aria-hidden': 'true' }),
    h('span', { class: 'ai-effort-lbl' }, t('ai.effort.follow', { level: inherited === EFFORT_DEFAULT ? t('ai.effort.default') : inherited }))));
  }
  return items;
}

/** The strength menu of the current route: the options (Default first) as radio items, the shown one checked */
function effortMenu(c: Composer, sv: StrengthView, route: ModelRef | null): HTMLElement {
  const items = effortItems(c, sv, route);
  return h('div', {
    class: 'ai-model-pop ai-effort-pop', id: 'ai-effort-pop', role: 'menu', 'aria-label': t('ai.model.pick.strengthAria', { param: EFFORT_PARAM[sv.family] }),
    onkeydown: (e: KeyboardEvent) => {
      const at = items.indexOf(document.activeElement as HTMLButtonElement);
      const to = e.key === 'ArrowDown' ? at + 1 : e.key === 'ArrowUp' ? at - 1 : e.key === 'Home' ? 0 : e.key === 'End' ? items.length - 1 : null;
      if (to == null) return;
      e.preventDefault();
      items[(to + items.length) % items.length]?.focus();
    },
  }, ...items);
}

/** The strength button beside the brain button: shown while the picker is usable and the route's family is known */
function syncEffort(c: Composer, show: boolean, route: ModelRef | null, busy: boolean): void {
  const sv = show ? strengthView(aiSet, route?.connectionId ?? null, route?.model ?? null, effortSel) : null;
  c.effortBtn.hidden = sv == null;
  c.effortHost.querySelector('#ai-effort-pop')?.remove();
  if (!sv) {
    effortPop = false;
    return;
  }
  const param = EFFORT_PARAM[sv.family];
  const shownText = sv.selected === EFFORT_DEFAULT ? t('ai.effort.default') : sv.selected;
  c.effortLbl.textContent = shownText;
  const tip = `${t('ai.model.pick.strengthAria', { param })}: ${shownText}`;
  c.effortBtn.setAttribute('aria-label', tip);
  c.effortBtn.setAttribute('aria-expanded', String(effortPop));
  c.effortBtn.disabled = busy;
  c.effortBtn.classList.toggle('on', sv.selected !== EFFORT_DEFAULT);
  holdTip(c.effortBtn, tip);
  if (effortPop && !busy) c.effortHost.append(effortMenu(c, sv, route));
  else effortPop = false;
}

/** The brain button: a dot for a choice other than the default (amber when its connection is off), tip and name say the model */
function markModel(c: Composer, shown: string, off: boolean, offName: string | null, def: ChatDefault): void {
  const ref = shown === '' ? null : parseModelRef(shown);
  c.modelDot.hidden = shown === '';
  c.modelDot.classList.toggle('warn', off);
  const base = off && ref
    ? t('ai.model.off', { conn: offName ?? '', model: ref.model })
    : ref
      ? t('ai.model.chosen', { name: ref.model })
      : def
        ? t('ai.model.default', { model: def.model })
        : t('ai.model.label');
  c.modelBtn.setAttribute('aria-label', base);
  c.modelBtn.setAttribute('aria-expanded', String(modelPop));
  holdTip(c.modelBtn, base);
}

/** The popover: the API (connection) first, then the model of that connection; picking the model marked default means "follow the default" */
function modelPopover(c: Composer, menu: ChatMenu, def: ChatDefault, shown: string, busy: boolean): HTMLElement {
  const pv = pickerView(shown, pendingConn, menu, def);
  const ref = shown === '' ? null : parseModelRef(shown);
  const offId = menu.off && ref ? ref.connectionId : null;
  const focusLater = (id: string): void => {
    document.getElementById(id)?.focus();
  };
  const connSel = h('select', {
    class: 'ais-select ai-model-in', id: 'ai-model-conn', 'aria-labelledby': 'ai-model-conn-l', disabled: busy || (menu.conns.length === 0 && offId == null),
    onchange: () => {
      pendingConn = Number(connSel.value);
      syncModels(c);
      focusLater('ai-model-sel');
    },
  },
  h('option', { value: '', hidden: true, disabled: true }, t('ai.model.apiPick')),
  ...menu.conns.map((x) => h('option', { value: String(x.id) }, x.name)),
  offId != null && !menu.conns.some((x) => x.id === offId)
    ? h('option', { value: String(offId), disabled: true }, t('ai.model.offOpt', { name: menu.off?.name ?? '' }))
    : null);
  connSel.value = pv.connId == null ? '' : String(pv.connId);
  const models = menu.conns.find((x) => x.id === pv.connId)?.models ?? [];
  const noneAtAll = menu.conns.length === 0;
  const offPick = offId != null && pv.connId === offId && pv.modelValue !== '';
  const modelSelEl = h('select', {
    class: 'ais-select ai-model-in', id: 'ai-model-sel', 'aria-labelledby': 'ai-model-sel-l', disabled: busy || models.length === 0,
    onchange: () => {
      modelSel = chooseValue(modelSelEl.value, def);
      const mr = parseModelRef(modelSelEl.value);
      // the strength choice stays when the new family has the level ('default' always stays), else it is dropped
      effortSel = keepEffort(effortSel, strengthView(aiSet, mr?.connectionId ?? null, mr?.model ?? null, effortSel));
      pendingConn = null;
      syncModels(c);
      focusLater('ai-model-sel');
    },
  },
  h('option', { value: '', hidden: true, disabled: true }, noneAtAll ? t('ai.model.none') : t('ai.model.modelPick')),
  ...models.map((m) => h('option', { value: m.value }, m.isDefault ? t('ai.model.isDefault', { model: m.label }) : m.label)),
  offPick && ref ? h('option', { value: pv.modelValue, disabled: true }, ref.model) : null);
  modelSelEl.value = pv.modelValue;
  const close = iconButton({ icon: 'x', label: t('common.close'), tone: 'quiet', onClick: () => closeModelPop(true) });
  close.id = 'ai-model-close';
  const useDefault = modelSel !== ''
    ? h('div', { class: 'ai-model-foot' }, h('button', {
      class: 'btn quiet ai-mini', id: 'ai-model-default', type: 'button', disabled: busy,
      onclick: () => {
        modelSel = '';
        // the default route may be of another family: a level it cannot take is dropped, never sent
        effortSel = keepEffort(effortSel, strengthView(aiSet, def?.connectionId ?? null, def?.model ?? null, effortSel));
        pendingConn = null;
        syncModels(c);
        focusLater('ai-model-conn');
      },
    }, t('ai.model.useDefault')))
    : null;
  if (!menu.loaded) {
    const failed = modelsFailed && !modelsLoading;
    return h('div', { class: 'ai-model-pop', id: 'ai-model-pop', role: 'dialog', 'aria-labelledby': 'ai-model-pop-t' },
      h('div', { class: 'ai-model-pop-h' }, h('h3', { id: 'ai-model-pop-t', tabindex: '-1' }, t('ai.model.pick.title')), close),
      failed
        ? h('div', { class: 'ai-model-f', role: 'alert' },
          h('p', { class: 'callout warn' }, ico('info'), h('span', null, t('ai.model.loadFail'))),
          h('button', {
            class: 'btn quiet ai-mini', id: 'ai-model-retry', type: 'button',
            onclick: () => {
              void loadModels(true);
              syncModels(c);
            },
          }, t('common.retry')))
        : h('p', { class: 'ai-model-f', role: 'status' }, t('ai.loading')));
  }
  // v2.3: in the compact one-row box on a phone the strength is the last step here (API, model, strength); the expanded box
  // shows the separate button instead
  const route: ModelRef | null = ref ?? (def ? { connectionId: def.connectionId, model: def.model } : null);
  const sv = inlineStrength(c) ? strengthView(aiSet, route?.connectionId ?? null, route?.model ?? null, effortSel) : null;
  // a dropdown like the API and model steps: Default first, the family's levels, "follow the AI settings" while the chat has its own choice
  const strSel = sv
    ? h('select', {
      class: 'ais-select ai-model-in', id: 'ai-model-str', 'aria-labelledby': 'ai-model-str-l', disabled: busy,
      onchange: () => {
        effortSel = strSel?.value === FOLLOW_VALUE ? '' : (strSel?.value ?? effortSel);
        syncModels(c);
        focusLater('ai-model-str');
      },
    },
    ...sv.options.map((o) => h('option', { value: o }, o === EFFORT_DEFAULT ? t('ai.effort.defaultOption') : o)),
    effortSel !== ''
      ? h('option', { value: FOLLOW_VALUE }, t('ai.effort.follow', {
        level: (() => {
          const inh = strengthView(aiSet, route?.connectionId ?? null, route?.model ?? null, '')?.selected ?? EFFORT_DEFAULT;
          return inh === EFFORT_DEFAULT ? t('ai.effort.default') : inh;
        })(),
      }))
      : null)
    : null;
  if (strSel && sv) strSel.value = sv.selected;
  const strength = strSel
    ? h('div', { class: 'ai-model-f' }, h('label', { id: 'ai-model-str-l', for: 'ai-model-str' }, t('ai.model.strength')), strSel)
    : null;
  return h('div', { class: 'ai-model-pop', id: 'ai-model-pop', role: 'dialog', 'aria-labelledby': 'ai-model-pop-t' },
    h('div', { class: 'ai-model-pop-h' }, h('h3', { id: 'ai-model-pop-t', tabindex: '-1' }, t('ai.model.pick.title')), close),
    menu.off && ref ? h('p', { class: 'callout warn ai-model-off', role: 'status' }, ico('info'),
      h('span', null, t('ai.model.off', { conn: menu.off.name ?? '', model: ref.model }))) : null,
    h('div', { class: 'ai-model-f' }, h('label', { id: 'ai-model-conn-l', for: 'ai-model-conn' }, t('ai.model.api')), connSel),
    h('div', { class: 'ai-model-f' }, h('label', { id: 'ai-model-sel-l', for: 'ai-model-sel' }, t('ai.model.label')), modelSelEl),
    strength,
    useDefault);
}

/** The administrators' model picker: the brain button and its popover; hidden for everyone else and read only */
function syncModels(c: Composer): void {
  const show = isAdmin() && full() && aiFreeMode();
  if (!show && (modelPop || effortPop)) {
    modelPop = false;
    effortPop = false;
    pendingConn = null;
  }
  // A failed read is tried again after ten seconds at the earliest, so a broken connection to the server never loops
  if (show && !modelList && Date.now() - modelsAt > 10_000) void loadModels();
  const def = chatDefault();
  // The menu follows the selection (at first the chat's stored pick): an off pick warns only while it is the selection
  let menu = chatMenu(modelList, modelSel === '' ? null : parseModelRef(modelSel), def);
  // An unsent change that the list no longer offers falls back to the stored pick
  if (chatShown(modelSel, menu) !== modelSel && modelSel !== pickText(view.pick)) {
    modelSel = pickText(view.pick);
    effortSel = view.pickEffort ?? '';
    menu = chatMenu(modelList, modelSel === '' ? null : parseModelRef(modelSel), def);
  }
  // A stored pick on a broken or removed connection shows as the default; modelSel keeps the pick, so the request says
  // "unchanged" and the server falls back and clears it
  const shown = chatShown(modelSel, menu);
  const busy = sending();
  const sig = JSON.stringify([show, uiLang(), def, menu, shown, busy, modelPop, pendingConn, modelsLoading, modelsFailed, effortSel, effortPop, aiSetAt, inlineStrength(c)]);
  if (sig === modelSig) return;
  modelSig = sig;
  c.models.hidden = !show;
  const inPop = c.models.querySelector('#ai-model-pop')?.contains(document.activeElement) ? document.activeElement?.id ?? '' : '';
  c.models.querySelector('#ai-model-pop')?.remove();
  const dotRef = shown !== '' ? parseModelRef(shown) : def ? { connectionId: def.connectionId, model: def.model } : null;
  markModel(c, shown, menu.off != null, menu.off?.name ?? null, def);
  const inEffort = c.effortHost.contains(document.activeElement);
  syncEffort(c, show, dotRef, busy);
  if (inEffort && effortPop) (document.querySelector('#ai-effort-pop [aria-checked="true"]') as HTMLElement | null)?.focus();
  if (!show || !modelPop) return;
  c.models.append(modelPopover(c, menu, def, shown, busy));
  // A redraw (the list arrived, the language changed) keeps the focus where it was in the popover
  if (inPop) document.getElementById(inPop)?.focus();
}

/** The banner above the input while a question is edited: what it means and Cancel */
function syncBanner(c: Composer): void {
  if (edit && !editValid(view, edit)) {
    // The edited question left the branch (another tab, a poll): the edit ends, the earlier draft returns
    const e = edit;
    edit = null;
    c.input.value = e.draft;
    autoGrow(c.input);
  }
  const on = edit != null;
  const sig = `${on}|${uiLang()}`;
  if (c.banner.dataset.sig === sig) return;
  c.banner.dataset.sig = sig;
  c.banner.hidden = !on;
  if (!on) {
    c.banner.replaceChildren();
    return;
  }
  c.banner.replaceChildren(
    h('span', { class: 'ai-edit-t', role: 'status' }, ico('pencil'), h('span', null, t('ai.edit.banner'))),
    h('button', { class: 'btn quiet ai-mini', id: 'ai-edit-cancel', type: 'button', onclick: () => cancelEdit() }, t('ai.edit.cancel')));
}

/** Brings the composer in line with the state: the language, Send or Stop, the counter, the status slot */
function syncComposer(): void {
  const c = composer;
  if (!c) return;
  syncBanner(c);
  syncModels(c);
  c.label.textContent = t('ai.input.label');
  c.input.placeholder = t('ai.input.placeholder');
  const busy = sending();
  if (busy && c.btn.type !== 'button') stopSince = Date.now();
  c.btn.type = busy ? 'button' : 'submit';
  c.btn.classList.toggle('stop', busy);
  const label = t(busy ? 'ai.stop' : 'ai.send');
  const icon = busy ? 'stop' : 'send';
  if (c.btn.dataset.icon !== icon) {
    c.btn.dataset.icon = icon;
    c.btn.replaceChildren(ico(icon, 'ibtn-ic'));
  }
  c.btn.setAttribute('aria-label', label);
  // Send needs a model call: while the cap blocks calls it is aria-disabled with the cap text as its tip; Stop never is
  const q = callBlock();
  if (!busy && q) {
    c.btn.setAttribute('aria-disabled', 'true');
    c.btn.setAttribute('aria-describedby', 'ai-cap-note');
    holdTip(c.btn, capText(q));
  } else {
    c.btn.removeAttribute('aria-disabled');
    c.btn.removeAttribute('aria-describedby');
    holdTip(c.btn, label);
  }
  syncStatus(c);
  c.latestText.textContent = t('ai.latest');
  const noticeText = c.notice.lastElementChild;
  if (noticeText) noticeText.textContent = t('ai.adminNotice');
  syncCount();
  syncLatest();
}

// ---------------------------------------------------------------- Memory

async function addMem(): Promise<void> {
  const text = memDraft.trim();
  if (!text || memBusy) return;
  const max = limits().memoryTokens;
  if (memoryTokenCount(text) > max) {
    memFormError = aiCodeText('ai.memoryTooLong', max);
    render();
    return;
  }
  memBusy = true;
  memFormError = null;
  render();
  try {
    const m = await addMemory(text, 'manual');
    memories = [m, ...(memories ?? [])];
    memDraft = '';
    toast(t('ai.mem.saved'));
    void refreshAi(true);
  } catch (e) {
    memFormError = errorText(e);
  }
  memBusy = false;
  if (onPage()) render();
}

async function saveMemEdit(): Promise<void> {
  const ed = memEdit;
  if (!ed || memBusy) return;
  const text = ed.text.trim();
  if (!text) return;
  memBusy = true;
  memFormError = null;
  render();
  try {
    const m = await updateMemory(ed.id, text);
    memories = (memories ?? []).map((x) => (x.id === m.id ? m : x));
    memEdit = null;
  } catch (e) {
    memFormError = errorText(e);
  }
  memBusy = false;
  if (onPage()) render();
  document.getElementById(`ai-medit-${ed.id}`)?.focus();
}

async function removeMem(m: AiMemory): Promise<void> {
  if (!(await ask({ title: t('ai.mem.delete.title'), body: t('ai.mem.delete.ask'), ok: t('ai.mem.delete'), danger: true }))) return;
  try {
    await deleteMemory(m.id);
    memories = (memories ?? []).filter((x) => x.id !== m.id);
    toast(t('ai.mem.deleted'));
    void refreshAi(true);
  } catch (e) {
    toast(errorText(e), 6000);
    memWanted = true;
  }
  if (onPage()) render();
}

async function removeAllMem(): Promise<void> {
  const n = memories?.length ?? 0;
  if (!n) return;
  if (!(await ask({ title: t('ai.mem.deleteAll.title'), body: tn('ai.mem.deleteAll.ask', n), ok: t('ai.mem.deleteAll'), danger: true }))) return;
  try {
    await deleteAllMemories();
    memories = [];
    toast(t('ai.mem.deleted'));
    void refreshAi(true);
  } catch (e) {
    toast(errorText(e), 6000);
    memWanted = true;
  }
  if (onPage()) render();
}

const shortText = (s: string): string => ([...s].length > 40 ? [...s].slice(0, 40).join('') + '…' : s);

function cancelMemEdit(id: number): void {
  memEdit = null;
  memFormError = null;
  render();
  document.getElementById(`ai-medit-${id}`)?.focus();
}

/** An icon button of a memory row (pencil, red trash, save, cancel): the name is the label, disabled while a save runs */
function memIcon(icon: MiniIcon, label: string, id: string, onClick: () => void, tone: 'default' | 'danger' | 'ai' = 'default'): HTMLButtonElement {
  const b = iconButton({ icon, label, tone, onClick });
  b.id = id;
  b.disabled = memBusy;
  return b;
}

function memoryRow(m: AiMemory): HTMLElement {
  const editing = memEdit?.id === m.id;
  const max = limits().memoryTokens;
  const src = m.source === 'chat' ? 'ai.mem.source.chat' : 'ai.mem.source.manual';
  if (editing && memEdit) {
    const area = h('textarea', {
      id: `ai-medit-in-${m.id}`, class: 'ai-mem-in', rows: '2', maxlength: String(memoryBoxChars(max)), 'aria-label': t('ai.mem.editLabel'),
      oninput: () => {
        if (memEdit) memEdit = { ...memEdit, text: area.value };
      },
      // Esc cancels the edit (like the Cancel button) and gives the focus back to the row's Edit button
      onkeydown: (e: KeyboardEvent) => {
        if (e.key !== 'Escape' || memBusy) return;
        e.preventDefault();
        cancelMemEdit(m.id);
      },
    });
    area.value = memEdit.text;
    return h('li', { class: 'ai-mem-row editing' },
      area,
      h('div', { class: 'ai-mem-acts' }, memIcon('check', t('ai.mem.save'), `ai-msave-${m.id}`, () => void saveMemEdit(), 'ai'),
        memIcon('x', t('ai.mem.cancel'), `ai-mcancel-${m.id}`, () => cancelMemEdit(m.id))));
  }
  return h('li', { class: 'ai-mem-row' },
    h('span', { class: 'ai-mem-src', role: 'img', 'aria-label': t(src), title: t(src) }, miniIcon(m.source === 'chat' ? 'chat' : 'plus', 'ai-mi')),
    h('p', { class: 'ai-mem-t' }, m.content),
    h('div', { class: 'ai-mem-acts' },
      full()
        ? memIcon('pencil', t('ai.mem.editAria', { text: shortText(m.content) }), `ai-medit-${m.id}`, () => {
          memEdit = { id: m.id, text: m.content };
          render();
          document.getElementById(`ai-medit-in-${m.id}`)?.focus();
        })
        : null,
      memIcon('trash', t('ai.mem.deleteAria', { text: shortText(m.content) }), `ai-mdel-${m.id}`, () => void removeMem(m), 'danger')));
}

function memoryView(): HTMLElement {
  if (memWanted && !memLoading) void loadMemories();
  const lim = limits();
  const list = memories;
  let body: Child;
  if (!list) {
    body = memError
      ? h('div', { class: 'err', role: 'alert' }, h('p', null, t('ai.mem.errLoad', { reason: errorText(memError) })),
        h('div', { class: 'acts' }, h('button', { class: 'btn', type: 'button', onclick: () => void loadMemories() }, t('common.retry'))))
      : h('p', { class: 'dim', role: 'status' }, t('common.loading'));
  } else {
    body = [
      list.length ? h('ul', { class: 'ai-mems' }, list.map(memoryRow)) : h('p', { class: 'ai-list-note' }, t('ai.mem.empty')),
      list.length
        ? h('button', { class: 'btn danger ai-delall', id: 'ai-mdelall', type: 'button', disabled: memBusy, onclick: () => void removeAllMem() }, t('ai.mem.deleteAll'))
        : null,
    ];
  }
  const area = h('textarea', {
    id: 'ai-mem-input', class: 'ai-mem-in', rows: '2', maxlength: String(memoryBoxChars(lim.memoryTokens)), placeholder: t('ai.mem.placeholder'),
    disabled: memBusy, 'aria-describedby': 'ai-mem-count',
    oninput: () => {
      memDraft = area.value;
      count.textContent = t('ai.mem.count', { n: memoryTokenCount(area.value), max: lim.memoryTokens });
    },
  });
  area.value = memDraft;
  const count = h('span', { class: 'ai-count', id: 'ai-mem-count' }, t('ai.mem.count', { n: memoryTokenCount(memDraft), max: lim.memoryTokens }));
  const addBtn = iconButton({ icon: 'plus', label: t('ai.mem.add'), tone: 'ai', showLabel: true });
  addBtn.id = 'ai-mem-add';
  addBtn.type = 'submit';
  addBtn.disabled = memBusy || (list != null && list.length >= lim.memories);
  const form = full()
    ? h('form', {
      class: 'ai-mem-add', novalidate: true,
      onsubmit: (e: SubmitEvent) => {
        e.preventDefault();
        void addMem();
      },
    },
    h('label', { class: 'fld-l', for: 'ai-mem-input' }, t('ai.mem.addLabel')),
    h('div', { class: 'ai-mem-addrow' }, area, addBtn),
    count)
    : null;
  return h('section', { class: 'panel ai-mem', 'aria-labelledby': 'ai-mem-h' },
    h('div', { class: 'ai-mem-head' },
      h('h2', { id: 'ai-mem-h', tabindex: '-1' }, t('ai.tab.memory')),
      list ? h('span', { class: 'ai-mem-n' }, t('ai.mem.count', { n: list.length, max: lim.memories })) : null),
    h('p', { class: 'ai-mem-intro' }, t('ai.mem.intro')),
    form,
    memFormError ? h('div', { class: 'err', role: 'alert' }, h('p', null, memFormError)) : null,
    body);
}

// ---------------------------------------------------------------- the page

/** CH-8: a paused user with chats sees one muted line in the composer's place (the notice line stays under it) */
function readOnlyLine(): HTMLElement {
  const st = aiStatus();
  const reason = st.state === 'ready' ? readOnlyReason(st) : 'unavailable';
  const paused = reason === 'suspended';
  const text = paused
    ? t('ai.readOnly.paused')
    : reason === 'featureOff'
      ? t(featureOffKey(isAdmin()))
      : (reason === 'noFunction' ? t('ai.err.notAllowed') : badgeReasonText(reason, isAdmin())) + (uiLang() === 'en' ? ' ' : '') + t('ai.readOnlyTail');
  return h('p', { class: 'ai-readonly-line', id: 'ai-readonly' }, ico(paused ? 'pauseCircle' : 'info'), h('span', null, text));
}

function chatsView(): HTMLElement {
  if (!composer) composer = makeComposer();
  syncComposer();
  listSig = listSignature();
  composer.dock.replaceChildren(...(full() ? [composer.form] : [readOnlyLine()]), composer.notice);
  const conv = h('section', { class: 'ai-conv', 'aria-labelledby': 'ai-conv-t' },
    h('div', { class: 'ai-conv-head', id: 'ai-conv-head' }, convHead()),
    h('div', { class: 'ai-scroll', id: 'ai-scroll', tabindex: '0', role: 'region', 'aria-label': t('ai.msgs.label'), onscroll: onScroll },
      h('div', { class: 'ai-msgs-wrap', id: 'ai-msgs-wrap' }, msgsBody())),
    composer.dock);
  return panesView(conv);
}

/** The memory view beside the sidebar: [menu and "Memory n / max" on a phone] ... [Back to chat], then the memory box */
function memoryPane(): HTMLElement {
  listSig = listSignature();
  const phone = onePane();
  const barTitle = phone
    ? h('span', { class: 'ai-mem-bar-t', 'aria-hidden': 'true' },
      t('ai.tab.memory'), memories ? ` ${t('ai.mem.count', { n: memories.length, max: limits().memories })}` : '')
    : null;
  return h('section', { class: 'ai-memwrap', style: memH && !phoneScreen() ? `--ai-mem-h:${memH}px` : null },
    h('div', { class: 'ai-conv-head ai-mem-bar' },
      phone ? drawerButton() : null,
      barTitle,
      h('button', { class: 'btn quiet ai-mem-back', id: 'ai-mem-back', type: 'button', onclick: () => backToChat() },
        miniIcon('back', 'ai-mi'), t('ai.mem.back'))),
    memoryView(),
    // owner (v1.3): on phones the GitHub line sits under the memory box, right above the notice line, not inside the box;
    // from 720 px the sidebar already ends with it (once, not twice)
    phone ? githubLine() : null, noticeButton('ai-notice-flow'));
}

/** Back to the conversation that was open (a new one when none is) */
function backToChat(): void {
  showChats();
  if (view.chatId == null && !view.messages.length) {
    if (window.matchMedia('(pointer: fine)').matches) document.getElementById('ai-input')?.focus({ preventScroll: true });
  }
  else document.getElementById('ai-conv-t')?.focus({ preventScroll: true });
}

/**
 * #/ai while the AI service cannot be reached (its status could not be read): says so, instead of "Not allowed", which would
 * blame the account. Retry reads the status again; the page follows when the service answers.
 */
export function renderAiUnavailable(): HTMLElement {
  lockDocument(false);
  const retryBtn = h('button', {
    class: 'btn', id: 'ai-unavail-retry', type: 'button',
    onclick: async () => {
      retryBtn.disabled = true;
      await refreshAi(true);
      // Still unreachable: no state change redraws the page, so it is drawn here (the button comes back)
      render();
    },
  }, t('common.retry'));
  return h('div', { class: 'page page-ai' },
    h('header', { class: 'page-head ai-head' }, h('h1', null, t('ai.title'))),
    h('div', { class: 'callout warn ai-unavail', role: 'alert' }, h('p', null, t(isAdmin() ? 'ai.err.unavailableAdmin' : 'ai.err.unavailable')), h('div', { class: 'acts' }, retryBtn)));
}

/**
 * #/ai for a user who cannot use AI and has no chats or memories to read: says why and what to do (instead of a bare
 * "Not allowed"). The cause is read from the AI status: the AI+ function paused, the AI+ feature off for everyone, or the
 * account without it. Nothing here names the ai-service container (administrators see that only in AI settings).
 */
export function renderAiNoAccess(): HTMLElement {
  lockDocument(false);
  const st = aiStatus();
  const key = st.state === 'ready' ? noAccessKey(st) : 'ai.noAccess.noFn';
  return h('div', { class: 'page page-ai' },
    h('header', { class: 'page-head ai-head' }, h('h1', null, t('ai.title'))),
    h('div', { class: 'callout note ai-noaccess', id: 'ai-noaccess', role: 'status' }, h('p', null, t(key))));
}

export function renderAiChat(m: ChatMode): HTMLElement {
  installOnce();
  mode = m;
  // Up to 900 px the document is locked: only the lists scroll (CH-1, CH-13)
  lockDocument(phoneScreen());
  if (!entered) enter();
  if (drawerOpen && !onePane()) resetDrawer();
  const root = h('div', { class: 'page page-ai' + (m === 'readOnly' ? ' ro' : '') + (tab === 'chats' ? ' conv-on' : '') },
    // v2.3: phones have no title row (the chat header takes its place)
    h('header', { class: 'page-head ai-head' },
      // Free mode on (administrators, chosen in Settings): a green tag next to the title; nothing in normal mode
      // A-OWNER-83: the tag carries its full and its short text; the title row shows the short one when it is tight
      h('div', { class: 'ai-title' }, h('h1', null, t('ai.title')),
        isAdmin() && aiFreeMode() ? h('span', { class: 'ai-mode-tag', role: 'status', title: t('ai.freeTag') },
          h('span', { class: 'sr-only' }, t('ai.freeTag')),
          h('span', { class: 'ai-mode-l', 'aria-hidden': 'true' }, t('ai.freeTag')),
          h('span', { class: 'ai-mode-s', 'aria-hidden': 'true' }, t('ai.freeTagShort'))) : null)),
    // Wide screens: the line under the title; up to 900 px it sits under the composer / the sidebar instead (CH-3)
    h('p', { class: 'ai-notice ai-notice-top' }, miniIcon('info', 'ai-mi'), h('span', null, t('ai.adminNotice'))),
    tab === 'chats' ? chatsView() : panesView(memoryPane()),
    h('div', { class: 'sr-only', id: 'ai-sr', role: 'status', 'aria-live': 'polite' }));
  // CH-13: every other text field (suggestion, memory edits) also undoes iOS's page pan while it has the focus
  const otherField = (e: Event): boolean => {
    const el = e.target as HTMLElement | null;
    return !!el && (el.tagName === 'INPUT' || el.tagName === 'TEXTAREA') && el.id !== 'ai-input';
  };
  root.addEventListener('focusin', (e) => {
    if (otherField(e) && document.documentElement.classList.contains('no-doc-scroll')) startPanWatch();
  });
  root.addEventListener('focusout', (e) => {
    if (otherField(e)) stopPanWatch();
  });
  // After the page is in the document: the composer's height, and the conversation at its end
  queueMicrotask(() => {
    if (!root.isConnected) return;
    fitHead();
    if (tab !== 'chats') {
      listObserver?.disconnect();
      listObserver = null;
      liftToastAbove(null);
      return;
    }
    // Toasts sit above the composer's footer (SH-4)
    liftToastAbove(composer && full() ? composer.dock : null);
    if (composer) autoGrow(composer.input);
    syncComposerHeight();
    watchList();
    if (stick) toBottom();
  });
  return root;
}
