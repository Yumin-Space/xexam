// v3.1: web push. The server key and courier state, this device's subscription, and the test notification.
import type { Device, PushConfig, PushSubscribeBody, PushTestResponse } from '../types/api.ts';
import { getJson, postJson } from './client.ts';

export const fetchPushConfig = (): Promise<PushConfig> => getJson<PushConfig>('/api/push/config');
export const subscribePush = (b: PushSubscribeBody): Promise<{ device: Device }> => postJson<{ device: Device }>('/api/push/subscribe', b);
export const sendPushTest = (): Promise<PushTestResponse> => postJson<PushTestResponse>('/api/push/test', {});
