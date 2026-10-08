import { useEffect, useState } from 'react';
import hljs from 'highlight.js/lib/core';
import type { LanguageFn } from 'highlight.js';
import { common } from 'lowlight';
import rehypeHighlight from 'rehype-highlight';

/**
 * Syntax highlighting with a small up-front language set (issue #72).
 *
 * The full `highlight.js` entry registers all ~190 grammars and cost
 * ~1.5 MB of source on the main chunk. Instead, the ~37 "common" languages
 * (the same set rehype-highlight uses by default) are registered eagerly and
 * everything else is fetched as its own small chunk the first time a file
 * or code fence asks for it.
 *
 * Both highlighters read from here: the `hljs` instance below (file viewer)
 * and rehype-highlight in Markdown, via `markdownLanguages()`. This module
 * is itself loaded lazily — Markdown only fetches it for messages with a
 * fenced code block.
 */

for (const [name, fn] of Object.entries(common)) hljs.registerLanguage(name, fn);

export { hljs, rehypeHighlight };

// One lazy chunk per grammar. Common languages are skipped by the
// `isRegistered` check before a loader is ever looked up.
const LOADERS = import.meta.glob<LanguageFn>(
  [
    '/node_modules/highlight.js/es/languages/*.js',
    // `*.js.js` are CommonJS-interop shims of the same grammars.
    '!/node_modules/highlight.js/es/languages/*.js.js',
    // Already in the main chunk via lowlight's `common` (keep in sync).
    '!/node_modules/highlight.js/es/languages/{arduino,bash,c,cpp,csharp,css,diff,go,graphql,ini,java,javascript,json,kotlin,less,lua,makefile,markdown,objectivec,perl,php,php-template,plaintext,python,python-repl,r,ruby,rust,scss,shell,sql,swift,typescript,vbnet,wasm,xml,yaml}.js',
  ],
  { import: 'default' },
);

const loaderByName = new Map<string, () => Promise<LanguageFn>>();
for (const [path, load] of Object.entries(LOADERS)) {
  const name = /\/([\w-]+)\.js$/.exec(path)?.[1];
  if (name) loaderByName.set(name, load);
}

/** Aliases for lazily loaded grammars. A grammar's own aliases only become
 *  known once it is registered, so names people write in fences and file
 *  extensions need mapping to the grammar file up front. */
const LAZY_ALIASES: Record<string, string> = {
  hs: 'haskell',
  ex: 'elixir',
  exs: 'elixir',
  ps: 'powershell',
  ps1: 'powershell',
  pwsh: 'powershell',
  bat: 'dos',
  cmd: 'dos',
  docker: 'dockerfile',
  clj: 'clojure',
  ml: 'ocaml',
  jl: 'julia',
  erl: 'erlang',
  fs: 'fsharp',
  pl: 'perl',
  proto: 'protobuf',
  tex: 'latex',
  vb: 'vbnet',
  nginxconf: 'nginx',
  coffee: 'coffeescript',
  hbs: 'handlebars',
};

const extra: Record<string, LanguageFn> = {};
const inflight = new Map<string, Promise<boolean>>();
const listeners = new Set<() => void>();
let version = 0;

export function isRegistered(name: string): boolean {
  return !!hljs.getLanguage(name);
}

/**
 * Register the grammar for `name` (a language name or alias) if it isn't
 * already. Resolves `true` once it's usable, `false` if highlight.js has no
 * such language.
 */
export function loadLanguage(name: string): Promise<boolean> {
  const key = name.toLowerCase();
  if (!key || key === 'plaintext' || isRegistered(key)) return Promise.resolve(isRegistered(key));
  const file = LAZY_ALIASES[key] ?? key;
  const load = loaderByName.get(file);
  if (!load) return Promise.resolve(false);
  let p = inflight.get(file);
  if (!p) {
    p = load().then(
      (fn) => {
        hljs.registerLanguage(file, fn);
        if (file !== key) hljs.registerAliases(key, { languageName: file });
        extra[file] = fn;
        version++;
        for (const l of listeners) l();
        return true;
      },
      () => {
        inflight.delete(file);
        return false;
      },
    );
    inflight.set(file, p);
  }
  return p;
}

/** Languages to hand rehype-highlight: the common set plus whatever has
 *  been loaded on demand so far. */
export function markdownLanguages(): Record<string, LanguageFn> {
  return { ...common, ...extra };
}

/**
 * Load any of `names` that aren't registered yet. Returns a counter that
 * bumps whenever a new grammar lands, for use as a memo dependency so the
 * caller re-highlights.
 */
export function useHighlightLanguages(names: readonly string[]): number {
  const [v, setV] = useState(version);
  const key = names.join('\0');

  useEffect(() => {
    const sync = () => setV(version);
    listeners.add(sync);
    for (const n of names) if (!isRegistered(n)) void loadLanguage(n);
    // A grammar may have landed between render and effect.
    sync();
    return () => {
      listeners.delete(sync);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [key]);

  return v;
}
