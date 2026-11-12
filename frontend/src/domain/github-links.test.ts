import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { describe, it } from 'node:test';
import { GITHUB_URL, LINK_REL, RELEASES_URL } from './github-links.ts';

describe('footer links', () => {
  it('the owner addresses', () => {
    assert.equal(GITHUB_URL, 'https://github.com/Yumin-Space/xexam');
    assert.equal(RELEASES_URL, 'https://github.com/Yumin-Space/xexam/releases');
    assert.equal(LINK_REL, 'noopener noreferrer');
  });
  it('the version comes from /api/info, never a literal', () => {
    const src = readFileSync(new URL('../components/github-line.ts', import.meta.url), 'utf8');
    assert.ok(src.includes("t('footer.release', { version: S.info.version })"));
    assert.equal(/Xexam \d/.test(src), false);
  });
  it('every page kind draws the line', () => {
    for (const f of ['../components/shell.ts', '../pages/auth.ts', '../pages/quiz.ts', '../pages/ai.ts', '../main.ts']) {
      assert.ok(readFileSync(new URL(f, import.meta.url), 'utf8').includes('githubLine('), f);
    }
    // AI Chat's full-height screen: under the chat list and under the memory tab, never in the conversation pane
    const ai = readFileSync(new URL('../pages/ai.ts', import.meta.url), 'utf8');
    assert.equal(ai.split('githubLine()').length - 1, 2);
    // The shell draws it in the page footer and on the shut-down screen
    const shell = readFileSync(new URL('../components/shell.ts', import.meta.url), 'utf8');
    assert.equal(shell.split('githubLine()').length - 1, 2);
  });
});
