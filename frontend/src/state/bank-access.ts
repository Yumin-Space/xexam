// Question-bank access and the bank-manager permission (v0.6.5), as pure functions the pages share and the tests check:
// - who may open Question banks (administrators, and plain users with the "bank manager" permission) and which banks they
//   manage there (the server marks each bank `manageable`);
// - what a row of User management says about a user's banks (v0.7: the chosen existing banks by name and whether the banks
//   imported later are included; administrators always see every bank), and the checklist draft behind the "Question
//   banks" dialog and the Add user form;
// - the change an Edit dialog sends (only the fields that changed);
// - v0.8: the compact bank controls ("Manage banks", "Select all", "Deselect all" on top of the list, "Include banks
//   imported later" as its first entry) and the change the "Change" dialog sends (banks, later and Manage banks); the menu
//   entry of AI Chat (from the AI status).
// The backend enforces every rule; these functions only decide what the pages show and send. No DOM, no i18n: the pages turn
// the results into text, so this file runs under `node --test` as it is.
import type { AuthUser, BankSummary, Role, UserRow, UserUpdate } from '../types/api.ts';

/** The fields of a user that decide what they may do with question banks */
type Holder = Pick<AuthUser, 'role' | 'canManageBanks'>;

/** Administrators, and plain users with the bank-manager permission, may open Question banks. */
export function mayManageBanks(u: Holder | null | undefined): boolean {
  return !!u && (u.role === 'admin' || u.canManageBanks === true);
}

/**
 * How the Question banks page is shown: administrators manage every bank and see each bank's owner; a bank manager sees and
 * manages only their own banks; anyone else gets the "Not allowed" page.
 */
export type BankPageView = 'admin' | 'manager' | 'forbidden';

export function bankPageView(u: Holder | null | undefined): BankPageView {
  if (!u) return 'forbidden';
  if (u.role === 'admin') return 'admin';
  return u.canManageBanks ? 'manager' : 'forbidden';
}

/** What a menu entry needs: an administrator, the permission to manage question banks, or (v0.8) AI Chat */
export type NavNeed = 'admin' | 'banks' | 'ai';

/** The part of the AI status (state/ai.ts) the menu looks at */
export interface AiNavStatus {
  state: 'loading' | 'unavailable' | 'ready';
  allowed?: boolean;
  hasData?: boolean;
}

/**
 * Whether the menu shows an entry. AI Chat (v0.8): when the user can use AI now, or has chats or memories to read and
 * delete (hidden while the status is loading or the AI service cannot be reached).
 */
export function navAllowed(need: NavNeed | undefined, u: Holder | null | undefined, ai?: AiNavStatus | null): boolean {
  if (!need) return true;
  if (need === 'admin') return u?.role === 'admin';
  if (need === 'ai') return !!u && ai?.state === 'ready' && (ai.allowed === true || ai.hasData === true);
  return mayManageBanks(u);
}

/**
 * The banks the Question banks page lists and offers as "Update a bank" targets: the ones the server marks manageable (every
 * bank for an administrator, their own for a bank manager). A bank without the flag (an older server) counts as manageable,
 * because only administrators could open the page before v0.6.5.
 */
export function manageableBanks<B extends Pick<BankSummary, 'manageable'>>(banks: readonly B[]): B[] {
  return banks.filter((b) => b.manageable !== false);
}

/** Whose upload a bank is, as the administrator's bank list says it: nobody known, the viewer, or someone else */
export type OwnerView = { kind: 'none' } | { kind: 'you' } | { kind: 'other'; name: string };

export function ownerView(b: Pick<BankSummary, 'ownerId' | 'owner'>, viewerId: number | null | undefined): OwnerView {
  if (b.ownerId == null) return { kind: 'none' };
  if (viewerId != null && b.ownerId === viewerId) return { kind: 'you' };
  return b.owner ? { kind: 'other', name: b.owner } : { kind: 'none' };
}

// ---------------------------------------------------------------- what a row of User management says

type NamedBank = Pick<BankSummary, 'id' | 'name'>;

/**
 * The bank access of one user, ready to be put into words (v0.7: two independent settings, the chosen existing banks and
 * "Include banks imported later"):
 * - `admin`: administrators see and manage every bank (the stored settings do not matter);
 * - `all`: every bank there is now is chosen and the banks imported later are included (and there is at least one bank);
 * - `some`: the chosen banks that still exist, by name in the order of the bank list (`names.length` is the count shown);
 *   `later`: the banks imported later are added too;
 * - `none`: no bank chosen; `later`: the banks imported later will be added (so the user sees none for now).
 * `own`: names of the banks a bank manager owns that the setting does not already cover (they see those too, while they keep
 * the permission). Always empty for administrators.
 */
export type AccessView =
  | { kind: 'admin'; own: string[] }
  | { kind: 'all'; own: string[] }
  | { kind: 'some'; names: string[]; later: boolean; own: string[] }
  | { kind: 'none'; later: boolean; own: string[] };

export function accessView(
  u: Pick<UserRow, 'role' | 'banks' | 'newBanks' | 'canManageBanks' | 'ownedBanks'>, banks: readonly NamedBank[],
): AccessView {
  if (u.role === 'admin') return { kind: 'admin', own: [] };
  const chosen = new Set(u.banks);
  const owned = new Set(u.canManageBanks ? u.ownedBanks : []);
  const names = banks.filter((b) => chosen.has(b.id)).map((b) => b.name);
  const own = banks.filter((b) => owned.has(b.id) && !chosen.has(b.id)).map((b) => b.name);
  const later = u.newBanks === true;
  if (later && banks.length > 0 && names.length === banks.length) return { kind: 'all', own };
  return names.length ? { kind: 'some', names, later, own } : { kind: 'none', later, own };
}

/** Names of the banks a user owns (the delete dialog says they move to the administrator), in the order of the bank list */
export function ownedBankNames(u: Pick<UserRow, 'ownedBanks'>, banks: readonly NamedBank[]): string[] {
  const owned = new Set(u.ownedBanks);
  return banks.filter((b) => owned.has(b.id)).map((b) => b.name);
}

// ---------------------------------------------------------------- the checklist ("Question banks" dialog, Add user form)

/**
 * The checklist while it is being edited: `ids` are the ticked existing banks, `later` is "Include banks imported later",
 * `manage` (v0.8) is "Manage banks" (the bank-manager permission). They are independent: "Select all" and "Deselect all"
 * change only the listed banks, and unticking a bank never touches `later` or `manage`.
 */
export interface AccessDraft {
  ids: number[];
  later: boolean;
  manage: boolean;
}

/** The draft of a stored setting (a new user starts with nothing ticked, `later` and `manage` off: `draftOf([], false)`) */
export function draftOf(banks: readonly number[], later: boolean, manage = false): AccessDraft {
  return { ids: [...new Set(banks)].sort((a, b) => a - b), later, manage };
}

/** Ticks or unticks one bank */
export function toggleBank(d: AccessDraft, id: number, on: boolean): AccessDraft {
  const ids = new Set(d.ids);
  if (on) ids.add(id);
  else ids.delete(id);
  return { ...d, ids: [...ids].sort((a, b) => a - b) };
}

/** Whether a bank's box shows ticked */
export const isTicked = (d: AccessDraft, id: number): boolean => d.ids.includes(id);

/** What "Select all" shows for the listed `banks`: every one ticked, none, or some (the indeterminate state) */
export type SelectAllState = 'all' | 'none' | 'some';

export function selectAllState(d: AccessDraft, banks: readonly Pick<BankSummary, 'id'>[]): SelectAllState {
  const n = banks.filter((b) => d.ids.includes(b.id)).length;
  if (n === 0) return 'none';
  return n === banks.length ? 'all' : 'some';
}

/** v0.8 "Select all": ticks every listed bank; `later` and `manage` stay */
export function selectAllBanks(d: AccessDraft, banks: readonly Pick<BankSummary, 'id'>[]): AccessDraft {
  return { ...d, ids: [...new Set(banks.map((b) => b.id))].sort((a, b) => a - b) };
}

/** v0.8 "Deselect all": unticks every bank; `later` and `manage` stay */
export function deselectAllBanks(d: AccessDraft): AccessDraft {
  return { ...d, ids: [] };
}

/** What "Select all" / "Deselect all" show: every listed bank ticked, none, or some */
export const toolState = (d: AccessDraft, banks: readonly Pick<BankSummary, 'id'>[]): SelectAllState => selectAllState(d, banks);

/** "Select all" changed: from all ticked it unticks every bank, otherwise it ticks every listed bank; `later` stays */
export function toggleAll(d: AccessDraft, banks: readonly Pick<BankSummary, 'id'>[]): AccessDraft {
  return selectAllState(d, banks) === 'all' ? deselectAllBanks(d) : selectAllBanks(d, banks);
}

/** The setting to send */
export interface AccessChoice {
  banks: number[];
  newBanks: boolean;
  /** v0.8: "Manage banks" (the stored bank-manager permission) */
  canManageBanks: boolean;
}

/**
 * The setting to send: the ticked banks that are still in `banks` (a bank deleted while the dialog was open is dropped
 * instead of making the server refuse the whole change), ascending, "Include banks imported later" and "Manage banks".
 */
export function accessOf(d: AccessDraft, banks: readonly Pick<BankSummary, 'id'>[]): AccessChoice {
  const known = new Set(banks.map((b) => b.id));
  return { banks: d.ids.filter((id) => known.has(id)), newBanks: d.later, canManageBanks: d.manage };
}

/** Whether two settings are the same: the banks (in any order), "later" and "Manage banks" */
export function sameAccess(a: AccessChoice, b: AccessChoice): boolean {
  const x = [...new Set(a.banks)].sort((p, q) => p - q);
  const y = [...new Set(b.banks)].sort((p, q) => p - q);
  return a.newBanks === b.newBanks && a.canManageBanks === b.canManageBanks && x.length === y.length && x.every((v, i) => v === y[i]);
}

/** What the "Change" dialog sends, and what changed (for the toast) */
export interface AccessChange {
  /** null: nothing changed */
  body: UserUpdate | null;
  /** The banks or "Include banks imported later" changed */
  banks: boolean;
  /** "Manage banks" changed */
  manage: boolean;
}

/**
 * The body of POST /api/users/update for the "Change" dialog (v0.8): `{id}`, plus `banks` and `newBanks` when either
 * changed, plus `canManageBanks` when it changed; null when nothing did.
 */
export function accessChange(
  u: Pick<UserRow, 'id' | 'banks' | 'newBanks' | 'canManageBanks'>, d: AccessDraft, banks: readonly Pick<BankSummary, 'id'>[],
): AccessChange {
  const next = accessOf(d, banks);
  const banksChanged = !sameAccess({ ...next, canManageBanks: false }, { banks: u.banks, newBanks: u.newBanks, canManageBanks: false });
  const manageChanged = next.canManageBanks !== u.canManageBanks;
  if (!banksChanged && !manageChanged) return { body: null, banks: false, manage: false };
  const body: UserUpdate = { id: u.id };
  if (banksChanged) {
    body.banks = next.banks;
    body.newBanks = next.newBanks;
  }
  if (manageChanged) body.canManageBanks = next.canManageBanks;
  return { body, banks: banksChanged, manage: manageChanged };
}

// ---------------------------------------------------------------- the Edit dialog

export interface EditDraft {
  /** Already trimmed */
  username: string;
  role: Role;
  enabled: boolean;
  canManageBanks: boolean;
}

/**
 * The body of POST /api/users/update for an Edit dialog: only the fields that changed, or null when nothing did. The
 * bank-manager permission is the stored flag; it is sent only when it changed (also for an administrator, where it matters
 * again if they are made a plain user later).
 */
export function editChange(u: Pick<UserRow, 'id' | 'username' | 'role' | 'disabled' | 'canManageBanks'>, d: EditDraft): UserUpdate | null {
  const body: UserUpdate = { id: u.id };
  if (d.username !== u.username) body.username = d.username;
  if (d.role !== u.role) body.role = d.role;
  if (d.enabled === u.disabled) body.disabled = !d.enabled;
  if (d.canManageBanks !== u.canManageBanks) body.canManageBanks = d.canManageBanks;
  return Object.keys(body).length > 1 ? body : null;
}
