#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""Writes Xexam's icon files into frontend/public/ from one source SVG.

    python3 -I tools/icons/make_icons.py <path to the source svg>

Writes (fixed paths, no metadata in any of them):
  frontend/public/icon.svg               the source SVG normalised: no XML declaration, no comments, width / height kept,
                                         LF line ends, a final newline
  frontend/public/icon-32.png            RGBA, transparent around the rounded corners (browser tab)
  frontend/public/icon-192.png           RGBA (Android home screen)
  frontend/public/icon-512.png           RGBA
  frontend/public/apple-touch-icon.png   180 x 180 opaque RGB, full-bleed: drawn from a variant whose background has no
                                         rounded corners (iOS rounds the corners itself; transparent corners would turn black)

The PNGs are drawn by headless Chrome (the DevTools client of tools/screenshots/browser.py) at device scale 1 with a
transparent page background, then re-encoded here in pure Python (zlib) with only the IHDR, IDAT and IEND chunks, so the
output depends only on the pixels. The script prints the SHA-256 of every file and is idempotent. Needs Chrome or
Chromium (XEXAM_CHROME / CHROME_PATH, or the usual install locations).
"""
import base64
import hashlib
import re
import struct
import sys
import time
import xml.etree.ElementTree as ET
import zlib
from pathlib import Path

# python3 -I does not put the script's folders on sys.path: reuse the screenshot tool's browser helpers explicitly
# (without leaving a __pycache__ folder in tools/screenshots)
sys.dont_write_bytecode = True
sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'screenshots'))

from browser import Chrome, find_chrome, strip_png  # noqa: E402

REPO = Path(__file__).resolve().parents[2]
PUBLIC = REPO / 'frontend' / 'public'
SIZES = {'icon-32.png': 32, 'icon-192.png': 192, 'icon-512.png': 512}
APPLE = ('apple-touch-icon.png', 180)
PNG_SIGNATURE = b'\x89PNG\r\n\x1a\n'


# ---------------------------------------------------------------- SVG

def normalise_svg(text):
    """The source SVG without XML declaration, doctype and comments, with LF line ends and one final newline. Refuses
    anything a strict CSP or a plain image should not carry (scripts, event handlers, style elements, external links)."""
    text = text.replace('\r\n', '\n').replace('\r', '\n').lstrip('\ufeff')
    text = re.sub(r'<\?xml[^>]*\?>', '', text)
    text = re.sub(r'<!DOCTYPE[^>]*>', '', text, flags=re.I)
    text = re.sub(r'<!--.*?-->', '', text, flags=re.S)
    lines = [line.rstrip() for line in text.split('\n')]
    text = '\n'.join(line for line in lines if line.strip()) + '\n'
    lowered = text.lower()
    for bad in ('<script', '<style', 'style=', 'javascript:', '<foreignobject', 'xlink:href="http', 'href="http'):
        if bad in lowered:
            raise SystemExit('the source SVG contains %r; use presentation attributes only' % bad)
    if re.search(r'\son[a-z]+\s*=', lowered):
        raise SystemExit('the source SVG contains an event handler attribute')
    root = ET.fromstring(text)
    if root.tag != '{http://www.w3.org/2000/svg}svg':
        raise SystemExit('the source is not an SVG document (root element %s)' % root.tag)
    if not root.get('viewBox'):
        raise SystemExit('the source SVG has no viewBox')
    if not (root.get('width') and root.get('height')):
        raise SystemExit('the source SVG must keep its width and height attributes')
    return text


def sized(svg, size):
    """The SVG drawn at `size` x `size` px (width / height of the root element replaced; the viewBox scales it)."""
    head, rest = svg.split('>', 1)
    head = re.sub(r'\swidth="[^"]*"', ' width="%d"' % size, head, count=1)
    head = re.sub(r'\sheight="[^"]*"', ' height="%d"' % size, head, count=1)
    return head + '>' + rest


def full_bleed(svg):
    """The variant for the apple-touch-icon: the first <rect> (the background) without rounded corners."""
    m = re.search(r'<rect\b[^>]*>', svg)
    if not m:
        raise SystemExit('the source SVG has no background <rect>')
    rect = re.sub(r'\s(rx|ry)="[^"]*"', '', m.group(0))
    rect = rect.replace('<rect', '<rect rx="0"', 1)
    return svg[:m.start()] + rect + svg[m.end():]


# ---------------------------------------------------------------- PNG

def decode_png(data):
    """(width, height, rows of RGBA bytes) of an 8-bit RGB / RGBA, non-interlaced PNG."""
    if data[:8] != PNG_SIGNATURE:
        raise ValueError('not a PNG')
    pos, idat, ihdr = 8, [], None
    while pos < len(data):
        n = struct.unpack('>I', data[pos:pos + 4])[0]
        kind, body = data[pos + 4:pos + 8], data[pos + 8:pos + 8 + n]
        if kind == b'IHDR':
            ihdr = body
        elif kind == b'IDAT':
            idat.append(body)
        elif kind == b'IEND':
            break
        pos += 12 + n
    w, h, depth, ctype, _, _, interlace = struct.unpack('>IIBBBBB', ihdr)
    if depth != 8 or interlace or ctype not in (2, 6):
        raise ValueError('unexpected PNG format (depth %d, colour type %d, interlace %d)' % (depth, ctype, interlace))
    bpp = 4 if ctype == 6 else 3
    raw = zlib.decompress(b''.join(idat))
    stride = w * bpp
    rows, prev, i = [], bytearray(stride), 0
    for _ in range(h):
        f, line = raw[i], bytearray(raw[i + 1:i + 1 + stride])
        i += 1 + stride
        if f == 1:
            for x in range(bpp, stride):
                line[x] = (line[x] + line[x - bpp]) & 0xFF
        elif f == 2:
            for x in range(stride):
                line[x] = (line[x] + prev[x]) & 0xFF
        elif f == 3:
            for x in range(stride):
                left = line[x - bpp] if x >= bpp else 0
                line[x] = (line[x] + ((left + prev[x]) >> 1)) & 0xFF
        elif f == 4:
            for x in range(stride):
                a = line[x - bpp] if x >= bpp else 0
                b = prev[x]
                c = prev[x - bpp] if x >= bpp else 0
                p = a + b - c
                pa, pb, pc = abs(p - a), abs(p - b), abs(p - c)
                line[x] = (line[x] + (a if pa <= pb and pa <= pc else b if pb <= pc else c)) & 0xFF
        elif f != 0:
            raise ValueError('unknown PNG filter %d' % f)
        rows.append(bytes(line))
        prev = line
    if bpp == 3:
        rows = [b''.join(r[x:x + 3] + b'\xff' for x in range(0, stride, 3)) for r in rows]
    return w, h, rows


def encode_png(w, h, rows, ctype):
    """A PNG with only IHDR, IDAT and IEND: colour type 6 (RGBA) or 2 (RGB, alpha dropped). Each row gets the filter
    with the smallest sum of absolute values (the usual heuristic), compressed with zlib level 9."""
    bpp = 4 if ctype == 6 else 3
    if ctype == 2:
        rows = [b''.join(r[x:x + 3] for x in range(0, len(r), 4)) for r in rows]
    stride = w * bpp
    out, prev = bytearray(), bytes(stride)
    for line in rows:
        best = None
        for f in range(5):
            cand = bytearray(stride)
            for x in range(stride):
                a = line[x - bpp] if x >= bpp else 0
                b = prev[x]
                c = prev[x - bpp] if x >= bpp else 0
                if f == 0:
                    v = line[x]
                elif f == 1:
                    v = line[x] - a
                elif f == 2:
                    v = line[x] - b
                elif f == 3:
                    v = line[x] - ((a + b) >> 1)
                else:
                    p = a + b - c
                    pa, pb, pc = abs(p - a), abs(p - b), abs(p - c)
                    v = line[x] - (a if pa <= pb and pa <= pc else b if pb <= pc else c)
                cand[x] = v & 0xFF
            score = sum(v if v < 128 else 256 - v for v in cand)
            if best is None or score < best[0]:
                best = (score, f, cand)
        out.append(best[1])
        out += best[2]
        prev = line

    def chunk(kind, body):
        return struct.pack('>I', len(body)) + kind + body + struct.pack('>I', zlib.crc32(kind + body) & 0xFFFFFFFF)

    ihdr = struct.pack('>IIBBBBB', w, h, 8, ctype, 0, 0, 0)
    return PNG_SIGNATURE + chunk(b'IHDR', ihdr) + chunk(b'IDAT', zlib.compress(bytes(out), 9)) + chunk(b'IEND', b'')


# ---------------------------------------------------------------- drawing

def render(page, svg, size, transparent):
    """Draws `svg` alone on a page of exactly `size` x `size` CSS px at device scale 1; returns the PNG bytes."""
    page.call('Emulation.setDeviceMetricsOverride', width=size, height=size, deviceScaleFactor=1, mobile=False,
              screenWidth=size, screenHeight=size)
    if transparent:
        page.call('Emulation.setDefaultBackgroundColorOverride', color={'r': 0, 'g': 0, 'b': 0, 'a': 0})
    else:
        page.call('Emulation.setDefaultBackgroundColorOverride', color={'r': 255, 'g': 255, 'b': 255, 'a': 1})
    html = ('<!doctype html><html><head><meta charset="utf-8"><style>html,body{margin:0;padding:0;background:transparent;'
            'overflow:hidden}svg{display:block}</style></head><body>%s</body></html>' % sized(svg, size))
    page.call('Page.navigate', url='data:text/html;base64,' + base64.b64encode(html.encode('utf-8')).decode('ascii'))
    for _ in range(100):
        if page.evaluate('document.readyState') == 'complete' and page.evaluate('!!document.querySelector("svg")'):
            break
        time.sleep(0.05)
    time.sleep(0.3)
    shot = page.call('Page.captureScreenshot', format='png', captureBeyondViewport=False,
                     clip={'x': 0, 'y': 0, 'width': size, 'height': size, 'scale': 1})
    return strip_png(base64.b64decode(shot['data']))


def main():
    if len(sys.argv) != 2:
        raise SystemExit('usage: python3 -I tools/icons/make_icons.py <path to the source svg>')
    source = Path(sys.argv[1])
    svg = normalise_svg(source.read_text(encoding='utf-8'))
    chrome = find_chrome()
    if not chrome:
        raise SystemExit('no Chrome / Chromium found: set XEXAM_CHROME or CHROME_PATH')
    outputs = {'icon.svg': svg.encode('utf-8')}
    with Chrome(chrome, 'en-US') as browser:
        page = browser.page
        page.call('Page.enable')
        page.call('Runtime.enable')
        for name, size in SIZES.items():
            w, h, rows = decode_png(render(page, svg, size, True))
            if (w, h) != (size, size):
                raise SystemExit('%s: Chrome drew %d x %d instead of %d x %d' % (name, w, h, size, size))
            if rows[0][3] != 0:
                raise SystemExit('%s: the corner is not transparent (is the page background opaque?)' % name)
            outputs[name] = encode_png(w, h, rows, 6)
        name, size = APPLE
        w, h, rows = decode_png(render(page, full_bleed(svg), size, False))
        if (w, h) != (size, size):
            raise SystemExit('%s: Chrome drew %d x %d instead of %d x %d' % (name, w, h, size, size))
        if any(r[x] != 255 for r in rows for x in range(3, len(r), 4)):
            raise SystemExit('%s: not every pixel is opaque' % name)
        outputs[name] = encode_png(w, h, rows, 2)
    PUBLIC.mkdir(parents=True, exist_ok=True)
    for name, data in outputs.items():
        path = PUBLIC / name
        old = path.read_bytes() if path.exists() else None
        if old != data:
            path.write_bytes(data)
        print('%-22s %7d bytes  sha256 %s%s' % (name, len(data), hashlib.sha256(data).hexdigest(),
                                                 '' if old != data else '  (unchanged)'))


if __name__ == '__main__':
    main()
