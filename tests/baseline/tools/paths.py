# -*- coding: utf-8 -*-
"""基线测试用到的所有路径，都从这里取，不写死某台电脑的绝对路径。

  LEGACY_ROOT   被测的 Python 旧版代码所在目录（里面有 server.py、app/、web/）。
                默认：本仓库根目录（迁移前）；迁移后在 legacy/python/。可用环境变量 XEXAM_LEGACY_ROOT 指定。
  FIXTURES      tests/baseline/fixtures（虚构的演示工作簿、合成的边界用例、示例数据库）
  GOLDEN        tests/baseline/golden（旧版的真实响应）
  DEMO / DEMO2  虚构的 5 题演示工作簿
"""
import os

HERE = os.path.dirname(os.path.abspath(__file__))
BASELINE = os.path.dirname(HERE)
REPO_ROOT = os.path.dirname(os.path.dirname(BASELINE))


def _find_legacy_root():
    env = os.environ.get('XEXAM_LEGACY_ROOT')
    if env:
        return os.path.abspath(env)
    for cand in (REPO_ROOT, os.path.join(REPO_ROOT, 'legacy', 'python')):
        if os.path.isfile(os.path.join(cand, 'server.py')):
            return cand
    raise SystemExit('找不到旧版 Python 代码（server.py）。请设置环境变量 XEXAM_LEGACY_ROOT。')


LEGACY_ROOT = _find_legacy_root()
FIXTURES = os.path.join(BASELINE, 'fixtures')
GOLDEN = os.path.join(BASELINE, 'golden')
PARSER_GOLDEN = os.path.join(BASELINE, 'parser_golden')
DEMO = os.path.join(FIXTURES, 'demo.xlsx')
DEMO2 = os.path.join(FIXTURES, 'demo_v2.xlsx')
