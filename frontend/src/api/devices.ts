// v3.1: the signed-in devices (one row per browser) of the account, and of any user for administrators.
import type { Device, DeviceHello, DevicesResponse, UserDevicesResponse } from '../types/api.ts';
import { getJson, postJson } from './client.ts';

export const fetchDevices = (): Promise<DevicesResponse> => getJson<DevicesResponse>('/api/devices');
/** The app says hello at every start after sign-in (zone, installed app, notification permission) */
export const sayHello = (b: DeviceHello): Promise<{ device: Device }> => postJson<{ device: Device }>('/api/devices/hello', b);
export const signOutDevice = (id: number): Promise<DevicesResponse> => postJson<DevicesResponse>('/api/devices/signout', { id });
export const signOutOtherDevices = (): Promise<DevicesResponse> => postJson<DevicesResponse>('/api/devices/signout-others', {});
/** Administrators: the devices of one user */
export const fetchUserDevices = (user: number): Promise<UserDevicesResponse> =>
  getJson<UserDevicesResponse>(`/api/users/devices?user=${encodeURIComponent(String(user))}`);
/** Administrators: sign out one device of a user (`id`), or every device of the user (`id` null) */
export const signOutUserDevice = (user: number, id: number | null): Promise<UserDevicesResponse> =>
  postJson<UserDevicesResponse>('/api/users/devices/signout', { user, id });
