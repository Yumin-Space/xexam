// The OFL text of the blackboard-mode font must ship with the built frontend (Vite copies public/ into dist/).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { existsSync, readFileSync } from 'node:fs';

test('the Xiaolai OFL text sits in public/fonts', () => {
  const url = new URL('../../public/fonts/Xiaolai-OFL.txt', import.meta.url);
  assert.ok(existsSync(url));
  assert.match(readFileSync(url, 'utf8'), /SIL OPEN FONT LICENSE/i);
});
