#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""Retakes every README / docs screenshot of Xexam with one command, from a throw-away instance with made-up demo data.

    python tools/screenshots/retake.py [--lang en|ja|zh-CN|all] [--out DIR] [--chrome PATH] [--only NAMES]
                                       [--backend docker|cargo] [--verbose]

1. builds the demo question banks (demo_banks.py) into a temporary folder;
2. builds the backend image from backend/Dockerfile under a unique tag and runs it with a new volume on /data, TZ=UTC
   and XEXAM_VERSION = CODE_VERSION of VERSION, published on a free port of 127.0.0.1 only
   (--backend cargo: builds backend/ with cargo and runs the binary on a temporary data folder instead);
3. builds the frontend with Vite (frontend/, into the temporary folder) and serves it with `vite preview` on a free port,
   with /api proxied to that backend;
4. seeds users, banks and records (seed.py), restarts the backend so the System log starts empty, and adds a little
   everyday activity for that log;
5. shoots every scene (scenes.py) for each language with headless Chrome and a temporary profile, into
   docs/<lang>/screenshots, or <DIR>/<lang> with --out; PNG text / time / EXIF chunks are stripped;
6. removes the container, the volume, the image tag, the temporary folders and every process it started, also when
   something failed or the run was interrupted.

Needs Python 3.8+ with openpyxl, Node.js with frontend/node_modules (`npm ci` is run when they are missing), Docker
(or cargo with --backend cargo) and Chrome / Chromium (--chrome, or the XEXAM_CHROME / CHROME_PATH environment
variable, else the usual install locations). Nothing is pushed or published.
"""
import argparse
import json
import os
import re
import secrets
import shutil
import signal
import socket
import subprocess
import sys
import tempfile
import time
import traceback
import urllib.error
import urllib.request

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)

import demo_banks  # noqa: E402
import scenes  # noqa: E402
import seed  # noqa: E402
from browser import find_chrome, kill_tree, popen_group, remove_dir  # noqa: E402

REPO = os.path.dirname(os.path.dirname(HERE))
LANGS = ['en', 'ja', 'zh-CN']


class Cleanup:
    """Undo steps, run in reverse order at the end, whatever happened before."""

    def __init__(self):
        self.steps = []

    def add(self, what, fn, *args):
        self.steps.append((what, fn, args))

    def run(self):
        while self.steps:
            what, fn, args = self.steps.pop()
            try:
                fn(*args)
            except Exception as e:  # keep going: every other step must still run
                print('cleanup: could not %s: %s' % (what, e))


def shown_path(path):
    """`path` relative to the working directory when it can be (os.path.relpath fails on Windows across drives)."""
    try:
        return os.path.relpath(path)
    except ValueError:
        return os.path.abspath(path)


def free_port():
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as s:
        s.bind(('127.0.0.1', 0))
        return s.getsockname()[1]


def code_version():
    with open(os.path.join(REPO, 'VERSION'), encoding='utf-8') as fh:
        m = re.search(r'^CODE_VERSION=\s*(\S+)\s*$', fh.read(), re.M)
    if not m:
        raise SystemExit('VERSION has no CODE_VERSION= line')
    return m.group(1)


def run(args, cwd=None, env=None, verbose=False, what=None):
    """Runs a command to its end; its output is shown with --verbose, or its tail when it fails."""
    if verbose:
        r = subprocess.run(args, cwd=cwd, env=env)
        out = ''
    else:
        r = subprocess.run(args, cwd=cwd, env=env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                           encoding='utf-8', errors='replace')
        out = r.stdout or ''
    if r.returncode:
        tail = '\n'.join(out.splitlines()[-60:])
        raise RuntimeError('%s failed (exit code %d)%s' % (what or args[0], r.returncode, ':\n' + tail if tail else ''))
    return out


def docker(*args, check=True):
    r = subprocess.run(['docker'] + list(args), stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                       encoding='utf-8', errors='replace')
    if check and r.returncode:
        raise RuntimeError('docker %s failed: %s' % (args[0], (r.stdout or '').strip()[-2000:]))
    return r.stdout or ''


def wait_http(url, seconds=90, why=''):
    deadline = time.time() + seconds
    last = None
    while time.time() < deadline:
        try:
            with urllib.request.urlopen(url, timeout=5) as r:
                if r.status == 200:
                    return
        except (urllib.error.URLError, OSError) as e:
            last = e
        time.sleep(0.5)
    raise RuntimeError('%s did not answer within %d s (%s)%s' % (url, seconds, last, why))


class DockerBackend:
    """backend/Dockerfile as a throw-away container: unique image tag, container and volume names."""

    log_ok = False

    def __init__(self, cleanup, version, verbose):
        token = secrets.token_hex(4)
        self.tag = 'xexam-shots-backend:' + token
        self.name = self.volume = 'xexam-shots-' + token
        self.port = free_port()
        self.url = 'http://127.0.0.1:%d' % self.port
        self.cleanup, self.version, self.verbose = cleanup, version, verbose

    def start(self):
        if subprocess.run(['docker', 'info'], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL).returncode:
            raise RuntimeError('Docker is not running (start it, or use --backend cargo)')
        print('building the backend image %s (takes minutes when nothing is cached) ...' % self.tag)
        self.cleanup.add('remove the image ' + self.tag, docker, 'rmi', '-f', self.tag)
        run(['docker', 'build', '--progress=plain', '-f', 'backend/Dockerfile', '-t', self.tag, '.'], cwd=REPO,
            verbose=self.verbose, what='docker build')
        docker('volume', 'create', self.volume)
        self.cleanup.add('remove the volume ' + self.volume, docker, 'volume', 'rm', '-f', self.volume)
        self.cleanup.add('remove the container ' + self.name, docker, 'rm', '-f', '-v', self.name)
        docker('run', '-d', '--name', self.name, '-e', 'TZ=UTC', '-e', 'XEXAM_VERSION=' + self.version,
               '-p', '127.0.0.1:%d:8765' % self.port, '-v', self.volume + ':/data', self.tag)
        self._ready()

    def _ready(self):
        try:
            wait_http(self.url + '/api/info')
        except RuntimeError:
            print(docker('logs', '--tail', '40', self.name, check=False))
            raise

    def restart(self):
        docker('restart', '-t', '10', self.name)
        self._ready()
        self.log_ok = 'Data directory: /data' in docker('logs', self.name, check=False)
        if not self.log_ok:
            print('warning: the backend log does not say "Data directory: /data"')


class CargoBackend:
    """Fallback without Docker: `cargo build --release` of backend/ and the binary on a temporary data folder. The
    System log would show that folder's host path, so its screenshot is skipped (the Docker backend shows /data)."""

    log_ok = False

    def __init__(self, cleanup, version, verbose, work):
        self.port = free_port()
        self.url = 'http://127.0.0.1:%d' % self.port
        self.cleanup, self.version, self.verbose = cleanup, version, verbose
        self.data = os.path.join(work, 'data')
        self.logfile = os.path.join(work, 'backend.log')
        self.proc = None
        self.exe = None

    def start(self):
        cargo = shutil.which('cargo')
        if not cargo:
            raise RuntimeError('cargo not found')
        manifest = os.path.join(REPO, 'backend', 'Cargo.toml')
        print('building the backend with cargo (release) ...')
        run([cargo, 'build', '--release', '--manifest-path', manifest], verbose=self.verbose, what='cargo build')
        meta = subprocess.run([cargo, 'metadata', '--format-version', '1', '--no-deps', '--manifest-path', manifest],
                              stdout=subprocess.PIPE, check=True, encoding='utf-8')
        target = json.loads(meta.stdout)['target_directory']        # honours CARGO_TARGET_DIR
        self.exe = os.path.join(target, 'release', 'xexam-backend' + ('.exe' if sys.platform == 'win32' else ''))
        os.makedirs(self.data, exist_ok=True)
        self._spawn()

    def _spawn(self):
        env = dict(os.environ, TZ='UTC', XEXAM_DATA=self.data, XEXAM_VERSION=self.version)
        for k in ('XEXAM_ALLOWED_HOSTS', 'XEXAM_ALLOWED_ORIGINS', 'XEXAM_ALLOW_ANY_HOST'):
            env.pop(k, None)
        log = open(self.logfile, 'a', encoding='utf-8')
        self.proc = popen_group([self.exe, 'serve', '--host', '127.0.0.1', '--port', str(self.port)],
                                stdout=log, stderr=subprocess.STDOUT, env=env, cwd=os.path.dirname(self.data))
        proc = self.proc
        self.cleanup.add('stop the backend', kill_tree, proc)
        self.cleanup.add('close the backend log', log.close)
        wait_http(self.url + '/api/info')

    def restart(self):
        kill_tree(self.proc)
        self._spawn()
        print('note: --backend cargo skips desktop-13-logs (the log would show a host path as the data directory)')


def start_frontend(work, backend_url, cleanup, verbose):
    """`vite build` into the temporary folder, then `vite preview` on a free port with /api proxied to the backend."""
    fe = os.path.join(REPO, 'frontend')
    node = shutil.which('node')
    if not node:
        raise RuntimeError('Node.js (node) not found')
    vite = os.path.join(fe, 'node_modules', 'vite', 'bin', 'vite.js')
    if not os.path.isfile(vite):
        npm = shutil.which('npm')
        if not npm:
            raise RuntimeError('frontend/node_modules is missing and npm was not found')
        print('frontend/node_modules is missing: running npm ci in frontend/ ...')
        run([npm, 'ci'], cwd=fe, verbose=verbose, what='npm ci')
    dist = os.path.join(work, 'web')
    print('building the frontend (vite build) ...')
    run([node, vite, 'build', '--outDir', dist, '--emptyOutDir', '--logLevel', 'warn'], cwd=fe, verbose=verbose,
        what='vite build')
    port = free_port()
    log = open(os.path.join(work, 'vite.log'), 'w', encoding='utf-8')
    cleanup.add('close the vite log', log.close)
    proc = popen_group([node, vite, 'preview', '--outDir', dist, '--host', '127.0.0.1', '--port', str(port), '--strictPort'],
                       cwd=fe, env=dict(os.environ, XEXAM_BACKEND=backend_url), stdout=log, stderr=subprocess.STDOUT)
    cleanup.add('stop vite preview', kill_tree, proc)
    base = 'http://127.0.0.1:%d' % port
    wait_http(base + '/', 60)
    wait_http(base + '/api/info', 30, ' (the /api proxy to the backend)')
    return base


def parse_args():
    p = argparse.ArgumentParser(description='Retake the README / docs screenshots from a throw-away demo instance.')
    p.add_argument('--lang', default='all', choices=LANGS + ['all'], help='one language, or all (default)')
    p.add_argument('--out', help='write to OUT/<lang>/ instead of docs/<lang>/screenshots/')
    p.add_argument('--chrome', help='Chrome / Chromium executable (default: XEXAM_CHROME, CHROME_PATH, the usual places)')
    p.add_argument('--only', help='comma-separated file names to retake, e.g. desktop-13-logs.png (the rest are skipped)')
    p.add_argument('--backend', default='docker', choices=['docker', 'cargo'], help='how to run the backend (default docker)')
    p.add_argument('--verbose', action='store_true', help='show the output of the builds')
    return p.parse_args()


def main():
    args = parse_args()
    langs = LANGS if args.lang == 'all' else [args.lang]
    only = [n.strip() for n in (args.only or '').split(',') if n.strip()]
    unknown = [n for n in only if n not in scenes.SCENES]
    if unknown:
        raise SystemExit('unknown scene(s): %s (known: %s)' % (', '.join(unknown), ', '.join(scenes.SCENES)))
    chrome = find_chrome(args.chrome)
    if not chrome:
        raise SystemExit('no Chrome / Chromium found: pass --chrome PATH or set XEXAM_CHROME')
    version = code_version()
    out_root = os.path.abspath(args.out) if args.out else None

    def stop(signum, frame):
        raise KeyboardInterrupt
    for name in ('SIGTERM', 'SIGBREAK', 'SIGHUP'):
        if hasattr(signal, name):
            signal.signal(getattr(signal, name), stop)

    started = time.time()
    cleanup = Cleanup()
    written = []
    code = 0
    try:
        work = tempfile.mkdtemp(prefix='xexam-shots-')
        cleanup.add('remove the temporary folder', remove_dir, work)
        banks = demo_banks.build(os.path.join(work, 'banks'))
        print('Xexam v%s screenshots, languages: %s' % (version, ', '.join(langs)))
        if args.backend == 'docker':
            backend = DockerBackend(cleanup, version, args.verbose)
        else:
            backend = CargoBackend(cleanup, version, args.verbose, work)
        backend.start()
        base = start_frontend(work, backend.url, cleanup, args.verbose)
        print('seeding the demo data ...')
        seed.prepare(base, banks)
        backend.restart()                            # the System log starts empty again
        wait_http(base + '/api/info', 30)
        cookie = seed.activity(base)
        seed.seed_exams(base, cookie)
        for lang in langs:
            out = os.path.join(out_root, lang) if out_root else os.path.join(REPO, 'docs', lang, 'screenshots')
            print('== %s -> %s' % (lang, shown_path(out)))
            written += scenes.shoot_language(chrome, base, cookie, lang, out, banks[demo_banks.KITCHEN_V2], only,
                                             with_log=backend.log_ok)
    except KeyboardInterrupt:
        print('interrupted')
        code = 130
    except Exception:
        traceback.print_exc()
        code = 1
    finally:
        print('cleaning up ...')
        cleanup.run()
    print('%s: %d image(s) written in %.0f s' % ('done' if code == 0 else 'stopped', len(written), time.time() - started))
    return code


if __name__ == '__main__':
    sys.exit(main())
