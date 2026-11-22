"""Make the README gallery images from the documentation screenshots.

The documentation screenshots are full pages of different heights, which look uneven side by side in a README. This script
crops each chosen one to the visible screen (desktop 16:10, phone 390 x 844) and scales it down, so every image of a row
has the same shape, and draws a thin grey frame around it. Run it after retake.py; it writes docs/<lang>/screenshots/readme/*.png.

    python tools/screenshots/readme_shots.py          # all three languages
    python tools/screenshots/readme_shots.py en       # one language

Needs Pillow (pip install pillow).
"""
import os
import sys

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


def main(langs):
    for lang in langs:
        src = os.path.join(ROOT, 'docs', lang, 'screenshots')
        dst = os.path.join(src, 'readme')
        os.makedirs(dst, exist_ok=True)
        for name, out, kind in SHOTS:
            tw, th = SIZE[kind]
            im = Image.open(os.path.join(src, name)).convert('RGB')
            im = crop_top(im, th / tw).resize((tw, th), Image.LANCZOS)
            # a thin grey frame, so the light pages stand out from the white README background
            ImageDraw.Draw(im).rectangle((0, 0, tw - 1, th - 1), outline=(208, 215, 222), width=2)
            im.save(os.path.join(dst, out), optimize=True)
        print('%s: %d images in %s' % (lang, len(SHOTS), os.path.relpath(dst, ROOT)))


if __name__ == '__main__':
    main(sys.argv[1:] or LANGS)
