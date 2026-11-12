// In-page measurement code of the phone walk (checks.py evaluates this file after every page load).
// Everything is exposed as window.__walk.<name>; each function returns plain JSON. It reads the DOM only (it never changes
// the app's state), except that audit() scrolls elements into view and puts the scroll position back. Elements are found
// through the test hooks of spec 2.12 f or by their role, never by an internal class that a page may rename.
(() => {
  if (window.__walk) return;
  const W = {};

  // ------------------------------------------------------------------ small helpers
  const visible = (e) => {
    if (!e || !(e instanceof Element)) return false;
    const cs = getComputedStyle(e);
    if (cs.visibility === 'hidden' || cs.display === 'none') return false;
    const r = e.getBoundingClientRect();
    if (r.width <= 0 || r.height <= 0) return false;
    for (let p = e; p; p = p.parentElement) {
      const s = getComputedStyle(p);
      if (s.display === 'none' || s.visibility === 'hidden') return false;
      if (p.hidden) return false;
    }
    return true;
  };
  const sel = (e) => {
    if (!e) return '';
    let s = e.tagName.toLowerCase();
    if (e.id) s += '#' + e.id;
    const c = (typeof e.className === 'string' ? e.className : '').trim().split(/\s+/).filter(Boolean).slice(0, 3);
    if (c.length) s += '.' + c.join('.');
    const l = e.getAttribute('data-focus') || e.getAttribute('data-slot') || e.getAttribute('data-act');
    if (l) s += '[' + l + ']';
    return s;
  };
  const box = (e) => {
    const r = e.getBoundingClientRect();
    return { x: r.left, y: r.top, w: r.width, h: r.height, r: r.right, b: r.bottom };
  };
  const label = (e) => ((e.getAttribute('aria-label') || e.innerText || e.value || e.title || '') + '').replace(/\s+/g, ' ').trim().slice(0, 40);
  const px = (v) => parseFloat(v) || 0;
  W.topH = () => px(getComputedStyle(document.documentElement).getPropertyValue('--top-h'));

  // ------------------------------------------------------------------ W-SH-1 sideways scroll
  // The document and every big block must fit the width. Elements that scroll on purpose (overflow-x auto / scroll) are fine.
  // The first descendant box of `e` that sticks out of its content width (null when none). Pseudo-elements (the invisible
  // ::after hit areas of buttons at an edge) do not count, nor what a nested scroller / clipper or a fixed box holds.
  const sticksOut = (e) => {
    const r = e.getBoundingClientRect();
    const left = r.left + e.clientLeft, right = left + e.clientWidth;
    for (const k of e.querySelectorAll('*')) {
      const kr = k.getBoundingClientRect();
      if (kr.width === 0 || (kr.right <= right + 1 && kr.left >= left - 1)) continue;
      let inner = false;
      for (let p = k; p && p !== e; p = p.parentElement) {
        const cs = getComputedStyle(p);
        if (cs.position === 'fixed' || (p !== k && cs.overflowX !== 'visible')) { inner = true; break; }
      }
      if (inner || !visible(k) || k.closest('.sr-only')) continue;
      return k;
    }
    return null;
  };
  W.overflow = () => {
    const de = document.documentElement;
    const out = { doc: Math.max(de.scrollWidth, document.body.scrollWidth) - de.clientWidth, vw: de.clientWidth, offenders: [] };
    const blocks = document.querySelectorAll('.main, .wrap, .panel, .card, .paper, section, dialog[open], .ai-panes, .rv, table');
    for (const e of blocks) {
      if (!visible(e)) continue;
      const cs = getComputedStyle(e);
      if (/(auto|scroll)/.test(cs.overflowX)) continue;
      if (e.scrollWidth > e.clientWidth + 1 && e.clientWidth > 0) {
        const k = sticksOut(e);
        if (k) out.offenders.push(sel(e) + ' ' + e.scrollWidth + '>' + e.clientWidth + ' (' + sel(k) + ')');
      }
    }
    // A vertical scroller (a message list, a panel body, a menu) must not scroll sideways as well; tables in a scroller
    // scroll sideways on purpose
    for (const e of document.querySelectorAll('body *')) {
      if (out.offenders.length > 8) break;
      const cs = getComputedStyle(e);
      if (!/(auto|scroll)/.test(cs.overflowX) || !/(auto|scroll)/.test(cs.overflowY) || e.tagName === 'PRE') continue;
      if (e.scrollWidth <= e.clientWidth + 1 || e.clientWidth === 0 || !visible(e)) continue;
      const k = sticksOut(e);
      if (k && !k.closest('table')) out.offenders.push('scrolls sideways: ' + sel(e) + ' ' + e.scrollWidth + '>' + e.clientWidth + ' (' + sel(k) + ')');
    }
    // anything that sticks out of the right edge of the viewport (and is not inside a scroller)
    for (const e of document.querySelectorAll('body *')) {
      if (out.offenders.length > 8) break;
      const r = e.getBoundingClientRect();
      if (r.width === 0 || r.right <= out.vw + 1) continue;
      let scroller = false;
      for (let p = e.parentElement; p; p = p.parentElement) {
        if (/(auto|scroll|hidden|clip)/.test(getComputedStyle(p).overflowX) && p !== document.body && p !== de) { scroller = true; break; }
      }
      const pos = getComputedStyle(e).position;
      if (!scroller && pos !== 'fixed' && visible(e) && !e.closest('.sr-only, #toast')) out.offenders.push('sticks out: ' + sel(e) + ' right=' + Math.round(r.right));
    }
    out.offenders = out.offenders.slice(0, 6);
    return out;
  };

  // ------------------------------------------------------------------ W-SH-3 / W-SH-5 hit areas and covering
  const INTERACTIVE = 'a[href], button, input:not([type=hidden]), select, textarea, summary, [role=button], [role=tab], [role=switch], [role=checkbox], [role=radio], [role=menuitem]';
  const inRunningText = (e) => {
    if (e.tagName !== 'A') return false;
    const p = e.closest('p, li, dd, td, span.dim, .sub');
    return !!p && p !== e && (p.textContent || '').trim().length > (e.textContent || '').trim().length + 8;
  };
  const targetOf = (e) => {
    // checkboxes / radios / file inputs are usually hidden inside a label: the label is what a thumb hits
    if (e.tagName === 'INPUT' && /^(checkbox|radio|file)$/.test(e.type)) {
      const lab = e.closest('label');
      if (lab && visible(lab)) return lab;
    }
    // any other form field inside its own <label> (no other control in it): a tap anywhere on the label focuses the field,
    // so the label is the hit area (Home's count field: a 34 px field in a 44 px label)
    if (/^(INPUT|SELECT|TEXTAREA)$/.test(e.tagName)) {
      const lab = e.closest('label');
      if (lab && visible(lab) && lab.querySelectorAll(INTERACTIVE).length === 1) return lab;
    }
    return e;
  };
  // Size of the area around the centre in which a tap lands on `el`: scans elementFromPoint in four directions. This is
  // what a thumb gets, so it covers the box, an invisible ::after and anything that overlaps a neighbour.
  const hitSize = (el, r) => {
    const cx = r.left + r.width / 2, cy = r.top + r.height / 2;
    const inside = (x, y) => { const h = document.elementFromPoint(x, y); return !!h && (el === h || el.contains(h)); };
    if (!inside(cx, cy)) return null;
    const reach = (dx, dy) => { let n = 0; while (n < 40 && inside(cx + dx * (n + 1), cy + dy * (n + 1))) n++; return n; };
    return { w: reach(-1, 0) + reach(1, 0) + 1, h: reach(0, -1) + reach(0, 1) + 1 };
  };
  const pinnedBottomTop = () => {
    let top = innerHeight;
    for (const e of document.querySelectorAll('.pfoot, .home-go, .ai-composer')) {
      const p = getComputedStyle(e).position;
      if ((p === 'fixed' || p === 'sticky') && visible(e)) top = Math.min(top, e.getBoundingClientRect().top);
    }
    return top;
  };
  W.audit = (opts) => {
    opts = opts || {};
    const touch = !!opts.touch;
    const y0 = scrollY;
    const res = { count: 0, small: [], covered: [], skipped: 0 };
    const seen = new Set();
    const els = [...document.querySelectorAll(INTERACTIVE)].filter(visible).filter((e) => !opts.within || e.closest(opts.within));
    for (const raw of els) {
      const e = targetOf(raw);
      if (seen.has(e)) continue;
      seen.add(e);
      if (e.closest('#toast, .sr-only')) continue;
      let r = e.getBoundingClientRect();
      if (r.width < 2 && r.height < 2) continue;
      if (e.tagName === 'SUMMARY' || inRunningText(e) || e.closest('[data-walk-skip]')) { res.skipped++; continue; }
      const pos = getComputedStyle(e).position;
      const pinned = e.closest('.topbar, .pfoot, .home-go, .ai-composer, dialog') || pos === 'fixed' || pos === 'sticky';
      // not in the free area (above the pinned bottom bar / composer, below the top row): scroll it there first
      if (!pinned && (r.top < W.topH() + 4 || r.bottom > Math.min(innerHeight, pinnedBottomTop()) - 4)) {
        e.scrollIntoView({ block: 'center', inline: 'nearest' });
        r = e.getBoundingClientRect();
      }
      // Inside a scroller of its own (a menu, a list) and scrolled (partly) out of it: bring it into that scroller's view first
      // (a row cut by the scroller's edge is what a thumb scrolls into view before tapping it)
      for (let p = e.parentElement; p && p !== document.body; p = p.parentElement) {
        if (!/(auto|scroll)/.test(getComputedStyle(p).overflowY)) continue;
        const pr = p.getBoundingClientRect();
        if ((r.top < pr.top - 0.5 || r.bottom > pr.bottom + 0.5) && r.height < pr.height) {
          // centred, so the scroller's edge does not cut the control's tap margin
          e.scrollIntoView({ block: 'center', inline: 'nearest' });
          r = e.getBoundingClientRect();
        }
        break;
      }
      res.count++;
      const top = Math.max(W.topH(), 0), bottom = pinnedBottomTop();
      const free = pinned || (r.top >= top - 1 && r.bottom <= bottom + 1);
      if (r.bottom <= 0 || r.top >= innerHeight) { res.skipped++; continue; }
      const hs = hitSize(e, r);
      const cx = r.left + r.width / 2, cy = r.top + r.height / 2;
      const hit = document.elementFromPoint(cx, cy);
      const ok = !!hit && (e === hit || e.contains(hit) || hit.contains(e) ||
        (hit.closest('label') && hit.closest('label') === e.closest('label')) ||
        (hit.closest(INTERACTIVE) === e));
      if (free && !ok && hit && cy >= 0 && cy <= innerHeight) {
        res.covered.push(sel(e) + ' "' + label(e) + '" covered by ' + sel(hit));
      }
      if (touch && hs && (hs.w < 43.5 || hs.h < 43.5)) {
        res.small.push(sel(e) + ' "' + label(e) + '" hit ' + Math.round(hs.w) + 'x' + Math.round(hs.h) + ' box ' + Math.round(r.width) + 'x' + Math.round(r.height));
      }
    }
    window.scrollTo(0, y0);
    res.small = res.small.slice(0, 12);
    res.covered = res.covered.slice(0, 12);
    return res;
  };

  // ------------------------------------------------------------------ W-SH-4 pinned bars and insets
  // Declared hit boxes: the element box united with its ::after box (spec SH-2 hit areas), for W-US-1.
  W.hitBox = (e) => {
    const r = e.getBoundingClientRect();
    let b = { l: r.left, t: r.top, r: r.right, b: r.bottom };
    const cs = getComputedStyle(e, '::after');
    if (cs.content !== 'none' && cs.position === 'absolute') {
      const own = getComputedStyle(e);
      const w = px(cs.width), h = px(cs.height);
      const l = r.left + px(own.borderLeftWidth) + px(cs.left), t = r.top + px(own.borderTopWidth) + px(cs.top);
      if (w > 0 && h > 0) b = { l: Math.min(b.l, l), t: Math.min(b.t, t), r: Math.max(b.r, l + w), b: Math.max(b.b, t + h) };
    }
    return b;
  };
  // Is the bar fully inside the viewport and are its controls clear of the insets? (insets: {top,bottom,left,right})
  W.pinned = (barSel, insets) => {
    const bar = [...document.querySelectorAll(barSel)].find(visible);
    if (!bar) return { found: false };
    const r = bar.getBoundingClientRect();
    const out = { found: true, box: box(bar), vw: innerWidth, vh: innerHeight, problems: [] };
    if (r.left < -0.5 || r.right > innerWidth + 0.5 || r.bottom > innerHeight + 0.5 || r.top < -0.5) out.problems.push('bar box leaves the viewport');
    for (const c of bar.querySelectorAll(INTERACTIVE)) {
      if (!visible(c)) continue;
      const b = c.getBoundingClientRect();
      if (b.width < 2) continue;
      if (b.bottom > innerHeight - insets.bottom + 0.5) out.problems.push(sel(c) + ' bottom ' + Math.round(b.bottom) + ' is inside the bottom inset (' + insets.bottom + ')');
      if (b.left < insets.left - 0.5) out.problems.push(sel(c) + ' left ' + Math.round(b.left) + ' is inside the left inset (' + insets.left + ')');
      if (b.right > innerWidth - insets.right + 0.5) out.problems.push(sel(c) + ' right ' + Math.round(b.right) + ' is inside the right inset (' + insets.right + ')');
    }
    out.problems = out.problems.slice(0, 6);
    return out;
  };

  // ------------------------------------------------------------------ rows and positions
  // Do the visible controls of `rootSel` share one row? (their vertical centres differ by less than `tol` px)
  W.oneRow = (rootSel, itemSel, tol) => {
    const root = [...document.querySelectorAll(rootSel)].find(visible);
    if (!root) return { found: false };
    const items = [...root.querySelectorAll(itemSel || 'button, a')].filter(visible);
    const ys = items.map((e) => { const r = e.getBoundingClientRect(); return r.top + r.height / 2; });
    const spread = ys.length ? Math.max(...ys) - Math.min(...ys) : 0;
    return { found: true, n: items.length, spread: Math.round(spread * 10) / 10, one: spread <= (tol || 4), box: box(root),
      items: items.map((e) => sel(e) + ' ' + Math.round(e.getBoundingClientRect().left) + '+' + Math.round(e.getBoundingClientRect().width)) };
  };
  // x / width of the bar's slots by data-slot, and their top inside the bar (identical for every state; the bar itself
  // may sit higher up on a short page, so its own position is not compared)
  W.slots = () => {
    const bar = [...document.querySelectorAll('.pfoot')].find(visible);
    const out = {};
    if (!bar) return null;
    const barTop = bar.getBoundingClientRect().top;
    for (const e of bar.querySelectorAll('[data-slot]')) {
      if (!visible(e)) continue;
      const r = e.getBoundingClientRect();
      out[e.getAttribute('data-slot')] = { l: Math.round(r.left * 10) / 10, w: Math.round(r.width * 10) / 10, t: Math.round(r.top - barTop) };
    }
    out._bar = box(bar);
    return out;
  };
  W.rectOf = (s, i) => {
    const e = [...document.querySelectorAll(s)].filter(visible)[i || 0];
    return e ? box(e) : null;
  };
  W.hitAtCentre = (s) => {
    const e = [...document.querySelectorAll(s)].filter(visible)[0];
    if (!e) return null;
    const r = e.getBoundingClientRect();
    const hit = document.elementFromPoint(r.left + r.width / 2, r.top + r.height / 2);
    return { ok: !!hit && (e === hit || e.contains(hit)), hit: sel(hit), box: box(e) };
  };

  // ------------------------------------------------------------------ W-SH-6 contrast
  const parseColor = (s) => {
    if (!s || s === 'transparent') return [0, 0, 0, 0];
    let m = s.match(/^rgba?\(([^)]+)\)$/);
    if (m) {
      const p = m[1].split(/[ ,/]+/).filter(Boolean).map(parseFloat);
      return [p[0], p[1], p[2], p.length > 3 ? p[3] : 1];
    }
    m = s.match(/^color\(srgb ([^)]+)\)$/);
    if (m) {
      const p = m[1].split(/[ /]+/).filter(Boolean).map(parseFloat);
      return [p[0] * 255, p[1] * 255, p[2] * 255, p.length > 3 ? p[3] : 1];
    }
    return null;
  };
  const over = (top, bot) => {
    const a = top[3] + bot[3] * (1 - top[3]);
    if (a === 0) return [0, 0, 0, 0];
    return [0, 1, 2].map((i) => (top[i] * top[3] + bot[i] * bot[3] * (1 - top[3])) / a).concat([a]);
  };
  const lum = (c) => {
    const f = (v) => { v /= 255; return v <= 0.03928 ? v / 12.92 : Math.pow((v + 0.055) / 1.055, 2.4); };
    return 0.2126 * f(c[0]) + 0.7152 * f(c[1]) + 0.0722 * f(c[2]);
  };
  const ratio = (a, b) => { const x = lum(a), y = lum(b); return (Math.max(x, y) + 0.05) / (Math.min(x, y) + 0.05); };
  W.contrast = (e) => {
    const cs = getComputedStyle(e);
    let fg = parseColor(cs.color);
    if (!fg) return null;
    let op = 1;
    const chain = [];
    for (let p = e; p; p = p.parentElement) { chain.push(p); op *= px(getComputedStyle(p).opacity) || (getComputedStyle(p).opacity === '0' ? 0 : 1); }
    let bg = [0, 0, 0, 0];
    for (const p of chain) {
      const c = parseColor(getComputedStyle(p).backgroundColor);
      if (c) bg = over(bg, c);
      if (bg[3] >= 0.999) break;
    }
    bg = over(bg, parseColor(getComputedStyle(document.body).backgroundColor) || [255, 255, 255, 1]);
    bg = over(bg, [255, 255, 255, 1]);
    fg = over([fg[0], fg[1], fg[2], fg[3] * op], bg);
    const size = px(cs.fontSize), bold = px(cs.fontWeight) >= 700;
    const large = size >= 24 || (size >= 18.66 && bold);
    return { ratio: Math.round(ratio(fg, bg) * 100) / 100, need: large ? 3 : 4.5 };
  };
  // Samples: [{name, selector}] -> the worst ratio per sample (every visible match), only elements with text
  W.contrastOf = (samples) => samples.map((s) => {
    const els = [...document.querySelectorAll(s.selector)].filter(visible).filter((e) => (e.innerText || '').trim() || e.value).slice(0, 12);
    if (!els.length) return { name: s.name, found: false };
    let worst = null;
    for (const e of els) {
      const c = W.contrast(e);
      if (c && (!worst || c.ratio - c.need < worst.ratio - worst.need)) worst = Object.assign({ el: sel(e) + ' "' + label(e) + '"' }, c);
    }
    return Object.assign({ name: s.name, found: true }, worst || { ratio: 21, need: 4.5, el: '' });
  });

  // ------------------------------------------------------------------ W-MN-1 menu
  W.menuReach = () => {
    const pop = [...document.querySelectorAll('.nav-pop')].find(visible);
    if (!pop) return { found: false };
    const out = { found: true, total: 0, unreachable: [], popBox: box(pop), vh: innerHeight, scrollable: pop.scrollHeight > pop.clientHeight + 1 };
    for (const e of pop.querySelectorAll('a[href], button')) {
      if (!visible(e) && getComputedStyle(e).display === 'none') continue;
      out.total++;
      e.scrollIntoView({ block: 'nearest' });
      const r = e.getBoundingClientRect();
      if (r.top < -0.5 || r.bottom > innerHeight + 0.5 || r.height < 2) out.unreachable.push(sel(e) + ' "' + label(e) + '" ' + Math.round(r.top) + '..' + Math.round(r.bottom));
    }
    return out;
  };

  // ------------------------------------------------------------------ dialogs and tables
  W.dialogActs = () => {
    const d = [...document.querySelectorAll('dialog[open]')].pop();
    if (!d) return { found: false };
    const acts = d.querySelector('.acts, .dlg-acts, footer');
    if (!acts) return { found: true, acts: false, box: box(d) };
    const r = acts.getBoundingClientRect();
    return { found: true, acts: true, inside: r.top >= -0.5 && r.bottom <= innerHeight + 0.5 && r.left >= -0.5 && r.right <= innerWidth + 0.5,
      box: box(acts), vh: innerHeight, vw: innerWidth };
  };
  W.tables = () => [...document.querySelectorAll('table')].filter(visible).map((t) => {
    const host = t.closest('.panel, .card, section, .main') || document.body;
    const hb = host.getBoundingClientRect(), tb = t.getBoundingClientRect();
    return { el: sel(t), w: Math.round(tb.width), host: Math.round(hb.width), right: Math.round(tb.right), hostRight: Math.round(hb.right),
      wide: tb.right > hb.right + 1 || tb.width > hb.width + 1, scrolls: !!t.closest('.rec-scroll, .tbl-scroll') && t.scrollWidth > (t.parentElement || t).clientWidth + 1 };
  });

  // ------------------------------------------------------------------ AI Chat (W-CH-*)
  W.chat = () => {
    const q = (s) => [...document.querySelectorAll(s)].find(visible);
    const msgs = [...document.querySelectorAll('#ai-msgs > li, .ai-msgs > li, .ai-msg')].filter(visible);
    const last = msgs[msgs.length - 1];
    const sc = q('.ai-scroll');
    const comp = q('.ai-composer');
    const ta = q('.ai-composer textarea, #ai-input');
    return {
      vh: innerHeight, vw: innerWidth, scrollY: scrollY, docH: document.documentElement.scrollHeight,
      topbar: q('.topbar') ? box(q('.topbar')) : null, head: q('.ai-conv-head') ? box(q('.ai-conv-head')) : null,
      composer: comp ? box(comp) : null, last: last ? box(last) : null, scroller: sc ? box(sc) : null,
      atEnd: sc ? sc.scrollHeight - sc.scrollTop - sc.clientHeight <= 24 : null,
      font: ta ? px(getComputedStyle(ta).fontSize) : null, nmsg: msgs.length,
      vv: window.visualViewport ? { h: visualViewport.height, top: visualViewport.offsetTop } : null,
      kb: getComputedStyle(document.documentElement).getPropertyValue('--kb'), vvh: getComputedStyle(document.documentElement).getPropertyValue('--vvh'),
    };
  };
  W.toEnd = () => {
    const sc = [...document.querySelectorAll('.ai-scroll')].find(visible);
    if (sc) sc.scrollTop = sc.scrollHeight;
    return !!sc;
  };

  window.__walk = W;
})();
