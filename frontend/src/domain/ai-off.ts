// Which text explains a switched-off feature or connection (v1.9). Pure: no DOM; the caller passes the role.
import type { MsgKey } from '../i18n/index.ts';

/** The tip for a feature that is switched off globally (administrators are told where to turn it on) */
export const featureOffKey = (admin: boolean): MsgKey => (admin ? 'ai.err.featureOffAdmin' : 'ai.err.featureOff');

/** The text for a feature whose connection is not enabled */
export const connectionOffKey = (admin: boolean): MsgKey => (admin ? 'ai.err.connectionOffAdmin' : 'ai.err.connectionOff');
