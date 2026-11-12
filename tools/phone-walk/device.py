# -*- coding: utf-8 -*-
"""A phone-like Chrome for the phone walk: one headless browser over CDP (tools/screenshots/browser.py), signed in as one
made-up user, with phone emulation and the input a thumb would make.

Device(chrome_exe, base, cookie, lang='zh', theme='light', width=390, height=844, ...)
  .set_size(w, h)          portrait or landscape (landscape when w > h), mobile emulation: device scale 2-3, touch points,
                           a mobile user agent, safe-area insets (47 / 34 portrait; left / right 47 and bottom 21 in
                           landscape) when this Chrome supports them (`safe_area_supported` is False otherwise: the checks
                           that need the insets are then reported as skipped)
  .set_theme(t)            prefers-color-scheme light / dark;  .set_reduced_motion(on)
  .keyboard(True / False)  simulates the on-screen keyboard by shrinking the viewport height to 55 % (no emulator shows the
                           real iOS keyboard, so this is the closest that exists; it never replaces the owner's iPhone check)
  .nav('#/route')          opens a route and waits;  .js(code) / .js_call(fn, *args)  evaluates in the page
  .rect(sel, idx)          box of the idx-th visible element (scrolled into view unless scroll=False)
  .tap(x, y) / .tap_sel(sel, idx)   a real touch tap (Input.dispatchTouchEvent);  .swipe(x, y, dy)  a touch swipe
  .type(text) / .key(name)  text and keys;  .scroll(y)
  .shot(path)              PNG without metadata (browser.strip_png); full=True for the whole page
  .net_clear() / .net_requests(pattern)   the requests the page made since (Network.requestWillBeSent), for example to
                           prove that no POST /api/ai/study/ask went out
  .close()
Used as a context manager, it closes the browser at the end.

Reference: the audit drivers (scratchpad/v10/audit-study/work/drv.py and audit-ai-shell/_scripts/*.py) use the same calls.
"""
import base64
import json
import os
import sys
import time
import urllib.request

HERE = os.path.dirname(os.path.abspath(__file__))
SHOTS = os.path.join(os.path.dirname(HERE), 'screenshots')
if SHOTS not in sys.path:
    sys.path.insert(0, SHOTS)

from browser import CDP, Chrome, find_chrome, strip_png  # noqa: E402
import scenes  # noqa: E402
import seed  # noqa: E402

BROWSER_LANG = {'zh': 'zh-CN', 'en': 'en-US', 'ja': 'ja'}
ANDROID_UA = ('Mozilla/5.0 (Linux; Android 14; Pixel 8) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0.0.0 '
              'Mobile Safari/537.36')
IPHONE_UA = scenes.IPHONE_UA
DESKTOP_UA = scenes.DESKTOP_UA
PRELUDE = scenes.PRELUDE
KEYBOARD_SHARE = 0.55          # the simulated on-screen keyboard leaves 55 % of the viewport height (spec 5.4)
PORTRAIT_INSETS = {'top': 47, 'bottom': 34, 'left': 0, 'right': 0}
LANDSCAPE_INSETS = {'top': 0, 'bottom': 21, 'left': 47, 'right': 47}


class EventCDP(CDP):
    """CDP that keeps the events it passes while waiting for an answer (browser.CDP drops them)."""

    def __init__(self, url, timeout=120):
        super().__init__(url, timeout)
        self.events = []

    def call(self, method, **params):
        self.next_id += 1
        mid = self.next_id
        self._send(0x1, json.dumps({'id': mid, 'method': method, 'params': params}).encode())
        while True:
            msg = json.loads(self._message())
            if msg.get('id') == mid:
                if 'error' in msg:
                    raise RuntimeError('%s: %s' % (method, msg['error']))
                return msg.get('result', {})
            if 'method' in msg:
                self.events.append(msg)

    def drain(self):
        """Reads what the browser has sent so far: any answered command comes after the events that preceded it."""
        self.call('Runtime.evaluate', expression='0')


def is_mobile_size(w, h):
    return min(w, h) < 700


class Device:
    def __init__(self, chrome_exe, base, cookie, lang='zh', theme='light', width=390, height=844, scale=None,
                 reduced_motion=False, settings=None):
        """`cookie`: the value of the xexam_session cookie of the made-up user (None = signed out); `settings`: extra
        fields for POST /api/settings (the UI and question language follow `lang`)."""
        self.base = base.rstrip('/')
        self.lang, self.theme = lang, theme
        self.chrome = Chrome(chrome_exe or find_chrome(), BROWSER_LANG[lang])
        self.scale = scale
        self.kb_open = False
        self.safe_area_supported = True
        self.w, self.h = width, height
        try:
            # a second connection to the same tab that keeps the events (the Chrome class's own one drops them)
            port = int(self.chrome.browser_url.split(':')[2].split('/')[0])
            tabs = [t for t in json.load(urllib.request.urlopen('http://127.0.0.1:%d/json/list' % port, timeout=5))
                    if t.get('type') == 'page']
            self.cdp = EventCDP(tabs[0]['webSocketDebuggerUrl'])
            for d in ('Page', 'Runtime', 'DOM', 'Network'):
                self.cdp.call(d + '.enable')
            self.cdp.call('Page.addScriptToEvaluateOnNewDocument', source=PRELUDE)
            self.cdp.call('Emulation.setTimezoneOverride', timezoneId='UTC')
            self.set_theme(theme)
            self.set_reduced_motion(reduced_motion)
            self.set_size(width, height)
            self.sign_in(cookie, settings)
        except BaseException:
            self.close()
            raise

    # ------------------------------------------------------------ device
    def set_size(self, w, h):
        self.w, self.h = w, h
        self.kb_open = False
        self._metrics(h)
        self._insets()

    def _metrics(self, height):
        mobile = is_mobile_size(self.w, self.h)
        landscape = self.w > self.h
        scale = self.scale or (3 if mobile else 2)
        if mobile:
            self.cdp.call('Emulation.setTouchEmulationEnabled', enabled=True, maxTouchPoints=5)
            self.cdp.call('Emulation.setUserAgentOverride', userAgent=IPHONE_UA if min(self.w, self.h) < 400 else ANDROID_UA)
        else:
            self.cdp.call('Emulation.setTouchEmulationEnabled', enabled=False)
            self.cdp.call('Emulation.setUserAgentOverride', userAgent=DESKTOP_UA)
        orient = {'type': 'landscapePrimary', 'angle': 90} if landscape else {'type': 'portraitPrimary', 'angle': 0}
        self.cdp.call('Emulation.setDeviceMetricsOverride', width=self.w, height=height, deviceScaleFactor=scale,
                      mobile=mobile, screenWidth=self.w, screenHeight=self.h, screenOrientation=orient)
        time.sleep(0.3)

    def _insets(self):
        """The notch / home-indicator insets of a phone (47 / 34), none on a larger screen."""
        mobile = is_mobile_size(self.w, self.h)
        insets = (LANDSCAPE_INSETS if self.w > self.h else PORTRAIT_INSETS) if mobile else dict(top=0, bottom=0, left=0, right=0)
        try:
            self.cdp.call('Emulation.setSafeAreaInsetsOverride', insets=insets)
            self.safe_area_supported = True
        except RuntimeError:
            self.safe_area_supported = False

    def set_theme(self, theme):
        self.theme = theme
        self._media('prefers-color-scheme', 'dark' if theme == 'dark' else 'light')

    def set_reduced_motion(self, on):
        self._media('prefers-reduced-motion', 'reduce' if on else 'no-preference')

    def _media(self, name, value):
        self.media = getattr(self, 'media', {})
        self.media[name] = value
        self.cdp.call('Emulation.setEmulatedMedia', features=[{'name': k, 'value': v} for k, v in self.media.items()])

    def keyboard(self, on):
        """Simulated on-screen keyboard: the viewport shrinks to 55 % of its height (and grows back)."""
        self.kb_open = bool(on)
        self._metrics(round(self.h * KEYBOARD_SHARE) if on else self.h)
        time.sleep(0.2)

    def viewport(self):
        return self.js('({w: innerWidth, h: innerHeight, scrollY, vv: visualViewport ? {h: visualViewport.height, top: '
                       'visualViewport.offsetTop} : null})')

    # ------------------------------------------------------------ session and navigation
    def sign_in(self, cookie, settings=None):
        self.cdp.call('Network.clearBrowserCookies')
        if cookie:
            self.cdp.call('Network.setCookie', name=seed.COOKIE, value=cookie, url=self.base + '/', httpOnly=True,
                          sameSite='Strict')
            self.nav('#/')
            self.save_settings(**(settings or {}))
        self.nav('#/')

    def save_settings(self, **kw):
        body = {'uiLang': self.lang, 'questionLang': self.lang, 'theme': self.theme, 'autoAdvance': False}
        body.update(kw)
        return self.js("(async () => { const r = await fetch('/api/settings', {method: 'POST', headers: "
                       "{'Content-Type': 'application/json'}, body: %s}); localStorage.removeItem('xexam.settings.v1'); "
                       "return r.status })()" % json.dumps(json.dumps(body)))

    def nav(self, route='#/', settle=1.4):
        self.cdp.call('Page.navigate', url='about:blank')
        time.sleep(0.15)
        self.cdp.call('Page.navigate', url=self.base + '/' + route)
        time.sleep(settle)

    def reload(self, settle=1.4):
        self.cdp.call('Page.reload')
        time.sleep(settle)

    # ------------------------------------------------------------ page access
    def js(self, code):
        return self.cdp.evaluate(code)

    def js_call(self, fn, *args):
        """Evaluates `(fn)(...args)` in the page; `fn` is JavaScript source such as "(a, b) => a + b"."""
        return self.js('(%s)(%s)' % (fn, ', '.join(json.dumps(a) for a in args)))

    def rect(self, sel, idx=0, scroll=True):
        """Box of the idx-th visible element matching `sel` (None when there is none)."""
        return self.js("""(() => { const els = [...document.querySelectorAll(%s)].filter(e => e.offsetParent || e.getClientRects().length);
          const e = els[%d]; if (!e) return null; %s
          const r = e.getBoundingClientRect(); return {x: r.left, y: r.top, w: r.width, h: r.height,
          cx: r.left + r.width / 2, cy: r.top + r.height / 2, text: (e.innerText || e.value || '').slice(0, 60)} })()"""
                       % (json.dumps(sel), idx, "{ const r0 = e.getBoundingClientRect(); "
                          # also when it is on screen but under pinned chrome (the practice bar, the top row): a thumb
                          # would scroll it into the free area first
                          "const h0 = document.elementFromPoint(r0.left + r0.width / 2, r0.top + r0.height / 2); "
                          "if (r0.top < 0 || r0.bottom > innerHeight || !h0 || !(h0 === e || e.contains(h0))) "
                          "e.scrollIntoView({block: 'center'}); }" if scroll else ''))

    # ------------------------------------------------------------ input
    def tap(self, x, y, wait=0.35):
        pt = [{'x': x, 'y': y, 'radiusX': 6, 'radiusY': 6, 'force': 1, 'id': 1}]
        self.cdp.call('Input.dispatchTouchEvent', type='touchStart', touchPoints=pt)
        time.sleep(0.05)
        self.cdp.call('Input.dispatchTouchEvent', type='touchEnd', touchPoints=[])
        time.sleep(wait)

    def click(self, x, y, wait=0.35):
        for t in ('mousePressed', 'mouseReleased'):
            self.cdp.call('Input.dispatchMouseEvent', type=t, x=x, y=y, button='left', clickCount=1)
        time.sleep(wait)

    def tap_sel(self, sel, idx=0, wait=0.4):
        r = self.rect(sel, idx)
        if not r:
            raise RuntimeError('no element ' + sel)
        time.sleep(0.15)
        r = self.rect(sel, idx, scroll=False)
        (self.tap if is_mobile_size(self.w, self.h) else self.click)(r['cx'], r['cy'], wait)
        return r

    def swipe(self, x, y, dy, steps=8, wait=0.5):
        """A touch swipe from (x, y) by dy pixels (negative = finger moves up = the page scrolls down)."""
        def pt(yy):
            return [{'x': x, 'y': yy, 'radiusX': 6, 'radiusY': 6, 'force': 1, 'id': 1}]
        self.cdp.call('Input.dispatchTouchEvent', type='touchStart', touchPoints=pt(y))
        for i in range(1, steps + 1):
            self.cdp.call('Input.dispatchTouchEvent', type='touchMove', touchPoints=pt(y + dy * i / steps))
            time.sleep(0.016)
        self.cdp.call('Input.dispatchTouchEvent', type='touchEnd', touchPoints=[])
        time.sleep(wait)

    def type(self, text):
        self.cdp.call('Input.insertText', text=text)

    def key(self, key, code=None, vk=0):
        for t in ('keyDown', 'keyUp'):
            self.cdp.call('Input.dispatchKeyEvent', type=t, key=key, code=code or key, windowsVirtualKeyCode=vk)

    def scroll(self, y):
        self.js('window.scrollTo(0, %d); window.scrollY' % y)
        time.sleep(0.25)

    # ------------------------------------------------------------ network log
    def net_clear(self):
        self.cdp.drain()
        self.cdp.events.clear()

    def net_requests(self, pattern=''):
        """[(method, url)] of the requests since net_clear() whose URL contains `pattern`."""
        self.cdp.drain()
        out = []
        for ev in self.cdp.events:
            if ev.get('method') == 'Network.requestWillBeSent':
                r = ev['params']['request']
                if pattern in r['url']:
                    out.append((r['method'], r['url']))
        return out

    # ------------------------------------------------------------ output
    def shot(self, path, full=False):
        params = {'format': 'png'}
        if full:
            height = self.js('Math.ceil(document.documentElement.scrollHeight)')
            params['captureBeyondViewport'] = True
            params['clip'] = {'x': 0, 'y': 0, 'width': self.w, 'height': min(height, 6000), 'scale': 1}
        data = strip_png(base64.b64decode(self.cdp.call('Page.captureScreenshot', **params)['data']))
        os.makedirs(os.path.dirname(os.path.abspath(path)), exist_ok=True)
        with open(path, 'wb') as fh:
            fh.write(data)
        return path

    def close(self):
        cdp = getattr(self, 'cdp', None)
        if cdp:
            cdp.close()
            self.cdp = None
        self.chrome.close()

    def __enter__(self):
        return self

    def __exit__(self, *exc):
        self.close()
