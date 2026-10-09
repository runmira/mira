import { build } from 'esbuild';
import path from 'node:path';
import { mkdtemp, writeFile, rm } from 'node:fs/promises';
import { pathToFileURL } from 'node:url';
import { fileURLToPath } from 'node:url';
const root = fileURLToPath(new URL('../../', import.meta.url));
/** Bundle real TypeScript modules for Node tests, replacing only explicit boundaries. */
export async function bundle(entry, mocks = {}) {
  const result = await build({
    stdin: { contents: entry, resolveDir: root, sourcefile: 'test-entry.tsx', loader: 'tsx' },
    jsx: 'automatic',
    bundle: true,
    write: false,
    format: 'esm',
    platform: 'node',
    target: 'esnext',
    packages: 'external',
    alias: { '@': path.join(root, 'src') },
    loader: { '.svg': 'dataurl', '.png': 'dataurl', '.css': 'empty' },
    define: { 'import.meta.env': '{}' },
    plugins: [
      {
        name: 'test-boundaries',
        setup(api) {
          api.onResolve({ filter: /\.css$/ }, (args) => ({
            path: args.path,
            namespace: 'empty-css',
          }));
          api.onLoad({ filter: /.*/, namespace: 'empty-css' }, () => ({
            contents: '',
            loader: 'js',
          }));
          api.onResolve({ filter: /.*/ }, (args) => {
            const full = path.resolve(args.resolveDir, args.path).replace(/\.(tsx?|mjs)$/, '');
            const key = Object.keys(mocks).find(
              (key) => full === path.join(root, key).replace(/\.(tsx?|mjs)$/, ''),
            );
            if (key) return { path: key, namespace: 'mock' };
          });
          api.onLoad({ filter: /.*/, namespace: 'mock' }, (args) => ({
            contents: mocks[args.path],
            loader: 'tsx',
          }));
        },
      },
    ],
  });
  const directory = await mkdtemp(path.join(root, 'node_modules/.mira-tests-'));
  try {
    const file = path.join(directory, 'bundle.mjs');
    await writeFile(file, result.outputFiles[0].text);
    return await import(pathToFileURL(file));
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
}
