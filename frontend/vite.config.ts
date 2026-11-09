import { defineConfig } from 'vite';

// 开发时把 /api 转给后端。changeOrigin 必须是 false：后端会比较 Host 与 Origin，
// 改写 Host 会让写请求被 403（审计里实测过）。请用 localhost 或 127.0.0.1 打开开发页。
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
