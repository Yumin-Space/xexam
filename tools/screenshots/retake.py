#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""Retakes every README / docs screenshot of Xexam with one command, from a throw-away instance with made-up demo data.

    python tools/screenshots/retake.py [--lang en|ja|zh-CN|all] [--out DIR] [--chrome PATH] [--only NAMES]
                                       [--backend docker|cargo] [--verbose]

1. builds the demo question banks (demo_banks.py) into a temporary folder;
2. builds the backend image (backend/Dockerfile) and the ai-service image (ai-service/Dockerfile) under unique tags and
   runs them on a new Docker network: the backend with a new volume on /data and a new link volume on /link (network
   alias backend), ai-service with its own new volume and the link volume read only, both with TZ=UTC and
   XEXAM_VERSION = CODE_VERSION of VERSION and published on free ports of 127.0.0.1 only; tests/integration/ai_stub.py
   runs on the same network as the made-up AI provider (alias ai-stub)
   (--backend cargo: builds backend/ and ai-service/ with cargo and runs the binaries on temporary data folders, with
   the stub as a local process, instead);
3. builds the frontend with Vite (frontend/, into the temporary folder) and serves it with `vite preview` on a free port,
   with /api proxied to that backend and /api/ai to that AI service (vite.config.ts checks each AI request with the
   backend first, like Nginx does);
4. seeds users, banks, records and the AI settings (seed.py), restarts the backend so the System log starts empty, adds
   a little everyday activity for that log and waits until ai-service has reported to the restarted backend (AI+
   available again);
5. shoots every scene (scenes.py) for each language with headless Chrome and a temporary profile, into
   docs/<lang>/screenshots, or <DIR>/<lang> with --out; PNG text / time / EXIF chunks are stripped;
6. removes the containers, the volumes, the network, the image tags, the temporary folders and every process it
   started, also when something failed or the run was interrupted.

Several runs may go at the same time (for example one per language, each with its own --lang or --out): every image
tag, container, volume and network name carries a random suffix of its run, the ports are free ports of 127.0.0.1 (a
port another run took in the meantime is retried with a new one), Chrome gets its own temporary profile and DevTools
port, and `npm ci` (only when frontend/node_modules is missing) runs once at a time.

Needs Python 3.8+ with openpyxl, Node.js with frontend/node_modules (`npm ci` is run when they are missing), Docker
(or cargo with --backend cargo) and Chrome / Chromium (--chrome, or the XEXAM_CHROME / CHROME_PATH environment
variable, else the usual install locations). Nothing is pushed or published. Every value the images show is made up:
the stub's address, the fake key (shown as ...9c21), the demo-* models, the demo users and banks; the tool never
connects to any other instance.
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
STUB = os.path.join(REPO, 'tests', 'integration', 'ai_stub.py')


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


def wait_http(url, seconds=90, why='', status=200, proc=None):
    """Waits until `url` answers with `status` (200, or for example 401 for a route that needs a session). With `proc`,
    gives up as soon as that process has ended (then the port may be answered by someone else)."""
    deadline = time.time() + seconds
    last = None
    while time.time() < deadline:
        if proc is not None and proc.poll() is not None:
            raise RuntimeError('%s: the process that should answer it ended (exit code %s)%s'
                               % (url, proc.returncode, why))
        try:
            with urllib.request.urlopen(url, timeout=5) as r:
                if r.status == status:
                    return
                last = 'status %d' % r.status
        except urllib.error.HTTPError as e:
            if e.code == status:
                return
            last = 'status %d' % e.code
        except (urllib.error.URLError, OSError) as e:
            last = e
        time.sleep(0.5)
    raise RuntimeError('%s did not answer %d within %d s (%s)%s' % (url, status, seconds, last, why))


class DockerBackend:
    """backend/Dockerfile and ai-service/Dockerfile as throw-away containers on their own network, with the made-up AI
    provider next to them: unique image tags, container, volume and network names."""

    log_ok = False
    stub_base = 'http://ai-stub:9000/v1'          # a single-label host: the AI settings page shows no http:// warning

    def __init__(self, cleanup, version, verbose):
        token = secrets.token_hex(4)
        self.tag = 'xexam-shots-backend:' + token
        self.ai_tag = 'xexam-shots-ai-service:' + token
        self.name = self.volume = self.network = 'xexam-shots-' + token
        self.link = 'xexam-shots-link-' + token
        self.ai_name = self.ai_volume = 'xexam-shots-ai-service-' + token
        self.stub_name = 'xexam-shots-stub-' + token
        self.port, self.ai_port = free_port(), free_port()
        self.cleanup, self.version, self.verbose = cleanup, version, verbose

    @property
    def url(self):
        return 'http://127.0.0.1:%d' % self.port

    @property
    def ai_url(self):
        return 'http://127.0.0.1:%d' % self.ai_port

    def _run_published(self, name, attr, inner, args):
        """`docker run -d --name name -p 127.0.0.1:<self.attr>:inner args...`; when another run took that port in the
        meantime, the container is removed and started again on a new free port (stored in self.attr)."""
        for attempt in range(6):
            out = docker('run', '-d', '--name', name, '-p', '127.0.0.1:%d:%d' % (getattr(self, attr), inner), *args,
                         check=False)
            if docker('inspect', '-f', '{{.State.Running}}', name, check=False).strip() == 'true':
                return
            if attempt < 5 and re.search(r'port is already allocated|ports are not available|address already in use',
                                         out, re.I):
                docker('rm', '-f', '-v', name, check=False)
                setattr(self, attr, free_port())
                continue
            logs = docker('logs', '--tail', '40', name, check=False)
            raise RuntimeError('docker run %s failed: %s\n%s' % (name, out.strip()[-2000:], logs.strip()))

    def start(self):
        if subprocess.run(['docker', 'info'], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL).returncode:
            raise RuntimeError('Docker is not running (start it, or use --backend cargo)')
        for tag, dockerfile in ((self.tag, 'backend/Dockerfile'), (self.ai_tag, 'ai-service/Dockerfile')):
            print('building the image %s (takes minutes when nothing is cached) ...' % tag)
            self.cleanup.add('remove the image ' + tag, docker, 'rmi', '-f', tag)
            run(['docker', 'build', '--progress=plain', '-f', dockerfile, '-t', tag, '.'], cwd=REPO,
                verbose=self.verbose, what='docker build ' + dockerfile)
        # Undone in reverse order: the containers first, then the volumes, then the network
        docker('network', 'create', self.network)
        self.cleanup.add('remove the network ' + self.network, docker, 'network', 'rm', self.network)
        for volume in (self.volume, self.link, self.ai_volume):
            docker('volume', 'create', volume)
            self.cleanup.add('remove the volume ' + volume, docker, 'volume', 'rm', '-f', volume)
        self.cleanup.add('remove the container ' + self.name, docker, 'rm', '-f', '-v', self.name)
        self._run_published(self.name, 'port', 8765, [
            '--network', self.network, '--network-alias', 'backend', '-e', 'TZ=UTC',
            '-e', 'XEXAM_VERSION=' + self.version, '-v', self.volume + ':/data', '-v', self.link + ':/link', self.tag])
        self._ready()
        self.cleanup.add('remove the container ' + self.stub_name, docker, 'rm', '-f', self.stub_name)
        docker('run', '-d', '--name', self.stub_name, '--network', self.network, '--network-alias', 'ai-stub',
               '-v', STUB + ':/stub.py:ro', 'python:3.12-slim', 'python', '-I', '/stub.py', '--host', '0.0.0.0',
               '--port', '9000')
        self.cleanup.add('remove the container ' + self.ai_name, docker, 'rm', '-f', '-v', self.ai_name)
        self._run_published(self.ai_name, 'ai_port', 8766, [
            '--network', self.network, '-e', 'TZ=UTC', '-e', 'XEXAM_VERSION=' + self.version,
            '-v', self.ai_volume + ':/data', '-v', self.link + ':/link:ro', self.ai_tag])
        try:
            wait_http(self.ai_url + '/healthz', why=' (ai-service)')
        except RuntimeError:
            print(docker('logs', '--tail', '40', self.ai_name, check=False))
            raise

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


def cargo_binary(cargo, crate, name, verbose):
    """`cargo build --release` of `crate` (backend or ai-service); returns the path of the binary `name`."""
    manifest = os.path.join(REPO, crate, 'Cargo.toml')
    print('building %s/ with cargo (release) ...' % crate)
    run([cargo, 'build', '--release', '--manifest-path', manifest], verbose=verbose, what='cargo build ' + crate)
    meta = subprocess.run([cargo, 'metadata', '--format-version', '1', '--no-deps', '--manifest-path', manifest],
                          stdout=subprocess.PIPE, check=True, encoding='utf-8')
    target = json.loads(meta.stdout)['target_directory']        # honours CARGO_TARGET_DIR
    return os.path.join(target, 'release', name + ('.exe' if sys.platform == 'win32' else ''))


class CargoBackend:
    """Fallback without Docker: `cargo build --release` of backend/ and ai-service/, the binaries on temporary data
    folders (one link folder shared by both) and the made-up AI provider as a local process on 127.0.0.1 (a loopback
    address: the AI settings page shows no http:// warning). The System log would show the data folder's host path, so
    its screenshot is skipped (the Docker backend shows /data)."""

    log_ok = False

    def __init__(self, cleanup, version, verbose, work):
        self.port, self.ai_port, self.stub_port = free_port(), free_port(), free_port()
        self.url = 'http://127.0.0.1:%d' % self.port
        self.ai_url = 'http://127.0.0.1:%d' % self.ai_port
        self.stub_base = 'http://127.0.0.1:%d/v1' % self.stub_port
        self.cleanup, self.version, self.verbose = cleanup, version, verbose
        self.work = work
        self.data = os.path.join(work, 'data')
        self.ai_data = os.path.join(work, 'data-ai')
        self.link = os.path.join(work, 'link')
        self.logfile = os.path.join(work, 'backend.log')
        self.proc = None
        self.exe = None

    def start(self):
        cargo = shutil.which('cargo')
        if not cargo:
            raise RuntimeError('cargo not found')
        self.exe = cargo_binary(cargo, 'backend', 'xexam-backend', self.verbose)
        ai_exe = cargo_binary(cargo, 'ai-service', 'xexam-ai-service', self.verbose)
        os.makedirs(self.data, exist_ok=True)
        self._spawn()
        log = open(os.path.join(self.work, 'stub.log'), 'w', encoding='utf-8')
        self.cleanup.add('close the stub log', log.close)
        stub = popen_group([sys.executable, '-I', STUB, '--host', '127.0.0.1', '--port', str(self.stub_port)],
                           stdout=log, stderr=subprocess.STDOUT)
        self.cleanup.add('stop the made-up AI provider', kill_tree, stub)
        env = dict(os.environ, TZ='UTC', XEXAM_VERSION=self.version, XEXAM_AI_DATA=self.ai_data,
                   XEXAM_AI_HOST='127.0.0.1', XEXAM_AI_PORT=str(self.ai_port), XEXAM_AI_BACKEND_URL=self.url,
                   XEXAM_LINK_DIR=self.link)
        for k in ('XEXAM_INTERNAL_TOKEN', 'XEXAM_AI_SECRET_KEY'):
            env.pop(k, None)
        log = open(os.path.join(self.work, 'ai.log'), 'w', encoding='utf-8')
        self.cleanup.add('close the ai-service log', log.close)
        proc = popen_group([ai_exe, 'serve'], stdout=log, stderr=subprocess.STDOUT, env=env, cwd=self.work)
        self.cleanup.add('stop ai-service', kill_tree, proc)
        # proc: when its port was taken (by another run, say), fail at once instead of talking to someone else's server
        wait_http(self.ai_url + '/healthz', why=' (ai-service)', proc=proc)

    def _spawn(self):
        env = dict(os.environ, TZ='UTC', XEXAM_DATA=self.data, XEXAM_VERSION=self.version, XEXAM_LINK_DIR=self.link)
        for k in ('XEXAM_ALLOWED_HOSTS', 'XEXAM_ALLOWED_ORIGINS', 'XEXAM_ALLOW_ANY_HOST', 'XEXAM_INTERNAL_TOKEN'):
            env.pop(k, None)
        log = open(self.logfile, 'a', encoding='utf-8')
        self.proc = popen_group([self.exe, 'serve', '--host', '127.0.0.1', '--port', str(self.port)],
                                stdout=log, stderr=subprocess.STDOUT, env=env, cwd=os.path.dirname(self.data))
        proc = self.proc
        self.cleanup.add('stop the backend', kill_tree, proc)
        self.cleanup.add('close the backend log', log.close)
        wait_http(self.url + '/api/info', proc=proc)

    def restart(self):
        kill_tree(self.proc)
        self._spawn()
        print('note: --backend cargo skips desktop-13-logs (the log would show a host path as the data directory)')


def start_frontend(work, backend_url, ai_url, cleanup, verbose):
    """`vite build` into the temporary folder, then `vite preview` on a free port with /api proxied to the backend and
    /api/ai to ai-service (after the auth step of vite.config.ts)."""
    fe = os.path.join(REPO, 'frontend')
    node = shutil.which('node')
    if not node:
        raise RuntimeError('Node.js (node) not found')
    vite = os.path.join(fe, 'node_modules', 'vite', 'bin', 'vite.js')
    if not os.path.isfile(vite):
        npm_ci(fe, vite, verbose)
    dist = os.path.join(work, 'web')
    print('building the frontend (vite build) ...')
    run([node, vite, 'build', '--outDir', dist, '--emptyOutDir', '--logLevel', 'warn'], cwd=fe, verbose=verbose,
        what='vite build')
    for attempt in range(6):
        port = free_port()
        logname = os.path.join(work, 'vite-%d.log' % attempt)
        log = open(logname, 'w', encoding='utf-8')
        cleanup.add('close the vite log', log.close)
        env = dict(os.environ, XEXAM_BACKEND=backend_url, XEXAM_AI_BACKEND=ai_url)
        proc = popen_group([node, vite, 'preview', '--outDir', dist, '--host', '127.0.0.1', '--port', str(port),
                            '--strictPort'], cwd=fe, env=env, stdout=log, stderr=subprocess.STDOUT)
        cleanup.add('stop vite preview', kill_tree, proc)
        base = 'http://127.0.0.1:%d' % port
        try:
            wait_http(base + '/', 60, ' (vite preview)', proc=proc)
            break
        except RuntimeError:
            log.flush()
            with open(logname, encoding='utf-8', errors='replace') as fh:
                taken = 'already in use' in fh.read()
            if proc.poll() is None or not taken or attempt == 5:
                raise
            print('the port %d was taken by another program in the meantime; trying another one' % port)
    wait_http(base + '/api/info', 30, ' (the /api proxy to the backend)')
    # Without a session the AI route must be refused by the backend's auth step (proves preview -> backend -> AI chain)
    wait_http(base + '/api/ai/status', 30, ' (the auth step of /api/ai through the backend)', status=401)
    return base


def npm_ci(fe, vite, verbose):
    """`npm ci` in frontend/, one run at a time on this machine (a lock file in the temporary folder), and only when
    node_modules is still missing once the lock is held."""
    npm = shutil.which('npm')
    if not npm:
        raise RuntimeError('frontend/node_modules is missing and npm was not found')
    lock = os.path.join(tempfile.gettempdir(), 'xexam-shots-npm-ci.lock')
    deadline = time.time() + 900
    while True:
        try:
            fd = os.open(lock, os.O_CREAT | os.O_EXCL | os.O_WRONLY)
            break
        except FileExistsError:
            if time.time() > deadline:
                raise RuntimeError('another run has held %s for 15 minutes (remove it if no run is going)' % lock)
            time.sleep(2)
    try:
        if not os.path.isfile(vite):
            print('frontend/node_modules is missing: running npm ci in frontend/ ...')
            run([npm, 'ci'], cwd=fe, verbose=verbose, what='npm ci')
    finally:
        os.close(fd)
        os.remove(lock)


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
        base = start_frontend(work, backend.url, backend.ai_url, cleanup, args.verbose)
        print('seeding the demo data ...')
        seed.prepare(base, banks, backend.stub_base)
        backend.restart()                            # the System log starts empty again
        wait_http(base + '/api/info', 30)
        cookie = seed.activity(base)                 # ends when ai-service has reported to the restarted backend
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
