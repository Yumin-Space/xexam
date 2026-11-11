// Question-bank access and the bank-manager permission (v0.6.5), as pure functions the pages share and the tests check:
// - who may open Question banks (administrators, and plain users with the "bank manager" permission) and which banks they
//   manage there (the server marks each bank `manageable`);
// - what a row of User management says about a user's banks (v0.7: the chosen existing banks by name and whether the banks
//   imported later are included; administrators always see every bank), and the checklist draft behind the "Question
//   banks" dialog and the Add user form;
// - the change an Edit dialog sends (only the fields that changed).
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

/** What a menu entry needs: an administrator, or the permission to manage question banks */
export type NavNeed = 'admin' | 'banks';

export function navAllowed(need: NavNeed | undefined, u: Holder | null | undefined): boolean {
  if (!need) return true;
  if (need === 'admin') return u?.role === 'admin';
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
 * The checklist while it is being edited: `ids` are the ticked existing banks, `later` is "Include banks imported later".
 * The two are independent: "Select all" only ticks every listed bank, and unticking a bank never touches `later`.
 */
export interface AccessDraft {
  ids: number[];
  later: boolean;
}

/** The draft of a stored setting (a new user starts with nothing ticked and `later` off: `draftOf([], false)`) */
export function draftOf(banks: readonly number[], later: boolean): AccessDraft {
  return { ids: [...new Set(banks)].sort((a, b) => a - b), later };
}

/** Ticks or unticks one bank */
export function toggleBank(d: AccessDraft, id: number, on: boolean): AccessDraft {
  const ids = new Set(d.ids);
  if (on) ids.add(id);
  else ids.delete(id);
  return { ids: [...ids].sort((a, b) => a - b), later: d.later };
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

/** "Select all" clicked: ticks every listed bank, or unticks them all when every one is ticked already; `later` stays */
export function toggleAll(d: AccessDraft, banks: readonly Pick<BankSummary, 'id'>[]): AccessDraft {
  const on = selectAllState(d, banks) !== 'all';
  return { ids: on ? [...new Set(banks.map((b) => b.id))].sort((a, b) => a - b) : [], later: d.later };
}

/** The setting to send */
export interface AccessChoice {
  banks: number[];
  newBanks: boolean;
}

/**
 * The setting to send: the ticked banks that are still in `banks` (a bank deleted while the dialog was open is dropped
 * instead of making the server refuse the whole change), ascending, and "Include banks imported later".
 */
export function accessOf(d: AccessDraft, banks: readonly Pick<BankSummary, 'id'>[]): AccessChoice {
  const known = new Set(banks.map((b) => b.id));
  return { banks: d.ids.filter((id) => known.has(id)), newBanks: d.later };
}

/** Whether two settings are the same (the order of the ids does not matter) */
export function sameAccess(a: AccessChoice, b: AccessChoice): boolean {
  const x = [...new Set(a.banks)].sort((p, q) => p - q);
  const y = [...new Set(b.banks)].sort((p, q) => p - q);
  return a.newBanks === b.newBanks && x.length === y.length && x.every((v, i) => v === y[i]);
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
