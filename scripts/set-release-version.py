#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""Record a published release: set RELEASE_VERSION in VERSION and the release badge at the top of the three READMEs.

usage:  python3 scripts/set-release-version.py <version, e.g. 0.6.5>

The release job of .github/workflows/ci.yml runs this only after the GitHub Release of that version was created, then
commits the result. CODE_VERSION is never touched here: it is the version of the code and is bumped by hand.

What changes:
- VERSION: the line RELEASE_VERSION=<old> becomes RELEASE_VERSION=<version> (comments and CODE_VERSION stay as they are).
- README.md, README/README.ja.md, README/README.zh-CN.md: the badge
  <img width="..." height="28" alt="... v<old>" src="https://img.shields.io/badge/release-v<old>-green?style=for-the-badge">
  gets the new version in its alt text and URL, and the width of the new badge (GitHub only rounds images that have
  both width and height, so the width must match the image or the badge is stretched).

Order: RELEASE_VERSION never moves back. When the version given is older than the RELEASE_VERSION already recorded (a
re-run of an older run's release job after a newer release), nothing is changed and the exit status is 0. A version newer
than CODE_VERSION is refused (status 1): the check job would reject such a VERSION file.
Prints what it changed; exits with status 1 if VERSION or a README badge is not where it is expected.
"""
import re
import sys
import urllib.request

READMES = ['README.md', 'README/README.ja.md', 'README/README.zh-CN.md']
BADGE = re.compile(
    r'<img width="(?P<w>\d+)" height="28" alt="(?P<alt>[^"]*?) v(?P<old_alt>[0-9.]+)" '
    r'src="https://img\.shields\.io/badge/release-v(?P<old>[0-9.]+)-green\?style=for-the-badge">')


def version_key(v):
    """Sort key of a version like 0.6 or 0.6.5 (0.6 sorts before 0.6.5, 0.6.10 after 0.6.9)."""
    return tuple(int(part) for part in v.split('.'))


def single_value(text, key):
    """The value of the one KEY= line of VERSION; exits when the key is missing, repeated or not a version."""
    # Blanks around the value are allowed (also a \r of a CRLF file), anything else on the line is not
    found = re.findall(r'(?m)^' + key + r'=[^\S\n]*(\S*)[^\S\n]*$', text)
    if len(found) != 1:
        sys.exit('VERSION: expected exactly one %s= line, found %d' % (key, len(found)))
    if not re.fullmatch(r'[0-9]+\.[0-9]+(\.[0-9]+)?', found[0]):
        sys.exit('VERSION: %s is malformed: %r' % (key, found[0]))
    return found[0]


def badge_width(version, fallback):
    """Width in px of the shields.io badge for this version, rounded; the old width if shields.io cannot be reached."""
    url = 'https://img.shields.io/badge/release-v%s-green?style=for-the-badge' % version
    try:
        with urllib.request.urlopen(urllib.request.Request(url, headers={'User-Agent': 'xexam-ci'}), timeout=15) as r:
            svg = r.read(4096).decode('utf-8', 'replace')
        m = re.search(r'<svg[^>]*\swidth="([0-9.]+)"', svg)
        if m:
            return str(round(float(m.group(1))))
    except OSError as e:
        print('shields.io not reachable (%s); keeping width %s' % (e, fallback))
    return fallback


def main():
    if len(sys.argv) != 2 or not re.fullmatch(r'[0-9]+\.[0-9]+(\.[0-9]+)?', sys.argv[1].lstrip('v')):
        sys.exit('usage: set-release-version.py <version, e.g. 0.6.5>')
    version = sys.argv[1].lstrip('v')

    # 1. VERSION: replace only the value of RELEASE_VERSION, and never move it back
    text = open('VERSION', encoding='utf-8').read()
    code = single_value(text, 'CODE_VERSION')
    recorded = single_value(text, 'RELEASE_VERSION')
    if version_key(version) < version_key(recorded):
        print('VERSION already records the newer release v%s; v%s is older, so nothing is changed' % (recorded, version))
        return
    if version_key(version) > version_key(code):
        sys.exit('v%s is newer than CODE_VERSION %s of this branch; refusing to record it' % (version, code))
    new_text = re.sub(r'(?m)^RELEASE_VERSION=.*$', 'RELEASE_VERSION=' + version, text)
    if new_text != text:
        open('VERSION', 'w', encoding='utf-8', newline='\n').write(new_text)
        print('VERSION: RELEASE_VERSION=%s' % version)

    # 2. The release badge of every README (same width in all three, measured once)
    width = None
    for path in READMES:
        text = open(path, encoding='utf-8').read()
        m = BADGE.search(text)
        if not m:
            sys.exit('%s: the release badge at the top was not found' % path)
        if width is None:
            width = badge_width(version, m.group('w'))
        badge = ('<img width="%s" height="28" alt="%s v%s" '
                 'src="https://img.shields.io/badge/release-v%s-green?style=for-the-badge">') % (width, m.group('alt'), version, version)
        new_text = text[:m.start()] + badge + text[m.end():]
        if new_text != text:
            open(path, 'w', encoding='utf-8', newline='\n').write(new_text)
            print('%s: release badge v%s (width %s)' % (path, version, width))


if __name__ == '__main__':
    main()
