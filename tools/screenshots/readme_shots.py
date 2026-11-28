"""Make the README gallery images from the documentation screenshots.

The documentation screenshots are full pages of different heights, which look uneven side by side in a README. This script
crops each chosen one to the visible screen (desktop 16:10, phone 390 x 844) and scales it down, so every image of a row
has the same shape, and draws a thin grey frame around it. The README shows the dark theme, so the images are taken
dark into a separate folder first; it writes docs/<lang>/screenshots/readme/*.png.

    python tools/screenshots/retake.py --theme dark --out /tmp/dark --only desktop-1-home.png,desktop-3-result.png,desktop-14-ai-chat.png,iphone-1-home.png,iphone-2-quiz.png,iphone-6-result.png,iphone-7-ai-chat.png,\
        iphone-11-fx-right.png,iphone-12-fx-wrong.png
    python tools/screenshots/readme_shots.py --from /tmp/dark          # all three languages
    python tools/screenshots/readme_shots.py --from /tmp/dark en       # one language

Needs Pillow (pip install pillow).
"""
import argparse
import os

from PIL import Image, ImageDraw

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), '..', '..'))
LANGS = ('en', 'ja', 'zh-CN')
# (source screenshot, README image, kind)
SHOTS = (
    ('desktop-1-home.png', 'desktop-home.png', 'desktop'),
    ('desktop-3-result.png', 'desktop-result.png', 'desktop'),
    ('desktop-14-ai-chat.png', 'desktop-ai-chat.png', 'desktop'),
    ('iphone-1-home.png', 'phone-home.png', 'phone'),
    ('iphone-2-quiz.png', 'phone-quiz.png', 'phone'),
    ('iphone-6-result.png', 'phone-result.png', 'phone'),
    ('iphone-7-ai-chat.png', 'phone-ai-chat.png', 'phone'),
    # v3.0: the practice answer effects, taken mid-effect (confetti for a right answer, two claw swipes for a wrong one)
    ('iphone-11-fx-right.png', 'phone-fx-right.png', 'phone'),
    ('iphone-12-fx-wrong.png', 'phone-fx-wrong.png', 'phone'),
)
# the shape of the visible screen and the size written
SIZE = {'desktop': (1280, 800), 'phone': (390, 844)}


def crop_top(im, ratio):
    """The top part of a full-page image with the given height / width ratio (padded with the page colour if shorter)"""
    w, h = im.size
    want = round(w * ratio)
    if h >= want:
        return im.crop((0, 0, w, want))
    out = Image.new(im.mode, (w, want), im.getpixel((w - 1, h - 1)))
    out.paste(im, (0, 0))
    return out


def main(source, langs):
    for lang in langs:
        src = os.path.join(source, lang)
        dst = os.path.join(ROOT, 'docs', lang, 'screenshots', 'readme')
        os.makedirs(dst, exist_ok=True)
        for name, out, kind in SHOTS:
            tw, th = SIZE[kind]
            im = Image.open(os.path.join(src, name)).convert('RGB')
            im = crop_top(im, th / tw).resize((tw, th), Image.LANCZOS)
            # a thin grey frame, so the dark pages keep an edge on GitHub's dark background too
            ImageDraw.Draw(im).rectangle((0, 0, tw - 1, th - 1), outline=(88, 96, 105), width=2)
            im.save(os.path.join(dst, out), optimize=True)
        print('%s: %d images in %s' % (lang, len(SHOTS), os.path.relpath(dst, ROOT)))


if __name__ == '__main__':
    ap = argparse.ArgumentParser(description='Make the README gallery images from dark screenshots')
    ap.add_argument('--from', dest='source', required=True, help='the --out folder of retake.py --theme dark (holds <lang>/)')
    ap.add_argument('langs', nargs='*', help='en, ja and / or zh-CN (default all three)')
    a = ap.parse_args()
    bad = [x for x in a.langs if x not in LANGS]
    if bad:
        ap.error('unknown language: ' + ', '.join(bad))
    main(a.source, a.langs or LANGS)
