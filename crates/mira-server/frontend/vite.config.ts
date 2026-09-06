import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';

// In dev, Vite serves the frontend and proxies /ws + /api to the Rust
// server (default port 8787). In production, the server serves the built
// assets directly via --static-dir dist/.
export default defineConfig({
  plugins: [react()],
  server: {
    port: 5173,
    proxy: {
      '/ws': { target: 'ws://127.0.0.1:8787', ws: true, changeOrigin: true },
      '/api': { target: 'http://127.0.0.1:8787', changeOrigin: true },
    },
  },
  build: { outDir: 'dist', sourcemap: true },
});
