// The service worker is plain JavaScript served as it is (public/sw.js). This test pins its contract: push only, never a
// request interceptor, the generic fallback texts, and the body of the re-subscription.
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

const src = readFileSync(new URL('../../public/sw.js', import.meta.url), 'utf8');
const listeners = [...src.matchAll(/addEventListener\(\s*['"]([a-z]+)['"]/g)].map((m) => m[1]);

test('sw.js has exactly the six listeners', () => {
  assert.deepEqual([...listeners].sort(), ['activate', 'install', 'message', 'notificationclick', 'push', 'pushsubscriptionchange']);
});

test('sw.js never listens for fetch and never caches', () => {
  assert.equal(listeners.includes('fetch'), false);
  assert.ok(!/addEventListener\(\s*['"]fetch['"]/.test(src));
  assert.ok(!src.includes('caches.'));
  assert.ok(!/importScripts/.test(src));
  assert.ok(!/^\s*(import|export)\s/m.test(src), 'no module syntax');
});

test('sw.js carries the three fallback texts', () => {
  assert.ok(src.includes('你有一条新通知'));
  assert.ok(src.includes('新しいお知らせがあります'));
  assert.ok(src.includes('You have a new notification'));
  assert.ok(src.includes("'xexam'"), 'fallback tag');
});

test('sw.js shows the notification the way the contract says', () => {
  for (const part of ['showNotification', 'renotify: false', "icon: '/icon-192.png'", "badge: '/badge-96.png'", 'timestamp: Date.now()']) {
    assert.ok(src.includes(part), part);
  }
  // a click opens or focuses a same-origin window and tells it where to go
  assert.ok(src.includes('matchAll'));
  assert.ok(src.includes("type: 'window'") && src.includes('includeUncontrolled: true'));
  assert.ok(src.includes("type: 'xexam.open'"));
  assert.ok(src.includes('openWindow'));
  assert.ok(src.includes("startsWith('/#/')"));
});

test('sw.js re-subscribes with endpoint, keys, expirationTime and keyId', () => {
  const i = src.indexOf("'pushsubscriptionchange'");
  assert.ok(i > 0);
  const body = src.slice(i);
  for (const word of ['/api/push/config', '/api/push/subscribe', 'userVisibleOnly: true', 'applicationServerKey', 'endpoint', 'p256dh', 'auth', 'expirationTime', 'keyId']) {
    assert.ok(body.includes(word), word);
  }
  assert.ok(body.includes("credentials: 'same-origin'"));
});

test('sw.js parses as a script', () => {
  assert.doesNotThrow(() => new Function(src));
});

test('manifest.webmanifest is the contract JSON', () => {
  const m = JSON.parse(readFileSync(new URL('../../public/manifest.webmanifest', import.meta.url), 'utf8'));
  assert.equal(m.name, 'Xexam');
  assert.equal(m.start_url, '/#/');
  assert.equal(m.scope, '/');
  assert.equal(m.display, 'standalone');
  assert.deepEqual(m.icons.map((i: { src: string; purpose: string }) => `${i.purpose}:${i.src}`), [
    'any:/icon-192.png', 'any:/icon-512.png', 'maskable:/icon-maskable-192.png', 'maskable:/icon-maskable-512.png',
  ]);
});
