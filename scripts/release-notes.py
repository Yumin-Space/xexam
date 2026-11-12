#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""Build the GitHub Release notes for one version: take that version's entry out of the English changelog (docs/en/changelog.md) and
append how to pull the three images (the Nginx frontend, the Rust backend and the AI service ai-service).

usage:  python3 scripts/release-notes.py <version, e.g. 0.4> <image prefixes, e.g. docker.io/name,ghcr.io/yumin-space> [changelog path, default docs/en/changelog.md]  > notes.md
The first prefix is the main registry (the one docker/docker-compose.yml pulls from); any further ones are listed as mirrors.
The release job of the pipeline runs this; you can also run it locally to preview what the Releases page will show.
Exits with status 1 if the version has no changelog entry (instead of publishing an empty release)."""
import re
import sys

DEFAULT_CHANGELOG = 'docs/en/changelog.md'


def extract(changelog_text, version):
    """Return (heading line, body). The heading looks like "### v0.2 — 2026-11-10"; the body ends before the next "### v", "<!--" or "---"."""
    lines = changelog_text.replace('\r\n', '\n').split('\n')
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
        sys.exit('usage: release-notes.py <version> <image prefixes, e.g. docker.io/name,ghcr.io/yumin-space> [changelog path, default docs/en/changelog.md]')
    version = sys.argv[1].lstrip('v')
    prefixes = [x.strip().rstrip('/') for x in sys.argv[2].split(',') if x.strip()]
    prefix, mirrors = prefixes[0], prefixes[1:]
    path = sys.argv[3] if len(sys.argv) > 3 else DEFAULT_CHANGELOG
    title, body = extract(open(path, encoding='utf-8').read(), version)
    if not body:
        sys.exit('the changelog %s has no entry for v%s (or the entry is empty)' % (path, version))
    print(body)
    print()
    print('---')
    print()
    print('**Images** (three: the Nginx frontend, the Rust backend and the AI service; `amd64` and `arm64` are in the same tag, the right one is picked on pull)')
    print()
    print('```bash')
    print('# Recommended: in the folder that holds docker-compose.yml, pull and start all three (from the root of a clone, add -f docker/docker-compose.yml)')
    print('docker compose pull && docker compose up -d')
    print()
    print('# Or pull them one by one, pinned to this version')
    print('docker pull %s/xexam-frontend:v%s' % (prefix, version))
    print('docker pull %s/xexam-backend:v%s' % (prefix, version))
    print('docker pull %s/xexam-ai-service:v%s' % (prefix, version))
    for m in mirrors:
        print()
        print('# The same images on %s (private mirror: docker login %s first)' % (m.split('/')[0], m.split('/')[0]))
        print('docker pull %s/xexam-frontend:v%s' % (m, version))
        print('docker pull %s/xexam-backend:v%s' % (m, version))
        print('docker pull %s/xexam-ai-service:v%s' % (m, version))
    print('```')
    print()
    print('Full documentation: [English](../../blob/main/README.md) · [日本語](../../blob/main/README/README.ja.md) · [简体中文](../../blob/main/README/README.zh-CN.md)')


main()
