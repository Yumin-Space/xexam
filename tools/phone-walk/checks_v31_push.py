# -*- coding: utf-8 -*-
"""v3.1 walk checks, part 2: notifications, install, the phone guide, signed-in devices, the time zone and the service worker.
Made-up accounts and data only. The walk's browsers are headless Chrome: they never get the notification permission, so the
checks cover the states a device can reach without it (the step 2 -> 3 move of the guide and the insecure branches are proven by
the unit tests of frontend/src/domain/push-env.test.ts). The walk runs on 127.0.0.1, a secure context. The Chrome of a phone-sized
device sends an iPhone user agent under 400 px, an Android one from 400 px up, a desktop one on a large screen
(tools/phone-walk/device.py), so the three platforms of the guide are all reachable.

  W-V31-NT-1  settings (sam)   `#set-notify` exists after the answering section; the account switch (`#set-push`) is ON, >= 44 px
                               high on phones, toggles off and on (the server follows); `.set-dev-state` shows a state text; no
                               sideways scroll
  W-V31-NT-2  settings (sam)   permission "default" (headless): desktop and Android -> Enable visible and enabled, Test hidden;
                               iPhone (browser, not the Home Screen app) -> the "needs install" state, Enable visible but
                               `disabled` with `aria-describedby="set-dev-state"`, Test hidden; with the account switch off
                               the state text changes to the account-off text and Enable is disabled (not hidden), on an
                               iPhone the install state keeps the first place; the switch is restored to ON
  W-V31-IN-1  settings (sam)   `#set-install` is present on phone sizes (iPhone: the iOS text; Android: the manual text) and
                               absent on a desktop; "Show the setup guide" opens Home with `#gd-card`
  W-V31-GD-1  home (sam)       after clearing `xexam.guide.v1`: `#gd-card` shows on phone sizes only; the iPhone user agent
                               (< 400 px) shows the iOS Safari steps with three drawings; the Android user agent shows the
                               manual step with Skip; a desktop never shows it
  W-V31-GD-2  home (sam)       `.gd-close` is >= 44 x 44, does not overlap the title, closes the card, and the card stays closed
                               after a reload; the card never causes sideways scroll at 320 px
  W-V31-DV-1  settings (sam)   `#set-devices` lists the current device first with the "This device" tag and without a sign-out
                               button on it; IP lines only while the sign-in info eye is open; "Sign out all other devices" is
                               there while another device is signed in (the walk signs sam in a second time with another
                               device cookie first) and signs it out
  W-V31-DV-2  users (admin)    `#us-devices-<id>` in each row's `.us-icons`, >= 44 px on phones, opens the device dialog with no
                               notification control; signing one device of a second session out works; the per-user facts
                               lines are still there
  W-V31-TZ-1  profile (sam)    `#pf-tz`: the first option is Automatic, `UTC` is in the list; choosing `UTC` saves it (server
                               value `UTC`), choosing Automatic again saves `auto`
  W-V31-SW-1  home (sam)       desktop: `link[rel=manifest]` with `crossorigin="use-credentials"`, the service worker registers
                               with scope `/`, `/manifest.webmanifest` parses and lists the four icons

The walk's devices keep `xexam.guide.v1 = dismissed` (device.py), so the guide never covers another check; the GD / IN checks
clear it for their own page loads and put it back. Registered in checks.py (GROUPS_V31_PUSH); run alone with --only 'W-V31-*'.
"""
import json
import re
import time

import seed
from checks import LANGS, goto, phone, until
from checks_v13 import verdict
from checks_v22 import js

IPHONE = (393, 852)            # < 400 px: the iPhone user agent
SMALL_IPHONE = (320, 640)
ANDROID = (430, 932)           # >= 400 px, a phone: the Android user agent
DESKTOP = (1280, 800)
GUIDE_KEY = 'xexam.guide.v1'
SECOND_DEVICE = 'walk-second'
SHARE_RE = re.compile(r'Share|分享|共享|共有', re.I)
HOME_SCREEN = {'zh': '添加到主屏幕', 'ja': 'ホーム画面に追加', 'en': 'Add to Home Screen'}   # Apple's own button names
HOME_READY = "document.querySelector('.page-dash') && document.querySelector('.dash-board') && !document.querySelector('.dash-board-skel')"
PUSH_ON = ("(async () => { await fetch('/api/settings', {method: 'POST', headers: {'Content-Type': 'application/json'}, "
           "body: JSON.stringify({push: true})}); return 1 })()")
CONFIRM = ("(() => { const x = [...document.querySelectorAll('dialog[open]')].pop(); "
           "(x.querySelector('.btn.danger.solid') || x.querySelector('.acts .btn.primary')).click(); return 1 })()")


def cases(rc, cid, page, fn, sizes, themes=None):
    if not rc.want(cid) or not rc.walk.wants_page(page):
        return
    for lang in rc.langs(LANGS):
        for theme in (themes or rc.themes(['light'])[:1]):
            for size in rc.sizes(sizes, lambda s: True):
                rc.guard(cid, page, fn, rc, lang, theme, size)


def platform_of(size):
    """The platform the walk's device pretends to be at this size (device.py): iPhone, Android or desktop."""
    if not phone(size):
        return 'desktop'
    return 'ios' if min(size) < 400 else 'android'


def guide_clear(d):
    d.js("(() => { try { localStorage.removeItem(%s); } catch (e) {} return 1 })()" % json.dumps(GUIDE_KEY))


def guide_dismiss(d):
    d.js("(() => { try { localStorage.setItem(%s, 'dismissed'); } catch (e) {} return 1 })()" % json.dumps(GUIDE_KEY))


def api_settings(d):
    return d.js("(async () => { const r = await fetch('/api/settings'); return await r.json() })()")


def api_devices(d):
    return d.js("(async () => { const r = await fetch('/api/devices'); return await r.json() })()")


def second_session(rc, user):
    """Signs `user` in once more with another device cookie (a second browser of the same person)."""
    s = seed.Session(rc.walk.base)
    s.sign_in(user, seed.DEMO_PASSWORD, device=SECOND_DEVICE)
    return s


def reveal_user(d, name, uid):
    """Brings the user row of `name` onto the visible page (search first, then paging; phones show 5 users per page)."""
    probe = "!!document.querySelector('#us-devices-%d')" % uid
    if d.js(probe):
        return
    d.js("(() => { const i = document.querySelector('#us-search'); if (i) { i.value = %s; i.dispatchEvent(new Event('input', {bubbles: true})); } return 1 })()" % json.dumps(name))
    time.sleep(0.6)
    for _ in range(12):
        if d.js(probe):
            return
        if not d.js("(() => { const n = document.querySelector('#us-next'); if (!n || n.getAttribute('aria-disabled') === 'true') return false; n.click(); return true })()"):
            return
        time.sleep(0.5)


def open_settings(d, seconds=1.8):
    goto(d, '#/settings', seconds)
    until(d, "document.querySelector('#set-notify')", 12.0)
    time.sleep(0.4)


# ------------------------------------------------------------------------------------------------ W-V31-NT-1

NT1_JS = """
const n = document.querySelector('#set-notify');
if (!n) return {found: false};
const secs = [...document.querySelectorAll('.set-sec')];
const own = n.classList.contains('set-sec') ? n : n.closest('.set-sec');
const idx = secs.indexOf(own);
const prev = idx > 0 ? secs[idx - 1] : null;
const sw = document.querySelector('#set-push');
const st = document.querySelector('.set-dev-state');
sw && sw.scrollIntoView({block: 'center'});
const b = sw ? box(sw) : null;
return {found: true, idx, prevSwitches: prev ? prev.querySelectorAll('.set-switch').length : 0, isSwitch: !!sw && sw.classList.contains('set-switch'),
  role: sw ? sw.getAttribute('role') : null, checked: sw ? sw.getAttribute('aria-checked') : null, h: b ? b.h : null, l: b ? b.l : null, r: b ? b.r : null,
  state: st ? st.textContent.trim() : null, stateRole: st ? st.getAttribute('role') : null,
  sideways: document.documentElement.scrollWidth - innerWidth, vw: innerWidth};"""


def nt1_case(rc, lang, theme, size):
    d = rc.dev('sam', lang, theme, size)
    p = []
    try:
        open_settings(d)
        r = js(d, NT1_JS)
        if not r['found']:
            rc.rec('W-V31-NT-1', 'settings', 'notify', 'sam', size, lang, theme, False, 'no #set-notify section on Settings', d)
            return
        if r['idx'] < 4 or r['prevSwitches'] < 2:
            p.append('#set-notify is section %d and follows a section with %d switches (expected after the answering section)' % (r['idx'], r['prevSwitches']))
        if not r['isSwitch'] or r['role'] != 'switch':
            p.append('#set-push is not a .set-switch with role switch')
        if r['checked'] != 'true':
            p.append('the account switch is not ON (aria-checked %r)' % r['checked'])
        if phone(size) and r['h'] is not None and r['h'] < 43.5:
            p.append('the account switch is %d px high (< 44)' % round(r['h']))
        if r['l'] is not None and (r['l'] < -0.5 or r['r'] > r['vw'] + 0.5):
            p.append('the switch sticks out sideways (%d..%d of %d px)' % (r['l'], r['r'], r['vw']))
        if not r['state']:
            p.append('.set-dev-state shows no state text')
        if r['sideways'] > 1:
            p.append('the page is %d px wider than the viewport' % r['sideways'])
        if not p:
            d.tap_sel('#set-push', 0, 0.8)
            if api_settings(d).get('push') is not False:
                p.append('a tap did not turn the server value off')
            d.tap_sel('#set-push', 0, 0.8)
            if api_settings(d).get('push') is not True:
                p.append('a second tap did not turn it back on')
    finally:
        d.js(PUSH_ON)
    verdict(rc, 'W-V31-NT-1', 'settings', 'notify', 'sam', lang, theme, size, d, p,
            'the notifications section follows the answering section; the account switch is ON, >= 44 px on phones, toggles; the state text shows')


# ------------------------------------------------------------------------------------------------ W-V31-NT-2

NT2_JS = """
const acts = document.querySelector('.set-notify-acts');
const btns = acts ? [...acts.querySelectorAll('button')].filter(vis) : [];
const enable = btns.find((b) => !b.classList.contains('ghost')) || null;
const test = btns.find((b) => b.classList.contains('ghost')) || null;
const st = document.querySelector('.set-dev-state');
return {acts: !!acts, enable: !!enable, enableDisabled: enable ? (enable.disabled || enable.getAttribute('aria-disabled') === 'true') : null,
  describedBy: enable ? enable.getAttribute('aria-describedby') : null, test: !!test, state: st ? st.textContent.trim() : '',
  permission: typeof Notification !== 'undefined' ? Notification.permission : 'unsupported'};"""


def nt2_case(rc, lang, theme, size):
    d = rc.dev('sam', lang, theme, size)
    plat = platform_of(size)
    p = []
    try:
        open_settings(d)
        r = js(d, NT2_JS)
        if r['permission'] != 'default':
            rc.rec('W-V31-NT-2', 'settings', 'actions', 'sam', size, lang, theme, None,
                   'the headless browser reports the permission %r, not default; nothing measured' % r['permission'])
            return
        if not r['acts']:
            p.append('no .set-notify-acts row')
        if not r['enable']:
            p.append('the Enable button is not shown')
        if r['test']:
            p.append('the Test button is shown without notifications on this device')
        if plat == 'ios':
            if not r['enableDisabled']:
                p.append('on an iPhone (browser) Enable is not disabled')
            if r['describedBy'] != 'set-dev-state':
                p.append('the disabled Enable button has aria-describedby %r, expected set-dev-state' % r['describedBy'])
        elif r['enableDisabled']:
            p.append('Enable is disabled on %s (state: %s)' % (plat, r['state'][:60]))
        first = r['state']
        # the account switch off: Enable stays (disabled), the state text changes (on an iPhone the install state wins)
        d.tap_sel('#set-push', 0, 0.9)
        off = js(d, NT2_JS)
        if not off['enable']:
            p.append('with the account switch off the Enable button is hidden (it must stay, disabled)')
        elif not off['enableDisabled']:
            p.append('with the account switch off Enable is not disabled')
        if off['test']:
            p.append('with the account switch off the Test button is shown')
        if plat == 'ios':
            if off['state'] != first:
                p.append('on an iPhone the state text changed when the account switch went off (the install state comes first)')
        elif not off['state'] or off['state'] == first:
            p.append('with the account switch off the state text did not change ("%s")' % off['state'][:60])
    finally:
        d.js(PUSH_ON)
    verdict(rc, 'W-V31-NT-2', 'settings', 'actions', 'sam', lang, theme, size, d, p,
            '%s: Enable %s, Test hidden; account off -> Enable disabled, state text %s' % (
                plat, 'disabled with aria-describedby' if plat == 'ios' else 'enabled', 'unchanged (install first)' if plat == 'ios' else 'changed'))


# ------------------------------------------------------------------------------------------------ W-V31-IN-1

IN1_JS = """
const hd = document.getElementById('set-install'); const s = hd && hd.closest('section');
if (!s) return {found: false};
const txt = [...s.querySelectorAll('.set-note, p')].map((e) => e.textContent.trim()).filter(Boolean);
const ghost = [...s.querySelectorAll('button.btn.ghost')].filter(vis);
const ib = document.getElementById('set-install-btn');
return {found: true, installBtnH: ib && vis(ib) ? box(ib).h : 0, text: txt.join(' | '), guideBtn: ghost.length, sideways: document.documentElement.scrollWidth - innerWidth,
  small: ghost.map((b) => box(b).h).filter((h) => h < 43.5).length};"""


def in1_case(rc, lang, theme, size):
    d = rc.dev('sam', lang, theme, size)
    plat = platform_of(size)
    p = []
    try:
        open_settings(d)
        r = js(d, IN1_JS)
        if plat == 'desktop':
            if r['found']:
                p.append('#set-install is shown on a desktop')
        elif not r['found']:
            p.append('no #set-install on a phone (%s)' % plat)
        else:
            if plat == 'ios':
                if not SHARE_RE.search(r['text']) or HOME_SCREEN[lang] not in r['text']:
                    p.append('the iPhone text does not name Share and "%s": %s' % (HOME_SCREEN[lang], r['text'][:100]))
            elif r['installBtnH']:
                # Android with a captured install prompt: the section is the Install button only
                if r['installBtnH'] < 43.5:
                    p.append('the Install button is %d px high (< 44)' % round(r['installBtnH']))
            elif lang == 'en':
                if not re.search(r'Install app|Add to Home screen', r['text'], re.I):
                    p.append('the Android text does not name the browser menu entries: %s' % r['text'][:100])
            elif not r['text']:
                p.append('the Android text is empty')
            if r['sideways'] > 1:
                p.append('the page is %d px wider than the viewport' % r['sideways'])
            if not r['guideBtn']:
                p.append('no "Show the setup guide" button')
            else:
                if r['small']:
                    p.append('a button of the install section is under 44 px')
                guide_clear(d)
                d.js("(() => { const bs = [...document.getElementById('set-install').closest('section').querySelectorAll('button.btn.ghost')]; bs[bs.length - 1].scrollIntoView({block: 'center'}); return 1 })()")
                time.sleep(0.2)
                idx = d.js("[...document.getElementById('set-install').closest('section').querySelectorAll('button.btn.ghost')].length - 1")
                d.js("(() => { const bs = [...document.getElementById('set-install').closest('section').querySelectorAll('button.btn.ghost')]; bs[bs.length - 1].click(); return 1 })()")
                until(d, "document.querySelector('#gd-card')", 8.0)
                if d.js('location.hash') != '#/':
                    p.append('"Show the setup guide" went to %s, not Home' % d.js('location.hash'))
                if not d.js("!!document.querySelector('#gd-card')"):
                    p.append('"Show the setup guide" did not show #gd-card on Home')
    finally:
        guide_dismiss(d)
    verdict(rc, 'W-V31-IN-1', 'settings', 'install', 'sam', lang, theme, size, d, p,
            '%s: %s; the guide button opens Home with the card' % (plat, 'no install section' if plat == 'desktop' else 'install section with its text'))


# ------------------------------------------------------------------------------------------------ W-V31-GD-1

GD1_JS = """
const c = document.querySelector('#gd-card');
if (!c) return {shown: false};
const li = c.querySelectorAll('.gd-list > li').length;
const art = [...c.querySelectorAll('svg[role="img"]')].length;
const btns = [...c.querySelectorAll('button')].filter(vis).map((b) => ({ghost: b.classList.contains('ghost'), close: b.classList.contains('gd-close'), text: b.textContent.trim()}));
return {shown: vis(c), li, art, btns, steps: !!c.querySelector('.gd-steps'), text: c.textContent.replace(/\\s+/g, ' ').trim().slice(0, 300),
  sideways: document.documentElement.scrollWidth - innerWidth};"""


def gd1_case(rc, lang, theme, size):
    d = rc.dev('sam', lang, theme, size)
    plat = platform_of(size)
    p = []
    try:
        guide_clear(d)
        goto(d, '#/', 1.8)
        until(d, HOME_READY, 10.0)
        time.sleep(0.8)
        r = js(d, GD1_JS)
        if plat == 'desktop':
            if r['shown']:
                p.append('the guide card is shown on a desktop')
        elif not r['shown']:
            p.append('no #gd-card on a %s phone after clearing %s' % (plat, GUIDE_KEY))
        elif plat == 'ios':
            if r['li'] != 3:
                p.append('the iOS steps list has %d steps, expected 3' % r['li'])
            if r['art'] != 3:
                p.append('%d drawings (svg role=img) in the iOS Safari steps, expected 3' % r['art'])
            if not SHARE_RE.search(r['text']) or HOME_SCREEN[lang] not in r['text']:
                p.append('the iOS steps do not name Share and "%s": %s' % (HOME_SCREEN[lang], r['text'][:100]))
        else:
            if r['li'] or r['art']:
                p.append('the Android manual step shows iOS drawings (%d steps, %d drawings)' % (r['li'], r['art']))
            if not any(b['ghost'] and not b['close'] for b in r['btns']):
                p.append('the Android manual step has no Skip button')
        if r.get('shown') and r['sideways'] > 1:
            p.append('the page is %d px wider than the viewport' % r['sideways'])
    finally:
        guide_dismiss(d)
    verdict(rc, 'W-V31-GD-1', 'home', 'guide', 'sam', lang, theme, size, d, p,
            '%s: %s' % (plat, 'no guide card' if plat == 'desktop' else ('iOS Safari steps with three drawings' if plat == 'ios' else 'manual install step with Skip')))


# ------------------------------------------------------------------------------------------------ W-V31-GD-2

GD2_JS = """
const c = document.querySelector('#gd-card'); if (!c) return {shown: false};
const x = c.querySelector('.gd-close'); const t = c.querySelector('#gd-title') || c.querySelector('h2, h3');
const hb = x ? window.__walk.hitBox(x) : null;
const ib = x ? box(x) : null, tb = t ? box(t) : null;
const overlap = ib && tb ? (Math.min(ib.r, tb.r) - Math.max(ib.l, tb.l) > 0.5 && Math.min(ib.b, tb.b) - Math.max(ib.t, tb.t) > 0.5) : null;
return {shown: true, close: !!x, w: hb ? hb.r - hb.l : null, h: hb ? hb.b - hb.t : null, overlap, title: !!t,
  sideways: document.documentElement.scrollWidth - innerWidth};"""


def gd2_case(rc, lang, theme, size):
    if not phone(size):
        return      # the setup guide is phone-only by contract (GD-1): nothing to close on a desktop or tablet
    d = rc.dev('sam', lang, theme, size)
    p = []
    try:
        guide_clear(d)
        goto(d, '#/', 1.8)
        until(d, HOME_READY, 10.0)
        time.sleep(0.8)
        r = js(d, GD2_JS)
        if not r['shown']:
            rc.rec('W-V31-GD-2', 'home', 'guide-close', 'sam', size, lang, theme, False, 'no #gd-card to test the close button on', d)
            return
        if not r['close']:
            p.append('no .gd-close button')
        else:
            if r['w'] < 43.5 or r['h'] < 43.5:
                p.append('.gd-close is %dx%d px (< 44 x 44)' % (round(r['w']), round(r['h'])))
            if r['overlap']:
                p.append('.gd-close overlaps the card title')
        if r['sideways'] > 1:
            p.append('the page is %d px wider than the viewport' % r['sideways'])
        if r['close']:
            d.tap_sel('#gd-card .gd-close', 0, 0.8)
            if d.js("!!document.querySelector('#gd-card')"):
                p.append('the card is still there after .gd-close')
            else:
                d.reload(1.6)
                until(d, HOME_READY, 10.0)
                time.sleep(0.6)
                if d.js("!!document.querySelector('#gd-card')"):
                    p.append('the card came back after a reload')
    finally:
        guide_dismiss(d)
    verdict(rc, 'W-V31-GD-2', 'home', 'guide-close', 'sam', lang, theme, size, d, p,
            'the close button is >= 44 x 44, clear of the title, closes the card for good; no sideways scroll')


# ------------------------------------------------------------------------------------------------ W-V31-DV-1

DV1_JS = """
const hd = document.getElementById('set-devices'); const s = hd && hd.closest('section');
if (!s) return {found: false};
s.scrollIntoView({block: 'start'});
const rows = [...s.querySelectorAll('.dv-row')];
const first = rows[0];
const out = s.querySelector('button.btn.ghost.danger');
const ipRe = /\\b\\d{1,3}(?:\\.\\d{1,3}){3}\\b/;
return {found: true, rows: rows.length, firstCurrent: !!first && !!first.querySelector('.dv-tags .tag.k'),
  currentCount: rows.filter((r) => r.querySelector('.dv-tags .tag.k')).length,
  firstOut: !!first && !!first.querySelector('.dv-out'), ips: rows.filter((r) => { const c = r.cloneNode(true); c.querySelectorAll('.dv-ua, [hidden]').forEach((e) => e.remove()); return ipRe.test(c.textContent); }).length,
  othersBtn: !!out && vis(out), sideways: document.documentElement.scrollWidth - innerWidth};"""


def dv1_case(rc, lang, theme, size):
    d = rc.dev('sam', lang, theme, size)
    p = []
    second_session(rc, 'sam')
    open_settings(d, 2.0)
    until(d, "(() => { const h = document.getElementById('set-devices'); const s = h && h.closest('section'); return !!(s && s.querySelector('.dv-row')) })()", 10.0)
    time.sleep(0.4)
    info = api_devices(d)
    r = js(d, DV1_JS)
    if not r['found']:
        rc.rec('W-V31-DV-1', 'settings', 'devices', 'sam', size, lang, theme, False, 'no #set-devices section on Settings', d)
        return
    if r['rows'] < 1:
        p.append('no .dv-row in the device list')
    if not r['firstCurrent'] or r['currentCount'] != 1:
        p.append('the "This device" tag is on %d rows and %s the first row' % (r['currentCount'], 'on' if r['firstCurrent'] else 'not on'))
    if r['firstOut']:
        p.append('the current device has a sign-out button')
    if not info.get('signinInfo') and r['ips']:
        p.append('%d rows show an IP address while the sign-in info eye is closed' % r['ips'])
    if r['sideways'] > 1:
        p.append('the page is %d px wider than the viewport' % r['sideways'])
    others = [x for x in info.get('devices', []) if not x.get('current') and x.get('state') in ('active', 'expired')]
    if not others:
        p.append('the second sign-in left no other active device in GET /api/devices')
    elif not r['othersBtn']:
        p.append('"Sign out all other devices" is not shown while another device is signed in')
    else:
        d.tap_sel('section[aria-labelledby="set-devices"] button.btn.ghost.danger', 0, 0.8)
        if not until(d, "document.querySelector('dialog[open]')", 5.0):
            p.append('no confirm dialog for "Sign out all other devices"')
        else:
            d.js(CONFIRM)
            time.sleep(1.2)
            after = api_devices(d)
            still = [x for x in after.get('devices', []) if not x.get('current') and x.get('state') in ('active', 'expired')]
            if still:
                p.append('%d other device(s) are still signed in after the action' % len(still))
            if not any(x.get('current') and x.get('state') == 'active' for x in after.get('devices', [])):
                p.append('this device is no longer signed in')
            if d.js("(() => { const h = document.getElementById('set-devices'); const s = h && h.closest('section'); return !!(s && s.querySelector('button.btn.ghost.danger')) })()"):
                p.append('the sign-out-others button is still shown with no other device left')
    verdict(rc, 'W-V31-DV-1', 'settings', 'devices', 'sam', lang, theme, size, d, p,
            'this device first with its tag and no sign-out button; "Sign out all other devices" signs the second device out')


# ------------------------------------------------------------------------------------------------ W-V31-DV-2

DV2_JS = """
const b = document.querySelector('#us-devices-%d');
if (!b) return {found: false};
b.scrollIntoView({block: 'center'});
const x = window.__walk.hitBox(b);
const row = b.closest('.us-row');
return {found: true, inIcons: !!b.closest('.us-icons'), w: x.r - x.l, h: x.b - x.t, facts: !!(row && row.querySelector('.us-facts, .us-meta'))};"""

DV2_DIALOG_JS = """
const dlg = document.querySelector('dialog[open]'); if (!dlg) return {open: false};
const names = [...dlg.querySelectorAll('button')].map((b) => b.textContent + ' ' + (b.getAttribute('aria-label') || '')).join(' ');
return {open: true, rows: dlg.querySelectorAll('.dv-row').length, outs: dlg.querySelectorAll('.dv-out').length,
  controls: dlg.querySelectorAll('[role=switch], input[type=checkbox]').length, notify: /notif|通知/i.test(names),
  l: box(dlg).l, r: box(dlg).r, vw: innerWidth};"""


def dv2_case(rc, lang, theme, size):
    d = rc.dev('admin', lang, theme, size)
    admin = rc.session('admin')
    riley = admin.user_id('riley')
    second_session(rc, 'riley')
    p = []
    goto(d, '#/users?tab=users', 2.0)
    until(d, "document.querySelector('.us-row')", 15.0)
    reveal_user(d, 'riley', riley)
    r = js(d, DV2_JS % riley)
    if not r['found']:
        rc.rec('W-V31-DV-2', 'users', 'devices', 'admin', size, lang, theme, False, 'no #us-devices-%d icon on the user row' % riley, d)
        return
    if not r['inIcons']:
        p.append('the devices icon is not inside .us-icons')
    if phone(size) and min(r['w'], r['h']) < 43.5:
        p.append('the devices icon is %dx%d px (< 44)' % (round(r['w']), round(r['h'])))
    if not r['facts']:
        p.append('the per-user facts lines of the row are gone')
    d.tap_sel('#us-devices-%d' % riley, 0, 0.9)
    until(d, "document.querySelector('dialog[open] .dv-row')", 8.0)
    dg = js(d, DV2_DIALOG_JS)
    if not dg['open']:
        p.append('the devices icon did not open a dialog')
    else:
        if dg['rows'] < 2:
            p.append('the dialog lists %d devices, expected the two sessions of riley' % dg['rows'])
        if dg['controls'] or dg['notify']:
            p.append('the dialog has a notification control (administrators never control notifications)')
        if dg['l'] < -0.5 or dg['r'] > dg['vw'] + 0.5:
            p.append('the dialog is %d..%d px wide in a %d px window' % (dg['l'], dg['r'], dg['vw']))
        # sign one device out: the one that is not riley's own walk browser device
        mine = [x['id'] for x in rc.session('riley').call('/api/devices')['devices'] if x.get('current')]
        listing = admin.call('/api/users/devices?user=%d' % riley)['devices']
        target = next((i for i, x in enumerate(listing) if x['id'] not in mine and x['state'] == 'active'), None)
        if target is None:
            p.append('no second active device of riley to sign out')
        else:
            tid = listing[target]['id']
            ok = d.js("(() => { const o = document.querySelector('dialog[open] .dv-row[data-device=\"%d\"] .dv-out'); "
                      "if (!o) return false; o.scrollIntoView({block: 'center'}); o.click(); return true })()" % tid)
            if not ok:
                p.append('the second device row has no sign-out button')
            else:
                until(d, "document.querySelectorAll('dialog[open]').length > 1", 4.0, 0.2)
                d.js(CONFIRM)
                time.sleep(1.2)
                after = admin.call('/api/users/devices?user=%d' % riley)['devices']
                st = next((x['state'] for x in after if x['id'] == tid), None)
                if st != 'signedOut':
                    p.append('the second device is "%s" after the sign-out, expected signedOut' % st)
    d.js("document.querySelectorAll('dialog[open]').forEach((x) => x.close()); 1")
    verdict(rc, 'W-V31-DV-2', 'users', 'devices', 'admin', lang, theme, size, d, p,
            'the devices icon opens the device dialog without notification controls; one device of a second session signs out; facts lines intact')


# ------------------------------------------------------------------------------------------------ W-V31-TZ-1

TZ_JS = """
const s = document.querySelector('#pf-tz'); if (!s) return {found: false};
s.scrollIntoView({block: 'center'});
const o = [...s.options]; const r = s.getBoundingClientRect();
return {found: true, first: o[0] ? o[0].value : null, hasUtc: o.some((x) => x.value === 'UTC'), value: s.value, n: o.length, h: r.height, vw: innerWidth,
  l: r.left, r2: r.right, sideways: document.documentElement.scrollWidth - innerWidth};"""

TZ_SET_JS = """(async (v) => { const s = document.querySelector('#pf-tz'); s.value = v; s.dispatchEvent(new Event('change', {bubbles: true}));
  await new Promise((r) => setTimeout(r, 900)); const j = await (await fetch('/api/settings')).json(); return j.tz })"""


def tz1_case(rc, lang, theme, size):
    d = rc.dev('sam', lang, theme, size)
    p = []
    try:
        goto(d, '#/profile', 1.8)
        until(d, "document.querySelector('#pf-tz')", 12.0)
        time.sleep(0.3)
        r = d.js('(() => { %s })()' % TZ_JS)
        if not r['found']:
            rc.rec('W-V31-TZ-1', 'profile', 'time-zone', 'sam', size, lang, theme, False, 'no #pf-tz select on Profile', d)
            return
        if r['first'] != 'auto':
            p.append('the first option is %r, expected auto (Automatic)' % r['first'])
        if not r['hasUtc']:
            p.append('UTC is not in the time zone list (%d options)' % r['n'])
        if phone(size) and r['h'] < 43.5:
            p.append('the select is %d px high (< 44)' % round(r['h']))
        if r['l'] < -0.5 or r['r2'] > r['vw'] + 0.5:
            p.append('the select sticks out sideways')
        if r['sideways'] > 1:
            p.append('the page is %d px wider than the viewport' % r['sideways'])
        if r['hasUtc']:
            got = d.js('(%s)(%s)' % (TZ_SET_JS, json.dumps('UTC')))
            if got != 'UTC':
                p.append('choosing UTC saved %r' % got)
            got = d.js('(%s)(%s)' % (TZ_SET_JS, json.dumps('auto')))
            if got != 'auto':
                p.append('choosing Automatic saved %r' % got)
    finally:
        d.js("(async () => { await fetch('/api/settings', {method: 'POST', headers: {'Content-Type': 'application/json'}, "
             "body: JSON.stringify({tz: 'auto'})}); return 1 })()")
    verdict(rc, 'W-V31-TZ-1', 'profile', 'time-zone', 'sam', lang, theme, size, d, p,
            '#pf-tz: Automatic first, UTC in the list, UTC and auto are saved')


# ------------------------------------------------------------------------------------------------ W-V31-SW-1

SW_JS = """(async () => {
  const link = document.querySelector('link[rel=manifest]');
  let reg = null;
  for (let i = 0; i < 40 && !reg; i++) { reg = await navigator.serviceWorker.getRegistration(); if (!reg) await new Promise((r) => setTimeout(r, 250)); }
  let man = null, err = null;
  try { const r = await fetch('/manifest.webmanifest', {credentials: 'include'}); man = {status: r.status, json: await r.json()}; } catch (e) { err = String(e); }
  return {link: link ? {href: link.getAttribute('href'), cross: link.getAttribute('crossorigin')} : null,
    scope: reg ? reg.scope : null, origin: location.origin,
    man: man ? {status: man.status, icons: (man.json.icons || []).map((i) => i.src + ' ' + i.sizes + ' ' + i.purpose)} : null, err};
})()"""


def sw1_case(rc, lang, theme, size):
    d = rc.dev('sam', lang, theme, size)
    goto(d, '#/', 1.8)
    until(d, HOME_READY, 10.0)
    r = d.js(SW_JS)
    p = []
    if not r['link']:
        p.append('no <link rel="manifest">')
    else:
        if r['link']['href'] != '/manifest.webmanifest':
            p.append('the manifest link points to %r' % r['link']['href'])
        if r['link']['cross'] != 'use-credentials':
            p.append('the manifest link has crossorigin %r, expected use-credentials' % r['link']['cross'])
    if r['scope'] != r['origin'] + '/':
        p.append('the service worker scope is %r, expected %s/' % (r['scope'], r['origin']))
    if r['err'] or not r['man'] or r['man']['status'] != 200:
        p.append('/manifest.webmanifest could not be read as JSON: %s' % (r['err'] or (r['man'] and r['man']['status'])))
    else:
        icons = r['man']['icons']
        want = ['/icon-192.png 192x192 any', '/icon-512.png 512x512 any', '/icon-maskable-192.png 192x192 maskable',
                '/icon-maskable-512.png 512x512 maskable']
        if sorted(icons) != sorted(want):
            p.append('the manifest icons are %s, expected %s' % (icons, want))
    verdict(rc, 'W-V31-SW-1', 'home', 'service-worker', 'sam', lang, theme, size, d, p,
            'manifest link with credentials, service worker scope /, manifest with the four icons')


def g_v31_push(rc):
    cases(rc, 'W-V31-NT-1', 'settings', nt1_case, [IPHONE, SMALL_IPHONE, DESKTOP])
    cases(rc, 'W-V31-NT-2', 'settings', nt2_case, [IPHONE, ANDROID, DESKTOP])
    cases(rc, 'W-V31-IN-1', 'settings', in1_case, [IPHONE, ANDROID, DESKTOP])
    cases(rc, 'W-V31-GD-1', 'home', gd1_case, [IPHONE, ANDROID, DESKTOP])
    cases(rc, 'W-V31-GD-2', 'home', gd2_case, [IPHONE, SMALL_IPHONE], rc.themes(['light', 'dark']))
    cases(rc, 'W-V31-DV-1', 'settings', dv1_case, [IPHONE, DESKTOP])
    cases(rc, 'W-V31-DV-2', 'users', dv2_case, [IPHONE, DESKTOP])
    cases(rc, 'W-V31-TZ-1', 'profile', tz1_case, [IPHONE, DESKTOP])
    cases(rc, 'W-V31-SW-1', 'home', sw1_case, [DESKTOP])


GROUPS_V31_PUSH = [g_v31_push]
