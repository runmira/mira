import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';
import path from 'node:path';
import { visualizer } from 'rollup-plugin-visualizer';

// `REACT_COMPILER_REPORT=1 npm run build` lists components the compiler
// skipped and why.
function compilerLogger() {
  if (!process.env.REACT_COMPILER_REPORT) return {};
  return {
    logger: {
      logEvent(filename: string | null, event: { kind: string; fnName?: string | null; detail?: unknown }) {
        if (event.kind === 'CompileSuccess') console.log(`[compiler] ok ${filename}`);
        else if (event.kind === 'CompileError' || event.kind === 'CompileSkip' || event.kind === 'PipelineError')
          console.log(`[compiler] ${event.kind} ${filename} ${event.fnName ?? ''} ${String((event.detail as any)?.reason ?? (event.detail as any)?.options?.reason ?? '').slice(0, 120)}`);
      },
    },
  };
}

const VENDOR_CHUNKS: [string, RegExp][] = [
  ['vendor-react', /^(react|react-dom|scheduler)$/],
  ['vendor-motion', /^(framer-motion|motion-dom|motion-utils)$/],
  ['vendor-markdown', /^(react-markdown|remark-.*|rehype-.*|micromark.*|mdast-.*|hast-.*|unist-.*|unified|vfile.*|property-information|entities|decode-named-character-reference|character-entities.*|space-separated-tokens|comma-separated-tokens|html-url-attributes|devlop|bail|trough|is-plain-obj|ccount|escape-string-regexp|markdown-table|longest-streak|zwitch|trim-lines|style-to-.*|inline-style-parser|estree-util-.*)$/],
];

type ModuleInfoLookup = (id: string) => { isEntry: boolean; importers: readonly string[] } | null;
const initialCache = new Map<string, boolean>();

/** Whether `id` is reachable from an entry through static imports alone. */
function isInitial(id: string, getModuleInfo: ModuleInfoLookup, seen = new Set<string>()): boolean {
  const cached = initialCache.get(id);
  if (cached !== undefined) return cached;
  if (seen.has(id)) return false;
  seen.add(id);
  const info = getModuleInfo(id);
  const result = !!info && (info.isEntry || info.importers.some((i) => isInitial(i, getModuleInfo, seen)));
  // Only cache positives: a negative may just be a cycle cut short by `seen`.
  if (result) initialCache.set(id, true);
  return result;
}

// In dev, Vite serves the frontend and proxies /ws + /api to the Rust
// server (default port 8787). In production, the server serves the built
// assets directly via --static-dir dist/.
export default defineConfig({
  plugins: [
    react({
      babel: {
        // Auto-memoizes components and hooks, so render work doesn't depend
        // on hand-placed useMemo/useCallback. Components that break the
        // Rules of React are skipped (left as-is), not miscompiled.
        plugins: [['babel-plugin-react-compiler', { target: '19', ...compilerLogger() }]],
      },
    }),
    // `ANALYZE=1 npm run build` writes dist/stats.html (treemap of every chunk).
    process.env.ANALYZE && visualizer({ filename: 'dist/stats.html', gzipSize: true, template: 'treemap' }),
  ],
  resolve: {
    alias: {
      '@': path.resolve(__dirname, './src'),
    },
  },
  server: {
    port: 5173,
    proxy: {
      '/ws': { target: 'ws://127.0.0.1:8787', ws: true, changeOrigin: true },
      '/api': { target: 'http://127.0.0.1:8787', changeOrigin: true },
    },
  },
  build: {
    outDir: 'dist',
    sourcemap: true,
    // Read by scripts/check-bundle-size.mjs to find the first-load chunks.
    manifest: true,
    rollupOptions: {
      output: {
        // First-load dependencies get their own long-lived chunks (they
        // change less often than app code, and no single file trips the
        // size warning). Only modules the entry pulls in statically are
        // grouped, so a vendor chunk can never drag a lazy-only dependency
        // (parse5, mermaid, Shiki, …) into the first load.
        manualChunks(id, { getModuleInfo }) {
          const pkg = /node_modules\/((?:@[^/]+\/)?[^/]+)/.exec(id)?.[1];
          if (!pkg || !isInitial(id, getModuleInfo)) return;
          for (const [chunk, test] of VENDOR_CHUNKS) if (test.test(pkg)) return chunk;
          return 'vendor';
        },
      },
    },
    // The chunks still over Vite's 500 kB default are single third-party
    // modules that only load on demand and can't be split further: the
    // file-icon set (~4 MB of inline SVG), mermaid, Shiki's wasm and its
    // largest grammars. First-load size is gated by `npm run size` instead.
    chunkSizeWarningLimit: 4200,
  },
  // Pierre's diff worker (`@pierre/diffs/worker/worker.js?worker`) code-splits
  // (wasm loader chunk); workers must build as ES modules, not IIFE.
  worker: { format: 'es' },
});
