// TEMPORARY: theme-preview.html against the server on 8787. Not shipped.
import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';
import path from 'node:path';
export default defineConfig({
  plugins: [react()],
  resolve: { alias: { '@': path.resolve(__dirname, './src') } },
  server: { port: 5174, proxy: { '/api': { target: 'http://127.0.0.1:8787', changeOrigin: true }, '/ws': { target: 'ws://127.0.0.1:8787', ws: true } } },
  worker: { format: 'es' },
});
