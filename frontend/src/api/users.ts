// User management (administrators only). Every change answers with the whole, current list and (v0.8 round 2) whether
// AI+ is available, so the page always draws the AI+ switches from the newest answer.
import type { GeoSource, UserCreate, UserRecordDetail, UserRecordsResponse, UserRow, UsersResponse, UserUpdate } from '../types/api.ts';
import { getJson, postJson } from './client.ts';

/** The list and whether AI+ is available, from one answer of `GET /api/users` or `POST /api/users/*` */
export interface UsersList {
  users: UserRow[];
  /** ai-service is connected and the AI+ service is on in AI settings: only then are the AI+ switches shown */
  aiAvailable: boolean;
  /** v1.3: the sign-in info eye (IP + approximate location); false on an older answer */
  signinInfo: boolean;
  /** v1.4: where the place of a sign-in comes from; `cloudflare` on an older answer */
  geoSource: GeoSource;
  /** v1.4: ai-service is connected (exactly true), whether or not AI+ is on */
  aiServiceUp: boolean;
}

/** An answer as the page uses it: `aiAvailable` and each row's `aiSuspended` count only when they are exactly true */
export function usersListOf(a: UsersResponse): UsersList {
  return {
    users: a.users.map((u) => ({
      ...u,
      canDownloadBanks: u.canDownloadBanks === true,
      aiSuspended: u.aiSuspended === true,
      lastActiveAt: typeof u.lastActiveAt === 'string' ? u.lastActiveAt : null,
      lastLoginIp: typeof u.lastLoginIp === 'string' ? u.lastLoginIp : null,
      lastLoginLocation: u.lastLoginLocation ?? null,
      lastActiveIp: typeof u.lastActiveIp === 'string' ? u.lastActiveIp : null,
      lastActiveLocation: u.lastActiveLocation ?? null,
      lastActiveUa: typeof u.lastActiveUa === 'string' ? u.lastActiveUa : null,
    })),
    aiAvailable: a.aiAvailable === true,
    signinInfo: a.signinInfo === true,
    geoSource: a.geoSource === 'api' ? 'api' : 'cloudflare',
    aiServiceUp: a.aiServiceUp === true,
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

/** v1.3: shows or hides the recorded sign-in IP + approximate location (closed: nothing new is recorded and saved values are hidden; nothing is deleted) */
export const setSigninInfo = (enabled: boolean): Promise<UsersList> => list(postJson<UsersResponse>('/api/users/signin-info', { enabled }));
/** v1.3: clears every saved sign-in IP and location (users, sessions, the activity log); cannot be undone */
export const clearSigninInfo = (): Promise<UsersList> => list(postJson<UsersResponse>('/api/users/signin-info/clear', {}));

/** v1.4: sets the place source back to Cloudflare request headers (switching to the external API goes through `enableGeo`, api/geo.ts) */
export const setGeoSource = (source: 'cloudflare'): Promise<UsersList> => list(postJson<UsersResponse>('/api/users/geo-source', { source }));

/** v2.7 (administrators, read only): the study-record list of one user, as `GET /api/records` answers it plus `user` */
export const fetchUserRecords = (uid: number): Promise<UserRecordsResponse> =>
  getJson<UserRecordsResponse>(`/api/users/records?user=${encodeURIComponent(String(uid))}`);
/** v2.7: one record of that user (404 when it is not theirs) */
export const fetchUserRecord = (uid: number, rid: number): Promise<UserRecordDetail> =>
  getJson<UserRecordDetail>(`/api/users/record?user=${encodeURIComponent(String(uid))}&id=${encodeURIComponent(String(rid))}`);
