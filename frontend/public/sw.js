// Xexam service worker (v3.1): push notifications only. It never intercepts a request and keeps no offline copy of the app.
// It shows what the server sent (the title and the body are already written in the user's language), opens the right page
// when a notification is tapped, and re-subscribes when the browser replaces the push subscription.
// Plain JavaScript, no build step: it is served as it is from /sw.js.
'use strict';

self.addEventListener('install', () => {
  self.skipWaiting();
});

self.addEventListener('activate', (event) => {
  event.waitUntil(self.clients.claim());
});

/** A click target inside the app: "/#/..." only, anything else opens the start page */
function safeUrl(u) {
  return typeof u === 'string' && u.startsWith('/#/') && u.length <= 512 ? u : '/';
}

/** The text shown when the payload is missing or unreadable (the server normally sends a finished text) */
function fallbackBody() {
  const lang = String((self.navigator && self.navigator.language) || '').toLowerCase();
  if (lang.startsWith('zh')) return '你有一条新通知';
  if (lang.startsWith('ja')) return '新しいお知らせがあります';
  return 'You have a new notification';
}

function readPayload(event) {
  let d = null;
  try {
    d = event.data ? event.data.json() : null;
  } catch (e) {
    d = null;
  }
  const ok = d && typeof d === 'object' && typeof d.title === 'string' && typeof d.body === 'string'
    && typeof d.tag === 'string' && typeof d.url === 'string';
  if (!ok) return { title: 'Xexam', body: fallbackBody(), tag: 'xexam', url: '/', lang: '' };
  return { title: d.title, body: d.body, tag: d.tag, url: safeUrl(d.url), lang: typeof d.lang === 'string' ? d.lang.slice(0, 35) : '' };
}

self.addEventListener('push', (event) => {
  const p = readPayload(event);
  event.waitUntil(self.registration.showNotification(p.title, {
    body: p.body,
    tag: p.tag,
    renotify: false,
    icon: '/icon-192.png',
    badge: '/badge-96.png',
    lang: p.lang || (self.navigator && self.navigator.language) || '',
    data: { url: p.url },
    timestamp: Date.now(),
  }));
});

self.addEventListener('notificationclick', (event) => {
  event.notification.close();
  const data = event.notification.data;
  const url = safeUrl(data && data.url);
  event.waitUntil((async () => {
    const list = await self.clients.matchAll({ type: 'window', includeUncontrolled: true });
    for (const c of list) {
      let same = false;
      try {
        same = new URL(c.url).origin === self.location.origin;
      } catch (e) {
        same = false;
      }
      if (!same) continue;
      try {
        const focused = typeof c.focus === 'function' ? await c.focus() : c;
        (focused || c).postMessage({ type: 'xexam.open', url });
        return;
      } catch (e) {
        // try the next window, or open a new one
      }
    }
    await self.clients.openWindow(url);
  })());
});

/** base64url (no padding) of an ArrayBuffer */
function b64url(buf) {
  if (!buf) return '';
  const bytes = new Uint8Array(buf);
  let s = '';
  for (let i = 0; i < bytes.length; i++) s += String.fromCharCode(bytes[i]);
  return self.btoa(s).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '');
}

/** The server key as bytes, for applicationServerKey */
function keyBytes(b64) {
  const s = String(b64).replace(/-/g, '+').replace(/_/g, '/');
  const bin = self.atob(s + '='.repeat((4 - (s.length % 4)) % 4));
  const out = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
  return out;
}

// The browser replaced or dropped the subscription: subscribe again with the server's key and tell the server. Every failure
// is ignored; the app confirms the subscription again the next time it opens.
self.addEventListener('pushsubscriptionchange', (event) => {
  event.waitUntil((async () => {
    try {
      const res = await fetch('/api/push/config', { credentials: 'same-origin' });
      if (!res.ok) return;
      const config = await res.json();
      if (!config || typeof config.publicKey !== 'string' || typeof config.keyId !== 'string') return;
      const sub = await self.registration.pushManager.subscribe({
        userVisibleOnly: true,
        applicationServerKey: keyBytes(config.publicKey),
      });
      const body = JSON.stringify({
        endpoint: sub.endpoint,
        keys: { p256dh: b64url(sub.getKey('p256dh')), auth: b64url(sub.getKey('auth')) },
        expirationTime: sub.expirationTime ?? null,
        keyId: config.keyId,
      });
      await fetch('/api/push/subscribe', {
        method: 'POST',
        credentials: 'same-origin',
        headers: { 'Content-Type': 'application/json' },
        body,
      });
    } catch (e) {
      // ignored on purpose
    }
  })());
});

// The page can ask the new worker to take over at once (used after an update).
self.addEventListener('message', (event) => {
  const d = event.data;
  if (d && d.type === 'xexam.skipWaiting') self.skipWaiting();
});
