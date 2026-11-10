// User management (administrators only). Every change answers with the whole, current list.
import type { Role, UserRow, UsersResponse, UserUpdate } from '../types/api.ts';
import { getJson, postJson } from './client.ts';

const list = async (p: Promise<UsersResponse>): Promise<UserRow[]> => (await p).users;

export const fetchUsers = (): Promise<UserRow[]> => list(getJson<UsersResponse>('/api/users'));
/** The new user must choose their own password at the first sign-in */
export const createUser = (username: string, password: string, role: Role): Promise<UserRow[]> =>
  list(postJson<UsersResponse>('/api/users/create', { username, password, role }));
export const updateUser = (change: UserUpdate): Promise<UserRow[]> => list(postJson<UsersResponse>('/api/users/update', change));
/** Sets a new initial password; the user is signed out and must choose a new password at the next sign-in */
export const resetUserPassword = (id: number, password: string): Promise<UserRow[]> =>
  list(postJson<UsersResponse>('/api/users/password', { id, password }));
/** Clears the sign-in lock of the user (back to round 1, five tries); also allowed for your own account */
export const unlockUser = (id: number): Promise<UserRow[]> => list(postJson<UsersResponse>('/api/users/unlock', { id }));
/** Deletes the user with their study records, wrong-answer notebook and settings */
export const deleteUser = (id: number): Promise<UserRow[]> => list(postJson<UsersResponse>('/api/users/delete', { id }));
