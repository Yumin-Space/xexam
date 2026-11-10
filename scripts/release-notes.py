#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""Build the GitHub Release notes for one version: take that version's section out of the changelog of the English README and
append how to pull the two images.

usage:  python3 scripts/release-notes.py <version, e.g. 0.4> <image prefix, e.g. ghcr.io/yumin-space> [README path]  > notes.md
The release job of the pipeline runs this; you can also run it locally to preview what the Releases page will show.
Exits with status 1 if the version has no changelog entry (instead of publishing an empty release)."""
import re
import sys


def extract(readme_text, version):
    """Return (heading line, body). The heading looks like "### v0.2 — 2026-11-10"; the body ends before the next "### v", "<!--" or "---"."""
    lines = readme_text.replace('\r\n', '\n').split('\n')
    head = re.compile(r'^### v%s(\s|$|[—-])' % re.escape(version))
    start = next((i for i, l in enumerate(lines) if head.match(l)), None)
    if start is None:
        return None, None
    end = len(lines)
    for j in range(start + 1, len(lines)):
        if lines[j].startswith('### v') or lines[j].startswith('<!--') or lines[j].strip() == '---':
            end = j
            break
    return lines[start], '\n'.join(lines[start + 1:end]).strip('\n')


def main():
    if len(sys.argv) < 3:
        sys.exit('usage: release-notes.py <version> <image prefix, e.g. ghcr.io/yumin-space> [README path]')
    version, prefix = sys.argv[1].lstrip('v'), sys.argv[2].rstrip('/')
    readme = sys.argv[3] if len(sys.argv) > 3 else 'README.md'
    title, body = extract(open(readme, encoding='utf-8').read(), version)
    if not body:
        sys.exit('the changelog in %s has no entry for v%s (or the entry is empty)' % (readme, version))
    print(body)
    print()
    print('---')
    print()
    print('**Images** (two: the Nginx frontend and the Rust backend; `amd64` and `arm64` are in the same tag, the right one is picked on pull)')
    print()
    print('```bash')
    print('# Recommended: pull and start both with the docker-compose.yml in the repository')
    print('docker compose pull && docker compose up -d')
    print()
    print('# Or pull them one by one, pinned to this version')
    print('docker pull %s/xexam-frontend:v%s' % (prefix, version))
    print('docker pull %s/xexam-backend:v%s' % (prefix, version))
    print('```')
    print()
    print('Full documentation: [English](../../blob/main/README.md) · [日本語](../../blob/main/README.ja.md) · [简体中文](../../blob/main/README.zh-CN.md)')


main()
