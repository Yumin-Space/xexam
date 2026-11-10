import { defineConfig } from 'vite';

// In development, proxy /api to the backend. changeOrigin must be false: the backend compares Host with Origin,
// and rewriting Host makes write requests fail with 403 (verified in the audit). Open the dev page via localhost or 127.0.0.1.
const backend = process.env.XEXAM_BACKEND ?? 'http://127.0.0.1:8765';

export default defineConfig({
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
