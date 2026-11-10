// Sign-in (v0.6): who is signed in, sign in / out, and changing one's own password.
import type { AuthState, AuthUser, AuthUserResponse } from '../types/api.ts';
import { getJson, postJson } from './client.ts';

export const fetchAuthState = (): Promise<AuthState> => getJson<AuthState>('/api/auth/state');
export const login = async (username: string, password: string): Promise<AuthUser> =>
  (await postJson<AuthUserResponse>('/api/auth/login', { username, password })).user;
export const logout = (): Promise<{ ok: boolean }> => postJson<{ ok: boolean }>('/api/auth/logout');
/**
 * Sets a new password for the signed-in user. `current` is required unless the user must change the password (then the
 * session proves who they are). The user's other sessions are signed out.
 */
export const changePassword = async (password: string, current?: string): Promise<AuthUser> =>
  (await postJson<AuthUserResponse>('/api/auth/password', current === undefined ? { password } : { current, password })).user;
