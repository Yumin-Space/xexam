import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { defineConfig, type Plugin } from 'vite';
import { parseChangelog } from './src/domain/changelog.ts';

// In development, proxy /api to the backend. changeOrigin must be false: the backend compares Host with Origin,
// and rewriting Host makes write requests fail with 403 (verified in the audit). Open the dev page via localhost or 127.0.0.1.
const backend = process.env.XEXAM_BACKEND ?? 'http://127.0.0.1:8765';

/**
 * `virtual:changelog`: the changelog entries of the three READMEs (English, Chinese, Japanese), parsed at build time for the
 * About page. The READMEs stay the single source of truth (CI checks their newest entry against VERSION). The nginx image
 * build copies them next to the frontend sources for this (see nginx/Dockerfile).
 */
function changelog(): Plugin {
  const id = 'virtual:changelog';
  const resolved = '\0' + id;
  const files = { en: 'README.md', zh: 'README.zh-CN.md', ja: 'README.ja.md' } as const;
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
  plugins: [changelog()],
  build: {
    target: ['es2020', 'safari15'],
    sourcemap: false,
    assetsInlineLimit: 0,
  },
  server: {
    host: '127.0.0.1',
    port: 5173,
    proxy: { '/api': { target: backend, changeOrigin: false } },
  },
});
