import { readFileSync } from 'node:fs';
import type { IncomingMessage, ServerResponse } from 'node:http';
import { fileURLToPath } from 'node:url';
import { defineConfig, type Plugin } from 'vite';
import { parseChangelog } from './src/domain/changelog.ts';

// In development, proxy /api to the backend. changeOrigin must be false: the backend compares Host with Origin,
// and rewriting Host makes write requests fail with 403 (verified in the audit). Open the dev page via localhost or 127.0.0.1.
const backend = process.env.XEXAM_BACKEND ?? 'http://127.0.0.1:8765';
// v0.8: /api/ai/ goes to the AI service (ai-service), after the xexamAiAuth() plugin below has checked the session
const aiBackend = process.env.XEXAM_AI_BACKEND ?? 'http://127.0.0.1:8766';
// One prefix for the plugin and the proxy entry (like Nginx's `location /api/ai/`): every request the AI proxy forwards has
// passed the plugin, so the AI service never sees a cookie or a client-sent identity. /api/ai without the slash (and
// /api/aiX) goes to the backend like any other /api path.
const AI_PREFIX = '/api/ai/';

/**
 * `xexamAiAuth()` (v0.8): the dev and preview servers do what Nginx's auth_request does in production, so the AI service
 * never sees a session cookie here either. For a request to /api/ai/*: a client-sent X-Xexam-Identity is dropped; the
 * backend's GET /internal/ai/auth checks the cookie and the Host / Origin guard (X-Original-Host / -Origin / -Method);
 * on 204 its signed X-Xexam-Identity is set and Cookie, Authorization and Cf-Access-Jwt-Assertion are removed before the
 * proxy forwards the request; on 401 / 403 the backend's answer is returned as it is; anything else is a 503 JSON
 * `ai.backendDown`. The middleware is added directly (not in a returned hook), so it runs before the proxy.
 */
function xexamAiAuth(): Plugin {
  const down = (res: ServerResponse): void => {
    res.statusCode = 503;
    res.setHeader('Content-Type', 'application/json; charset=utf-8');
    res.setHeader('Cache-Control', 'no-store');
    res.end(JSON.stringify({ error: '主服务暂时连不上，请稍后再试。', code: 'ai.backendDown' }));
  };
  const check = (req: IncomingMessage, res: ServerResponse, next: (err?: unknown) => void): void => {
    if (!(req.url ?? '').startsWith(AI_PREFIX)) {
      next();
      return;
    }
    delete req.headers['x-xexam-identity'];
    const headers: Record<string, string> = {
      'X-Original-Host': req.headers.host ?? '',
      'X-Original-Method': (req.method ?? 'GET').toUpperCase(),
    };
    if (typeof req.headers.cookie === 'string') headers.Cookie = req.headers.cookie;
    if (typeof req.headers.origin === 'string') headers['X-Original-Origin'] = req.headers.origin;
    fetch(`${backend}/internal/ai/auth`, { headers, redirect: 'manual', signal: AbortSignal.timeout(5000) }).then(async (r) => {
      const identity = r.headers.get('x-xexam-identity');
      if (r.status === 204 && identity) {
        req.headers['x-xexam-identity'] = identity;
        delete req.headers.cookie;
        delete req.headers.authorization;
        delete req.headers['cf-access-jwt-assertion'];
        next();
        return;
      }
      if (r.status === 401 || r.status === 403) {
        const body = await r.text();
        res.statusCode = r.status;
        res.setHeader('Content-Type', r.headers.get('content-type') ?? 'application/json; charset=utf-8');
        res.setHeader('Cache-Control', 'no-store');
        res.end(body);
        return;
      }
      down(res);
    }).catch(() => down(res));
  };
  return {
    name: 'xexam-ai-service-auth',
    configureServer(server) {
      server.middlewares.use(check);
    },
    configurePreviewServer(server) {
      server.middlewares.use(check);
    },
  };
}

/**
 * `virtual:changelog`: the changelog entries of the three READMEs (English, Chinese, Japanese), parsed at build time for the
 * About page. The READMEs stay the single source of truth (CI checks their newest entry against CODE_VERSION in VERSION).
 * Paths are relative to the repository root: README.md, README/README.ja.md, README/README.zh-CN.md. The nginx image build
 * copies them to the same places above the frontend sources (see nginx/Dockerfile).
 */
function changelog(): Plugin {
  const id = 'virtual:changelog';
  const resolved = '\0' + id;
  const files = { en: 'README.md', zh: 'README/README.zh-CN.md', ja: 'README/README.ja.md' } as const;
  const path = (f: string): string => fileURLToPath(new URL(`../${f}`, import.meta.url));
  return {
    name: 'xexam-changelog',
    resolveId: (source) => (source === id ? resolved : null),
    load(loadId) {
      if (loadId !== resolved) return null;
      const data: Record<string, unknown> = {};
      for (const [lang, file] of Object.entries(files)) {
        this.addWatchFile(path(file));
        const entries = parseChangelog(readFileSync(path(file), 'utf8'));
        if (!entries.length) this.error(`${file} has no changelog entries ("### vX.Y — date")`);
        data[lang] = entries;
      }
      return `export default ${JSON.stringify(data)};`;
    },
  };
}

export default defineConfig({
  plugins: [changelog(), xexamAiAuth()],
  build: {
    target: ['es2020', 'safari15'],
    sourcemap: false,
    assetsInlineLimit: 0,
  },
  server: {
    host: '127.0.0.1',
    port: 5173,
    // '/api/ai/' before '/api' (the first matching entry wins); `vite preview` uses these proxies too
    proxy: {
      [AI_PREFIX]: { target: aiBackend, changeOrigin: false },
      '/api': { target: backend, changeOrigin: false },
    },
  },
});
