// Sign-in (v0.6): who is signed in, sign in / out, and changing one's own password.
import type { AuthState, AuthUser, AuthUserResponse, LoginRequest, LoginResponse, SessionInfo } from '../types/api.ts';
import { getJson, postJson } from './client.ts';

export const fetchAuthState = (): Promise<AuthState> => getJson<AuthState>('/api/auth/state');
/** v1.3: `remember` keeps the sign-in for 30 days (administrators: fixed rule); the answer carries the session rule */
export const loginSession = async (username: string, password: string, remember = false): Promise<{ user: AuthUser; session: SessionInfo | null }> => {
  const body: LoginRequest = { username, password, remember };
  const a = await postJson<LoginResponse>('/api/auth/login', body);
  return { user: a.user, session: a.session ?? null };
};
export const login = async (username: string, password: string, remember = false): Promise<AuthUser> =>
  (await loginSession(username, password, remember)).user;
export const logout = (): Promise<{ ok: boolean }> => postJson<{ ok: boolean }>('/api/auth/logout');
/**
 * Sets a new password for the signed-in user. `current` is required unless the user must change the password (then the
 * session proves who they are). The user's other sessions are signed out.
 */
export const changePassword = async (password: string, current?: string, displayName?: string): Promise<AuthUser> => {
  const body: Record<string, string> = current === undefined ? { password } : { current, password };
  // v2.6: the first sign-in of a user without a display name sends the chosen one with the new password
  if (displayName !== undefined) body.displayName = displayName;
  return (await postJson<AuthUserResponse>('/api/auth/password', body)).user;
};
/** v2.6: sets or changes the signed-in user's display name (用户名); the answer is the user with the new name */
export const setDisplayName = async (displayName: string): Promise<AuthUser> =>
  (await postJson<AuthUserResponse>('/api/auth/name', { displayName })).user;
