#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""The phone walk: drives the real Xexam UI on a phone-like Chrome against a throw-away stack with made-up data only.

    python tools/phone-walk/walk.py                      # default run, report + screenshots into a new temp folder
    python tools/phone-walk/walk.py --full               # every check on the whole matrix (final gate)
    python tools/phone-walk/walk.py --pages home,quiz,result,ai,menu
    python tools/phone-walk/walk.py --only W-QZ-*,W-CH-1
    python tools/phone-walk/walk.py --langs zh --widths 390x844,360x740 --themes light
    python tools/phone-walk/walk.py --keep               # leave the stack running: prints the URL and the made-up accounts
    python tools/phone-walk/walk.py --keep --lan         # the same, reachable from a phone on the home network
    python tools/phone-walk/walk.py --out DIR --chrome PATH

This file (T1a) owns the command line, the stack (start, seed, stop), `--keep` and `--lan`. The checks live in
`checks.py` / `checks.js` (T1b); when `checks.py` exists, walk.py calls its `run(walk)` (see README.md, "Contract with
checks.py"), otherwise it says so and ends after the stack has been proven healthy.

The stack reuses tools/screenshots (retake.DockerBackend / CargoBackend, seed.prepare, demo_banks, browser.Chrome / CDP):
the backend and ai-service images under unique tags, a new network and volumes, the made-up AI stub, TZ=UTC, the frontend
built with Vite and served by `vite preview` (with the auth step of Nginx), plus the extra made-up data of seed_extra.py.
It removes only what it created (no prune), also on Ctrl+C, and never connects to any other instance: the owner's
previews hold real credentials and are not even looked at.
"""
import argparse
import fnmatch
import importlib
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

HERE = os.path.dirname(os.path.abspath(__file__))
SHOTS = os.path.join(os.path.dirname(HERE), 'screenshots')
for path in (SHOTS, HERE):
    if path not in sys.path:
        sys.path.insert(0, path)

import demo_banks  # noqa: E402
import retake  # noqa: E402
import seed  # noqa: E402
import seed_extra  # noqa: E402
from browser import find_chrome, kill_tree, popen_group, remove_dir  # noqa: E402

REPO = retake.REPO
PAGES = ['home', 'quiz', 'result', 'records', 'ai', 'ai-settings', 'users', 'logs', 'settings', 'banks', 'about',
         'signin', 'forbidden', 'menu']
LANGS = ['zh', 'en', 'ja']
THEMES = ['light', 'dark']
DEFAULT_SIZES = ['360x740', '390x844', '844x390']
# The owner's preview ports (their instances hold real credentials): the walk never uses one of them
FORBIDDEN_PORTS = {5285, 5295, 8885, 8886, 8895, 8896, 9085, 8899}
STOP_FILE = 'STOP'
# (user, what it is) in the order the printout shows; every password is the made-up seed.DEMO_PASSWORD
ACCOUNTS = [
    ('admin', 'administrator, AI+ always'),
    ('lee', 'administrator, 60 study records'),
    ('sam', 'user, may manage banks, AI+'),
    ('riley', 'user, no AI+'),
    ('alex', 'user, AI+ paused (greyed AI controls)'),
    ('kim', 'user, disabled (cannot sign in)'),
    (seed_extra.LOCKED_USER, 'user, locked (cannot sign in)'),
]
CANNOT_SIGN_IN = ('kim', seed_extra.LOCKED_USER)


# ---------------------------------------------------------------- command line

def parse_size(text):
    m = re.fullmatch(r'(\d{3,4})x(\d{3,4})', text.strip().lower())
    if not m:
        raise argparse.ArgumentTypeError('a size looks like 390x844, not %r' % text)
    return int(m.group(1)), int(m.group(2))


def csv(text):
    return [x.strip() for x in text.split(',') if x.strip()]


def parse_args(argv=None):
    p = argparse.ArgumentParser(description='Drive Xexam on a phone-like Chrome against a throw-away stack of made-up data.')
    p.add_argument('--full', action='store_true', help='every check on the whole width matrix x 3 languages x 2 themes (final gate)')
    p.add_argument('--pages', help='only these pages: %s' % ', '.join(PAGES))
    p.add_argument('--only', help='only the checks matching these comma-separated patterns, e.g. W-QZ-*,W-CH-1')
    p.add_argument('--langs', help='languages, comma separated (zh, en, ja)')
    p.add_argument('--widths', help='sizes WxH, comma separated, e.g. 390x844,360x740 (default %s)' % ','.join(DEFAULT_SIZES))
    p.add_argument('--themes', help='themes, comma separated (light, dark)')
    p.add_argument('--slow', action='store_true', help='also run the checks that wait for a time window (W-CAP-1 after retryAt)')
    p.add_argument('--keep', action='store_true',
                   help='start and seed the stack, print the URL and the made-up accounts, run no checks, and wait for '
                        'Ctrl+C (or a file named STOP in the output folder); then remove everything it created')
    p.add_argument('--lan', action='store_true',
                   help='with --keep: publish the web port on this computer\'s home-network address instead of 127.0.0.1 '
                        '(for a phone on the same network; made-up data and the AI stub only)')
    p.add_argument('--out', help='folder for report.json and the screenshots (default: a new temporary folder)')
    p.add_argument('--chrome', help='Chrome / Chromium executable (default: XEXAM_CHROME, CHROME_PATH, the usual places)')
    p.add_argument('--backend', default='docker', choices=['docker', 'cargo'], help='how to run the backend (default docker)')
    p.add_argument('--frontend', help='build this frontend folder instead of frontend/ (for example the frontend of another '
                                      'checkout; its parent folder must hold docs/ for the changelog, and it needs node_modules)')
    p.add_argument('--verbose', action='store_true', help='show the output of the builds')
    args = p.parse_args(argv)
    if args.lan and not args.keep:
        p.error('--lan only works together with --keep (the LAN address is for a phone trial, never for the automated walk)')
    args.pages = csv(args.pages) if args.pages else None
    args.only = csv(args.only) if args.only else None
    args.langs = csv(args.langs) if args.langs else None
    args.themes = csv(args.themes) if args.themes else None
    if args.pages and (bad := [x for x in args.pages if x not in PAGES]):
        p.error('unknown page(s) %s (known: %s)' % (', '.join(bad), ', '.join(PAGES)))
    if args.langs and (bad := [x for x in args.langs if x not in LANGS]):
        p.error('unknown language(s) %s (known: %s)' % (', '.join(bad), ', '.join(LANGS)))
    if args.themes and (bad := [x for x in args.themes if x not in THEMES]):
        p.error('unknown theme(s) %s (known: %s)' % (', '.join(bad), ', '.join(THEMES)))
    try:
        args.widths = [parse_size(x) for x in csv(args.widths)] if args.widths else None
    except argparse.ArgumentTypeError as e:
        p.error(str(e))
    return args


# ---------------------------------------------------------------- network helpers

def safe_port(host='127.0.0.1'):
    """A free port on `host` that is not one of the owner's preview ports."""
    for _ in range(200):
        with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as s:
            s.bind((host, 0))
            port = s.getsockname()[1]
        if port not in FORBIDDEN_PORTS:
            return port
    raise RuntimeError('no free port on %s' % host)


def lan_address():
    """The IPv4 address of this computer on the home network (the interface that leads to a private network), or None.
    No packet is sent: connecting a UDP socket only makes the system choose the outgoing interface."""
    found = []
    for target in ('10.255.255.255', '192.168.255.255', '172.31.255.255'):
        try:
            with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as s:
                s.connect((target, 9))
                found.append(s.getsockname()[0])
        except OSError:
            pass
    try:
        found += socket.gethostbyname_ex(socket.gethostname())[2]
    except OSError:
        pass
    for ip in found:
        parts = ip.split('.')
        if len(parts) != 4 or ip.startswith(('127.', '169.254.', '0.')):
            continue
        a, b = int(parts[0]), int(parts[1])
        if a == 10 or (a == 192 and b == 168) or (a == 172 and 16 <= b <= 31):
            return ip
    return None


def port_is_free(host, port):
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as s:
        try:
            s.bind((host, port))
            return True
        except OSError:
            return False


# ---------------------------------------------------------------- the stack

class WalkDocker(retake.DockerBackend):
    """retake.DockerBackend with its own names (xexam-walk-...), no owner-preview port, and in --lan mode the backend's
    Host / Origin guards listing the one LAN address (never XEXAM_ALLOW_ANY_HOST)."""

    def __init__(self, cleanup, version, verbose, guard):
        super().__init__(cleanup, version, verbose)
        token = secrets.token_hex(4)
        self.tag = 'xexam-walk-backend:' + token
        self.ai_tag = 'xexam-walk-ai-service:' + token
        self.name = self.volume = self.network = 'xexam-walk-' + token
        self.link = 'xexam-walk-link-' + token
        self.ai_name = self.ai_volume = 'xexam-walk-ai-service-' + token
        self.stub_name = 'xexam-walk-stub-' + token
        self.port, self.ai_port = safe_port(), safe_port()
        self.guard_env = [x for k, v in guard.items() for x in ('-e', '%s=%s' % (k, v))]

    def _run_published(self, name, attr, inner, args):
        if getattr(self, attr) in FORBIDDEN_PORTS:
            setattr(self, attr, safe_port())
        if name == self.name and self.guard_env:
            args = list(self.guard_env) + list(args)
        super()._run_published(name, attr, inner, args)

    def created(self):
        """Everything this backend creates in Docker, by name (written to stack.json)."""
        return {'images': [self.tag, self.ai_tag], 'containers': [self.name, self.ai_name, self.stub_name],
                'volumes': [self.volume, self.link, self.ai_volume], 'networks': [self.network]}


class WalkCargo(retake.CargoBackend):
    """retake.CargoBackend whose backend gets the same guard variables in --lan mode."""

    def __init__(self, cleanup, version, verbose, work, guard):
        super().__init__(cleanup, version, verbose, work)
        self.port, self.ai_port, self.stub_port = safe_port(), safe_port(), safe_port()
        self.url = 'http://127.0.0.1:%d' % self.port
        self.ai_url = 'http://127.0.0.1:%d' % self.ai_port
        self.stub_base = 'http://127.0.0.1:%d/v1' % self.stub_port
        self.guard_env = dict(guard)

    def _spawn(self):
        env = dict(os.environ, TZ='UTC', XEXAM_DATA=self.data, XEXAM_VERSION=self.version, XEXAM_LINK_DIR=self.link)
        for k in ('XEXAM_ALLOWED_HOSTS', 'XEXAM_ALLOWED_ORIGINS', 'XEXAM_ALLOW_ANY_HOST', 'XEXAM_INTERNAL_TOKEN'):
            env.pop(k, None)
        env.update(self.guard_env)
        log = open(self.logfile, 'a', encoding='utf-8')
        self.cleanup.add('close the backend log', log.close)
        self.proc = popen_group([self.exe, 'serve', '--host', '127.0.0.1', '--port', str(self.port)], stdout=log,
                                stderr=subprocess.STDOUT, env=env, cwd=os.path.dirname(self.data))
        self.cleanup.add('stop the backend', kill_tree, self.proc)
        retake.wait_http(self.url + '/api/info', proc=self.proc)

    def created(self):
        return {'images': [], 'containers': [], 'volumes': [], 'networks': []}


def start_web(work, backend_url, ai_url, cleanup, verbose, host, port, attempts, frontend=None):
    """`vite build` into the work folder, then `vite preview` on host:port with /api proxied to the backend and /api/ai to
    ai-service (after the auth step of vite.config.ts, like Nginx). Returns (base URL, the port)."""
    fe = os.path.abspath(frontend) if frontend else os.path.join(REPO, 'frontend')
    node = shutil.which('node')
    if not node:
        raise RuntimeError('Node.js (node) not found')
    vite = os.path.join(fe, 'node_modules', 'vite', 'bin', 'vite.js')
    if not os.path.isfile(vite):
        retake.npm_ci(fe, vite, verbose)
    dist = os.path.join(work, 'web')
    print('building the frontend (vite build) ...')
    retake.run([node, vite, 'build', '--outDir', dist, '--emptyOutDir', '--logLevel', 'warn'], cwd=fe, verbose=verbose,
               what='vite build')
    for attempt in range(attempts):
        logname = os.path.join(work, 'vite-%d.log' % attempt)
        log = open(logname, 'w', encoding='utf-8')
        cleanup.add('close the vite log', log.close)
        env = dict(os.environ, XEXAM_BACKEND=backend_url, XEXAM_AI_BACKEND=ai_url)
        proc = popen_group([node, vite, 'preview', '--outDir', dist, '--host', host, '--port', str(port), '--strictPort'],
                           cwd=fe, env=env, stdout=log, stderr=subprocess.STDOUT)
        cleanup.add('stop vite preview', kill_tree, proc)
        base = 'http://%s:%d' % (host, port)
        try:
            retake.wait_http(base + '/', 60, ' (vite preview)', proc=proc)
            break
        except RuntimeError:
            log.flush()
            with open(logname, encoding='utf-8', errors='replace') as fh:
                taken = 'already in use' in fh.read()
            if proc.poll() is None or not taken or attempt == attempts - 1:
                raise
            port = safe_port(host)
            print('that port was taken by another program in the meantime; trying %d' % port)
    retake.wait_http(base + '/api/info', 30, ' (the /api proxy to the backend)')
    retake.wait_http(base + '/api/ai/status', 30, ' (the auth step of /api/ai through the backend)', status=401)
    return base, port


class Stack:
    """The running throw-away stack. `base` is the URL the browser (and a phone) uses."""

    def __init__(self, args, chrome, out):
        self.args, self.chrome, self.out = args, chrome, out
        self.cleanup = retake.Cleanup()
        self.base = self.cookies = None
        self.backend = None
        self.lan_ip = None
        self.summary = {}
        self._cookies = {}

    def start(self):
        args = self.args
        version = retake.code_version()
        work = tempfile.mkdtemp(prefix='xexam-walk-')
        self.cleanup.add('remove the temporary folder', remove_dir, work)
        self.work = work
        host, port, guard = '127.0.0.1', None, {}
        if args.lan:
            self.lan_ip = lan_address()
            if not self.lan_ip:
                raise RuntimeError('--lan: no home-network address (10.x, 172.16-31.x or 192.168.x) found on this computer')
            host = self.lan_ip
            port = safe_port(host)
            guard = {'XEXAM_ALLOWED_HOSTS': self.lan_ip, 'XEXAM_ALLOWED_ORIGINS': 'http://%s:%d' % (self.lan_ip, port)}
        banks = demo_banks.build(os.path.join(work, 'banks'))
        print('Xexam v%s phone walk stack (made-up data only)' % version)
        if args.backend == 'docker':
            self.backend = WalkDocker(self.cleanup, version, args.verbose, guard)
        else:
            self.backend = WalkCargo(self.cleanup, version, args.verbose, work, guard)
        self.write_state({'stage': 'starting'})
        self.backend.start()
        attempts = 1 if args.lan else 6                  # LAN: the Origin list names the port, so no silent change
        self.base, port = start_web(work, self.backend.url, self.backend.ai_url, self.cleanup, args.verbose, host,
                                    port or safe_port(), attempts, args.frontend)
        self.port = port
        self.check_ports()
        print('seeding the demo data ...')
        seed.prepare(self.base, banks, self.backend.stub_base)
        self.backend.restart()                           # the System log starts empty again
        retake.wait_http(self.base + '/api/info', 30)
        cookie = seed.activity(self.base)                # ends when ai-service has reported to the restarted backend
        seed.seed_exams(self.base, cookie)
        self.summary = seed_extra.extra(self.base, cookie, os.path.join(work, 'banks'))
        self._cookies['admin'] = cookie
        self.write_state({'stage': 'ready'})

    def check_ports(self):
        ports = {self.port, self.backend.port, self.backend.ai_port}
        bad = ports & FORBIDDEN_PORTS
        if bad:
            raise RuntimeError('the stack got an owner-preview port (%s); stopping so nothing of the owner is touched'
                               % ', '.join(map(str, sorted(bad))))

    def cookie(self, user):
        """The xexam_session value of a made-up user (signed in through the API on first use)."""
        if user not in self._cookies:
            if user in CANNOT_SIGN_IN:
                raise ValueError('%s cannot sign in' % user)
            self._cookies[user] = seed.Session(self.base).sign_in(user, seed.DEMO_PASSWORD).cookie[seed.COOKIE]
        return self._cookies[user]

    def session(self, user):
        """A seed.Session (API client) signed in as `user`."""
        s = seed.Session(self.base)
        s.cookie[seed.COOKIE] = self.cookie(user)
        return s

    def write_state(self, extra=None):
        """stack.json in the output folder: the URL, the made-up accounts and every Docker name the stack created, so a
        stack that was killed hard can be cleaned up by name."""
        state = {'base': self.base, 'lan': self.lan_ip, 'accounts': {u: seed.DEMO_PASSWORD for u, _ in ACCOUNTS
                                                                      if u not in CANNOT_SIGN_IN},
                 'docker': self.backend.created() if self.backend else {}}
        state.update(extra or {})
        os.makedirs(self.out, exist_ok=True)
        with open(os.path.join(self.out, 'stack.json'), 'w', encoding='utf-8', newline='\n') as fh:
            json.dump(state, fh, indent=1)
            fh.write('\n')

    def stop(self):
        print('cleaning up ...')
        self.cleanup.run()


# ---------------------------------------------------------------- what checks.py gets

class Walk:
    """The context handed to checks.run(walk): the stack, the options, the output folder, a device factory."""

    def __init__(self, args, stack, chrome, out):
        self.args, self.stack, self.chrome, self.out = args, stack, chrome, out
        self.base = stack.base
        self.started = time.time()

    def device(self, user, lang='zh', theme='light', width=390, height=844, **kw):
        """A new phone-like Chrome signed in as `user` (a made-up user of the stack); close it (or use `with`)."""
        import device
        return device.Device(self.chrome, self.base, self.stack.cookie(user) if user else None, lang=lang, theme=theme,
                             width=width, height=height, **kw)

    def wants_page(self, page):
        return not self.args.pages or page in self.args.pages

    def wants_check(self, check_id):
        return not self.args.only or any(fnmatch.fnmatchcase(check_id, pat) for pat in self.args.only)


def load_checks():
    """checks.py (T1b), or None while it does not exist yet."""
    if not os.path.isfile(os.path.join(HERE, 'checks.py')):
        return None
    return importlib.import_module('checks')


# ---------------------------------------------------------------- printouts

def print_keep(stack, out):
    print()
    print('=' * 72)
    print('The phone walk stack is running (made-up data and the AI stub only).')
    if stack.lan_ip:
        print('  URL on your phone (same network): %s/' % stack.base)
    else:
        print('  URL: %s/' % stack.base)
    print('  Made-up accounts (password for all: %s):' % seed.DEMO_PASSWORD)
    for user, what in ACCOUNTS:
        print('    %-8s %s' % (user, what))
    print('  Extra data: bank "%s" (51 questions), %d study records for lee.' % (seed_extra.BANK_NAME,
                                                                                  stack.summary.get('leeRecords', 0)))
    if stack.lan_ip:
        print('  Windows may show a firewall prompt the first time: allow it for the PRIVATE network only.')
    print('  Stop: press Ctrl+C here (or create the file %s). That removes everything the stack created.'
          % os.path.join(out, STOP_FILE))
    print('=' * 72, flush=True)


def print_lan_warning():
    print('WARNING: --lan makes the stack reachable from every device on this network (made-up data and the AI stub only;')
    print('nothing real is loaded into it). Stop it with Ctrl+C when you are done.')
    print(flush=True)


# ---------------------------------------------------------------- main

def main(argv=None):
    args = parse_args(argv)
    chrome = None
    if not args.keep:
        chrome = find_chrome(args.chrome)
        if not chrome:
            raise SystemExit('no Chrome / Chromium found: pass --chrome PATH or set XEXAM_CHROME')
    elif args.chrome:
        chrome = find_chrome(args.chrome)
    out = os.path.abspath(args.out) if args.out else tempfile.mkdtemp(prefix='xexam-walk-out-')
    os.makedirs(out, exist_ok=True)

    def interrupted(signum, frame):
        raise KeyboardInterrupt
    for name in ('SIGTERM', 'SIGBREAK', 'SIGHUP'):
        if hasattr(signal, name):
            signal.signal(getattr(signal, name), interrupted)

    if args.lan:
        print_lan_warning()
        # Reachable from the network: never the public fixed passwords of the repo, a random pair per run
        seed.DEMO_PASSWORD = secrets.token_urlsafe(12)
        seed.INITIAL = secrets.token_urlsafe(12)
    stack = Stack(args, chrome, out)
    code = 0
    started = time.time()
    try:
        stack.start()
        if args.keep:
            print_keep(stack, out)
            stop = os.path.join(out, STOP_FILE)
            while not os.path.exists(stop):
                time.sleep(1)
            print('stop file found')
        else:
            checks = load_checks()
            if checks is None:
                print('no checks.py yet (T1b): the stack came up and was seeded, nothing to walk')
            else:
                code = int(checks.run(Walk(args, stack, chrome, out)) or 0)
    except KeyboardInterrupt:
        print('interrupted')
        code = 130
    except Exception:
        traceback.print_exc()
        code = 1
    finally:
        stack.stop()
    print('%s in %.0f s; output folder %s' % ('done' if code == 0 else 'stopped', time.time() - started, out))
    return code


if __name__ == '__main__':
    sys.exit(main())
