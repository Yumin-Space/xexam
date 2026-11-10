#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""生成某个版本的 GitHub Release 说明：从 README 的「更新日志」里抽出这个版本的那一段，再加上镜像的拉取方法。

用法:  python3 scripts/release-notes.py <版本，如 0.3> <镜像前缀，如 ghcr.io/yumin-space> [README 路径]  > notes.md
CI 的 release job 用它；本机也能直接跑，用来预览 Release 页面会显示什么。
找不到这个版本的更新日志就退出码 1（而不是发一个空的 Release）。"""
import re
import sys


def extract(readme_text, version):
    """返回 (标题行, 正文)。标题形如「### v0.2 — 2026-11-10」，正文到下一个「### v」、「<!--」或「---」之前。"""
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
        sys.exit('用法: release-notes.py <版本> <镜像前缀，如 ghcr.io/yumin-space> [README 路径]')
    version, prefix = sys.argv[1].lstrip('v'), sys.argv[2].rstrip('/')
    readme = sys.argv[3] if len(sys.argv) > 3 else 'README.md'
    title, body = extract(open(readme, encoding='utf-8').read(), version)
    if not body:
        sys.exit('README 的更新日志里找不到 v%s 这一段（或这一段是空的）' % version)
    print(body)
    print()
    print('---')
    print()
    print('**镜像**（两个：前端 Nginx 和 Rust 后端；x86 的 `amd64` 和 ARM 的 `arm64` 都在同一个标签里，拉取时自动选对）')
    print()
    print('```bash')
    print('# 推荐：用仓库里的 docker-compose.yml 一次拉取并启动两个')
    print('docker compose pull && docker compose up -d')
    print()
    print('# 或者单独拉取，固定在这个版本')
    print('docker pull %s/xexam-frontend:v%s' % (prefix, version))
    print('docker pull %s/xexam-backend:v%s' % (prefix, version))
    print('```')
    print()
    print('更新方法和完整说明见 [README](../../blob/main/README.md)。')


main()
