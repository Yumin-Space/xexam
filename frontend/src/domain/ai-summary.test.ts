import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { describe, it } from 'node:test';

describe('AI summary overlay footer', () => {
  const src = readFileSync(new URL('../components/ai-summary.ts', import.meta.url), 'utf8');
  it('has no Open in AI Chat control and never navigates', () => {
    assert.equal(/openChat|navigate\(|location\.hash/.test(src), false);
  });
  it('shows the saved text with the tip that names the chat', () => {
    assert.ok(src.includes("t('aiplus.saved')"));
    assert.ok(src.includes("t('aiplus.savedTip'"));
  });
  it('names the chat as AI Chat lists it: the server title plus the summary suffix', () => {
    assert.ok(src.includes("chatNameOf({ title, kind: 'summary' }"));
  });
});
