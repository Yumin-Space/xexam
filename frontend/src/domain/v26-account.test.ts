// v2.6 account package: sign-in gate decisions, the Profile name rule, and source checks of the Settings language group and the
// menu's profile link. Run with `node --experimental-strip-types --test` (npm test).
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { describe, it } from 'node:test';
import { nameSave, setupRequired, setupView } from './user-name.ts';

const src = (rel: string): string => readFileSync(new URL(rel, import.meta.url), 'utf8');

describe('sign-in gate', () => {
  it('asks for the password first, then only the name', () => {
    assert.equal(setupView({ mustChangePassword: true, needsName: true }), 'password');
    assert.equal(setupView({ mustChangePassword: true }), 'password');
    assert.equal(setupView({ mustChangePassword: false, needsName: true }), 'name');
    assert.equal(setupView({ mustChangePassword: false, needsName: false }), null);
    assert.equal(setupView({ mustChangePassword: false }), null);
    assert.equal(setupView(null), null);
  });
  it('restricts the session while either step is open', () => {
    assert.equal(setupRequired({ mustChangePassword: false, needsName: true }), true);
    assert.equal(setupRequired({ mustChangePassword: true }), true);
    assert.equal(setupRequired({ mustChangePassword: false }), false);
    assert.equal(setupRequired(undefined), false);
  });
});

describe('profile name save', () => {
  it('refuses empty and bad names, ignores the unchanged one', () => {
    assert.equal(nameSave(null, '   '), 'empty');
    assert.equal(nameSave('Sam', 'a@b'), 'bad');
    assert.equal(nameSave('Sam', 'x'.repeat(21)), 'bad');
    assert.equal(nameSave('Sam', ' Sam  '), 'same');
    assert.equal(nameSave('Sam', 'sam'), 'ok');
    assert.equal(nameSave(null, 'Sam  Lee'), 'ok');
    assert.equal(nameSave(undefined, '小明'), 'ok');
  });
});

describe('page sources', () => {
  it('Settings has the default-question-language cards and no account section', () => {
    const s = src('../pages/settings.ts');
    assert.match(s, /set-qlang-\$\{code\}/);
    assert.match(s, /save\(\{ questionLang/);
    assert.match(s, /'settings\.qlang\.label'/);
    assert.doesNotMatch(s, /accountSection|changeOwnPassword|set-pw/);
  });
  it('the menu user row links to the Profile page', () => {
    const s = src('../components/shell.ts');
    assert.match(s, /class: 'nav-link nav-who'/);
    assert.match(s, /href: HREF\.profile/);
    assert.match(s, /navigate\(\{ name: 'profile' \}\)/);
    assert.match(s, /shownName\(user\)/);
  });
  it('the profile page is centred and main.ts routes to it', () => {
    assert.match(src('../styles/profile.css'), /\.page-profile \{[^}]*margin-inline: auto/);
    assert.match(src('../main.ts'), /case 'profile':/);
    assert.match(src('../main.ts'), /authView === 'name'/);
  });
});
