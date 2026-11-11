// User management (administrators only). Every change answers with the whole, current list and (v0.8 round 2) whether
// AI+ is available, so the page always draws the AI+ switches from the newest answer.
import type { UserCreate, UserRow, UsersResponse, UserUpdate } from '../types/api.ts';
import { getJson, postJson } from './client.ts';

/** The list and whether AI+ is available, from one answer of `GET /api/users` or `POST /api/users/*` */
export interface UsersList {
  users: UserRow[];
  /** ai-service is connected and the AI+ service is on in AI settings: only then are the AI+ switches shown */
  aiAvailable: boolean;
}

/** An answer as the page uses it: `aiAvailable` and each row's `aiSuspended` count only when they are exactly true */
export function usersListOf(a: UsersResponse): UsersList {
  return {
    users: a.users.map((u) => ({ ...u, aiSuspended: u.aiSuspended === true })),
    aiAvailable: a.aiAvailable === true,
  };
}

const list = async (p: Promise<UsersResponse>): Promise<UsersList> => usersListOf(await p);

export const fetchUsers = (): Promise<UsersList> => list(getJson<UsersResponse>('/api/users'));
/**
 * The new user must choose their own password at the first sign-in. `banks`: the existing question banks they see (the
 * server's default is none); `newBanks`: "Include banks imported later" (default off); `canManageBanks`: may import banks
 * and manage their own (default false); `ai` (v0.8): the AI+ service (default false; `true` is refused with 409
 * `ai.serviceOff` while AI+ is not available).
 */
export const createUser = (body: UserCreate): Promise<UsersList> => list(postJson<UsersResponse>('/api/users/create', body));
/** Only the fields that change; an `ai` key (on or off) is refused with 409 `ai.serviceOff` while AI+ is not available */
export const updateUser = (change: UserUpdate): Promise<UsersList> => list(postJson<UsersResponse>('/api/users/update', change));
/** Sets a new initial password; the user is signed out and must choose a new password at the next sign-in */
export const resetUserPassword = (id: number, password: string): Promise<UsersList> =>
  list(postJson<UsersResponse>('/api/users/password', { id, password }));
/** Clears the sign-in lock of the user (back to round 1, five tries); also allowed for your own account */
export const unlockUser = (id: number): Promise<UsersList> => list(postJson<UsersResponse>('/api/users/unlock', { id }));
/** Deletes the user with their study records, wrong-answer notebook and settings */
export const deleteUser = (id: number): Promise<UsersList> => list(postJson<UsersResponse>('/api/users/delete', { id }));
