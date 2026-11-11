// Question-bank access and the bank-manager permission (v0.6.5): who may open Question banks, which banks they manage, what
// a row of User management says, the checklist draft and the Edit dialog's change. Made-up banks and users only.
import assert from 'node:assert/strict';
import test from 'node:test';
import {
  accessChange, accessOf, accessView, bankPageView, deselectAllBanks, draftOf, editChange, isTicked, manageableBanks, mayManageBanks,
  navAllowed, ownedBankNames, ownerView, sameAccess, selectAllBanks, selectAllState, toggleAll, toggleBank, toolState,
} from './bank-access.ts';
import type { UserRow } from '../types/api.ts';

const admin = { role: 'admin' as const, canManageBanks: true };
const manager = { role: 'user' as const, canManageBanks: true };
const plain = { role: 'user' as const, canManageBanks: false };

const BANKS = [
  { id: 1, name: 'Demo Alpha' },
  { id: 2, name: 'Demo Beta' },
  { id: 5, name: 'Demo Gamma' },
];

const row = (o: Partial<UserRow> = {}): UserRow => ({
  id: 7, username: 'demo-user', role: 'user', disabled: false, createdAt: '2026-01-01 10:00:00', lastLoginAt: null,
  mustChangePassword: false, records: 0, banks: [], newBanks: false, canManageBanks: false, ai: false, ownedBanks: [], lock: null, aiSuspended: false, ...o,
});

test('administrators and bank managers may open Question banks; plain users and nobody may not', () => {
  assert.equal(mayManageBanks(admin), true);
  // The stored flag of an administrator does not matter
  assert.equal(mayManageBanks({ role: 'admin', canManageBanks: false }), true);
  assert.equal(mayManageBanks(manager), true);
  assert.equal(mayManageBanks(plain), false);
  assert.equal(mayManageBanks(null), false);
  assert.equal(bankPageView(admin), 'admin');
  assert.equal(bankPageView(manager), 'manager');
  assert.equal(bankPageView(plain), 'forbidden');
  assert.equal(bankPageView(undefined), 'forbidden');
});

test('the menu shows Question banks to managers and User management to administrators only', () => {
  assert.equal(navAllowed(undefined, plain), true);
  assert.equal(navAllowed('banks', plain), false);
  assert.equal(navAllowed('banks', manager), true);
  assert.equal(navAllowed('banks', admin), true);
  assert.equal(navAllowed('admin', manager), false);
  assert.equal(navAllowed('admin', admin), true);
  assert.equal(navAllowed('admin', null), false);
});

test('the banks page lists only the banks the server marks manageable', () => {
  const banks = [{ id: 1, manageable: true }, { id: 2, manageable: false }, { id: 3, manageable: true }];
  assert.deepEqual(manageableBanks(banks).map((b) => b.id), [1, 3]);
  assert.deepEqual(manageableBanks([]), []);
});

test('the owner of a bank: nobody known, the viewer, or someone else by name', () => {
  assert.deepEqual(ownerView({ ownerId: null, owner: null }, 1), { kind: 'none' });
  assert.deepEqual(ownerView({ ownerId: 1, owner: 'demo-admin' }, 1), { kind: 'you' });
  assert.deepEqual(ownerView({ ownerId: 4, owner: 'demo-manager' }, 1), { kind: 'other', name: 'demo-manager' });
  // An owner id whose name is not known says nothing rather than a wrong name
  assert.deepEqual(ownerView({ ownerId: 4, owner: null }, 1), { kind: 'none' });
});

test('a row says administrators see every bank, whatever is stored', () => {
  assert.deepEqual(accessView(row({ role: 'admin', banks: [], ownedBanks: [1] }), BANKS), { kind: 'admin', own: [] });
});

test('a row lists the chosen banks by name in the order of the bank list, and whether later banks are included', () => {
  // every bank there is now, and the later ones
  assert.deepEqual(accessView(row({ banks: [1, 2, 5], newBanks: true }), BANKS), { kind: 'all', own: [] });
  // every bank there is now, without the later ones: counted and named like any choice
  assert.deepEqual(accessView(row({ banks: [5, 2, 1] }), BANKS),
    { kind: 'some', names: ['Demo Alpha', 'Demo Beta', 'Demo Gamma'], later: false, own: [] });
  assert.deepEqual(accessView(row({ banks: [5, 1] }), BANKS), { kind: 'some', names: ['Demo Alpha', 'Demo Gamma'], later: false, own: [] });
  assert.deepEqual(accessView(row({ banks: [5, 1], newBanks: true }), BANKS),
    { kind: 'some', names: ['Demo Alpha', 'Demo Gamma'], later: true, own: [] });
  assert.deepEqual(accessView(row({ banks: [] }), BANKS), { kind: 'none', later: false, own: [] });
  assert.deepEqual(accessView(row({ banks: [], newBanks: true }), BANKS), { kind: 'none', later: true, own: [] });
  // no bank at all yet: "none for now, the later ones are added", not "all"
  assert.deepEqual(accessView(row({ banks: [], newBanks: true }), []), { kind: 'none', later: true, own: [] });
  // An id the bank list does not know (deleted meanwhile) is not counted
  assert.deepEqual(accessView(row({ banks: [9] }), BANKS), { kind: 'none', later: false, own: [] });
});

test('a bank manager also sees their own banks; a plain user who lost the permission does not', () => {
  assert.deepEqual(accessView(row({ banks: [1], canManageBanks: true, ownedBanks: [1, 2] }), BANKS),
    { kind: 'some', names: ['Demo Alpha'], later: false, own: ['Demo Beta'] });
  assert.deepEqual(accessView(row({ banks: [], canManageBanks: true, ownedBanks: [5] }), BANKS), { kind: 'none', later: false, own: ['Demo Gamma'] });
  assert.deepEqual(accessView(row({ banks: [], canManageBanks: false, ownedBanks: [5] }), BANKS), { kind: 'none', later: false, own: [] });
  assert.deepEqual(accessView(row({ banks: [1, 2, 5], newBanks: true, canManageBanks: true, ownedBanks: [5] }), BANKS), { kind: 'all', own: [] });
  assert.deepEqual(ownedBankNames(row({ ownedBanks: [5, 2] }), BANKS), ['Demo Beta', 'Demo Gamma']);
});

test('the checklist: a new user starts with nothing ticked; the two settings are independent', () => {
  const empty = draftOf([], false);
  assert.deepEqual(empty, { ids: [], later: false, manage: false });
  assert.deepEqual(accessOf(empty, BANKS), { banks: [], newBanks: false, canManageBanks: false });

  let d = toggleBank(toggleBank(empty, 5, true), 1, true);
  assert.deepEqual(d, { ids: [1, 5], later: false, manage: false });
  assert.deepEqual(accessOf(d, BANKS), { banks: [1, 5], newBanks: false, canManageBanks: false });
  d = toggleBank(d, 5, false);
  assert.deepEqual(accessOf(d, BANKS), { banks: [1], newBanks: false, canManageBanks: false });
  assert.equal(isTicked(d, 1), true);
  assert.equal(isTicked(d, 2), false);

  // the owner's example: General knowledge ticked, Kitchen basics not, banks imported later included
  const later = { ...d, later: true };
  assert.deepEqual(accessOf(later, BANKS), { banks: [1], newBanks: true, canManageBanks: false });
  // unticking a bank keeps "later" (and "Manage banks")
  assert.deepEqual(toggleBank(later, 1, false), { ids: [], later: true, manage: false });
  assert.deepEqual(toggleBank({ ...later, manage: true }, 1, false), { ids: [], later: true, manage: true });

  assert.deepEqual(draftOf([5, 1, 5], true), { ids: [1, 5], later: true, manage: false });
  assert.deepEqual(draftOf([2], false, true), { ids: [2], later: false, manage: true });
});

test('"Select all" ticks or unticks every listed bank and shows checked / unchecked / indeterminate', () => {
  const none = draftOf([], true);
  assert.equal(selectAllState(none, BANKS), 'none');
  const all = toggleAll(none, BANKS);
  assert.deepEqual(all, { ids: [1, 2, 5], later: true, manage: false }, '"later" is not touched');
  assert.equal(selectAllState(all, BANKS), 'all');
  assert.deepEqual(toggleAll(all, BANKS), { ids: [], later: true, manage: false });
  const some = draftOf([2], false);
  assert.equal(selectAllState(some, BANKS), 'some');
  assert.deepEqual(toggleAll(some, BANKS), { ids: [1, 2, 5], later: false, manage: false }, 'from indeterminate: every bank');
  // a ticked id that is no longer listed does not count
  assert.equal(selectAllState(draftOf([9], false), BANKS), 'none');
  assert.equal(selectAllState(draftOf([], false), []), 'none');
});

test('the checklist drops a bank that was deleted while it was open', () => {
  assert.deepEqual(accessOf({ ids: [1, 3, 5], later: false, manage: false }, BANKS), { banks: [1, 5], newBanks: false, canManageBanks: false });
});

test('two settings are the same regardless of order; "Manage banks" counts too (v0.8)', () => {
  const c = (banks: number[], newBanks: boolean, canManageBanks = false) => ({ banks, newBanks, canManageBanks });
  assert.equal(sameAccess(c([2, 1], false), c([1, 2], false)), true);
  assert.equal(sameAccess(c([1], false), c([1, 2], false)), false);
  assert.equal(sameAccess(c([1], true), c([1], false)), false);
  assert.equal(sameAccess(c([], true), c([], true)), true);
  assert.equal(sameAccess(c([1], false, true), c([1], false, false)), false);
  assert.equal(sameAccess(c([1], false, true), c([1], false, true)), true);
});

test('the Edit dialog sends only what changed, including the bank-manager permission', () => {
  const u = row({ id: 3, username: 'demo-user', role: 'user', disabled: false, canManageBanks: false });
  const same = { username: 'demo-user', role: 'user' as const, enabled: true, canManageBanks: false };
  assert.equal(editChange(u, same), null);
  assert.deepEqual(editChange(u, { ...same, canManageBanks: true }), { id: 3, canManageBanks: true });
  assert.deepEqual(editChange(u, { ...same, username: 'demo-2', enabled: false }), { id: 3, username: 'demo-2', disabled: true });
  assert.deepEqual(editChange(u, { ...same, role: 'admin' }), { id: 3, role: 'admin' });
  assert.deepEqual(editChange(row({ id: 3, disabled: true }), { ...same, enabled: true }), { id: 3, disabled: false });
});

// ---------------------------------------------------------------- v0.8: the compact bank controls and AI Chat in the menu

test('"Select all" and "Deselect all" change only the listed banks; "later" and "Manage banks" stay', () => {
  const d = draftOf([2], true, true);
  assert.deepEqual(selectAllBanks(d, BANKS), { ids: [1, 2, 5], later: true, manage: true });
  assert.deepEqual(deselectAllBanks(d), { ids: [], later: true, manage: true });
  assert.deepEqual(selectAllBanks(draftOf([], false), []), { ids: [], later: false, manage: false });
  // the draft itself is not changed
  assert.deepEqual(d, { ids: [2], later: true, manage: true });
});

test('the tool states: all, some, and none (then "Deselect all" shows ticked and refuses the click)', () => {
  assert.equal(toolState(draftOf([1, 2, 5], false), BANKS), 'all');
  assert.equal(toolState(draftOf([5], false), BANKS), 'some');
  assert.equal(toolState(draftOf([], true), BANKS), 'none');
  // ids no longer listed do not count
  assert.equal(toolState(draftOf([9], false), BANKS), 'none');
  // no banks at all: "none" (both tools are disabled then)
  assert.equal(toolState(draftOf([], false), []), 'none');
});

test('the "Change" dialog: only Manage banks changed sends {id, canManageBanks}', () => {
  const u = row({ id: 7, banks: [1], newBanks: false, canManageBanks: false });
  const same = draftOf([1], false, false);
  assert.deepEqual(accessChange(u, same, BANKS), { body: null, banks: false, manage: false });
  assert.deepEqual(accessChange(u, { ...same, manage: true }, BANKS), { body: { id: 7, canManageBanks: true }, banks: false, manage: true });
  // banks changed: banks and newBanks, not the permission
  assert.deepEqual(accessChange(u, draftOf([1, 5], false, false), BANKS),
    { body: { id: 7, banks: [1, 5], newBanks: false }, banks: true, manage: false });
  // only "later" changed counts as a bank change too
  assert.deepEqual(accessChange(u, draftOf([1], true, false), BANKS),
    { body: { id: 7, banks: [1], newBanks: true }, banks: true, manage: false });
  // both
  assert.deepEqual(accessChange(row({ id: 7, banks: [], canManageBanks: true }), draftOf([2], false, false), BANKS),
    { body: { id: 7, banks: [2], newBanks: false, canManageBanks: false }, banks: true, manage: true });
  // the order of the stored ids does not matter; a bank deleted meanwhile is dropped
  assert.deepEqual(accessChange(row({ id: 7, banks: [5, 1] }), draftOf([1, 5, 9], false), BANKS).body, null);
});

test('AI Chat is in the menu when the user can use AI now or has AI data; never while loading or unreachable', () => {
  const ready = (allowed: boolean, hasData: boolean) => ({ state: 'ready' as const, allowed, hasData });
  assert.equal(navAllowed('ai', plain, ready(true, false)), true);
  assert.equal(navAllowed('ai', plain, ready(false, true)), true, 'AI+ off but chats or memories to read');
  assert.equal(navAllowed('ai', plain, ready(false, false)), false, 'hidden, not "unavailable"');
  assert.equal(navAllowed('ai', admin, ready(true, false)), true);
  assert.equal(navAllowed('ai', plain, { state: 'loading' }), false);
  assert.equal(navAllowed('ai', plain, { state: 'unavailable' }), false);
  assert.equal(navAllowed('ai', plain, null), false);
  assert.equal(navAllowed('ai', null, ready(true, true)), false);
  // AI settings is an administrators' entry (always shown to them)
  assert.equal(navAllowed('admin', admin, { state: 'unavailable' }), true);
});
