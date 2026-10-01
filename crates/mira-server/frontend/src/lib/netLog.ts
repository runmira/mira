/**
 * Ring buffer of the web app's own `fetch` traffic, for the developer-tools
 * pane.
 *
 * Scope, stated plainly so the pane doesn't overclaim: this records
 * requests the *frontend* makes — the `/api/*` calls the UI issues. It does
 * not see the model provider's HTTP traffic, because that originates in the
 * Rust server, not the browser. Token and cost figures come from
 * `/api/usage` instead.
 *
 * Instrumentation wraps `window.fetch` rather than patching XHR or hooking
 * the network layer, so anything using `fetch` (including code that
 * captured a reference before install) is still observed.
 */

export type NetEntry = {
  id: number;
  /** Wall-clock ms since epoch when the request started. */
  startedAt: number;
  method: string;
  url: string;
  /** Path only, for grouping and filtering. */
  path: string;
  status: number | null;
  ok: boolean;
  /** Round-trip duration in ms; null while in flight. */
  durationMs: number | null;
  /** Bytes of the response body, when we were able to read them. */
  bytes: number | null;
  error: string | null;
  /** Our own marker so the pane can exclude its polling from the view. */
  kind: 'api' | 'asset' | 'other';
};

/** Bounded so a long session can't grow this without limit. */
const MAX_ENTRIES = 400;

let entries: NetEntry[] = [];
let nextId = 1;
let installed = false;

const listeners = new Set<() => void>();

function emit() {
  for (const fn of listeners) fn();
}

function classify(url: string): NetEntry['kind'] {
  if (url.includes('/api/') || url.includes('/ws')) return 'api';
  if (/\.(js|mjs|css|woff2?|png|jpe?g|svg|gif|webp|ico|json)$/i.test(url)) return 'asset';
  return 'other';
}

function toPath(url: string): string {
  try {
    const u = new URL(url, window.location.origin);
    return u.pathname + u.search;
  } catch {
    return url;
  }
}

/**
 * Write fields onto an already-logged entry.
 *
 * Both the array and the entry object have to be replaced: consumers read
 * this through `useSyncExternalStore`, which compares the snapshot by
 * reference and bails out of re-rendering when it's unchanged. Mutating the
 * entry in place would leave the log visibly stuck on "…".
 */
function patch(id: number, fields: Partial<NetEntry>): void {
  const idx = entries.findIndex((e) => e.id === id);
  if (idx === -1) return;
  const next = entries.slice();
  next[idx] = { ...next[idx], ...fields };
  entries = next;
}

export function subscribe(fn: () => void): () => void {
  listeners.add(fn);
  return () => {
    listeners.delete(fn);
  };
}

/** Current snapshot, newest last. Stable reference between changes. */
export function getEntries(): NetEntry[] {
  return entries;
}

export function clear(): void {
  entries = [];
  emit();
}

export function remove(id: number): void {
  const next = entries.filter((e) => e.id !== id);
  if (next.length !== entries.length) {
    entries = next;
    emit();
  }
}

/**
 * Replace `window.fetch` with a logging wrapper. Idempotent — safe to call
 * from a hot path or in StrictMode's double-invoke.
 */
export function install(): void {
  if (installed) return;
  if (typeof window === 'undefined' || typeof window.fetch !== 'function') return;
  installed = true;

  const original = window.fetch.bind(window);

  window.fetch = async function patchedFetch(
    input: RequestInfo | URL,
    init?: RequestInit,
  ): Promise<Response> {
    const method = (
      init?.method ??
      (input instanceof Request ? input.method : 'GET')
    ).toUpperCase();
    const href =
      typeof input === 'string'
        ? input
        : input instanceof URL
          ? input.href
          : input.url;

    const entry: NetEntry = {
      id: nextId++,
      startedAt: Date.now(),
      method,
      url: href,
      path: toPath(href),
      status: null,
      ok: false,
      durationMs: null,
      bytes: null,
      error: null,
      kind: classify(href),
    };

    entries = [...entries, entry];
    if (entries.length > MAX_ENTRIES) {
      entries = entries.slice(entries.length - MAX_ENTRIES);
    }
    emit();

    const started = performance.now();
    try {
      const res = await original(input as RequestInfo, init);
      const len = res.headers.get('content-length');
      patch(entry.id, {
        status: res.status,
        ok: res.ok,
        durationMs: Math.round(performance.now() - started),
        bytes: len ? Number(len) : null,
      });
      emit();
      return res;
    } catch (err) {
      patch(entry.id, {
        durationMs: Math.round(performance.now() - started),
        error: err instanceof Error ? err.message : String(err),
      });
      emit();
      throw err;
    }
  };
}

/** CSV of the log, for "export" in the pane. */
export function toCsv(): string {
  const head = [
    'started_at_iso',
    'method',
    'path',
    'status',
    'ok',
    'duration_ms',
    'bytes',
    'error',
  ];
  const rows = entries.map((e) =>
    [
      new Date(e.startedAt).toISOString(),
      e.method,
      e.path,
      e.status ?? '',
      String(e.ok),
      e.durationMs ?? '',
      e.bytes ?? '',
      e.error ?? '',
    ]
      // Quote everything: paths carry query strings with commas.
      .map((v) => `"${String(v).replace(/"/g, '""')}"`)
      .join(','),
  );
  return [head.join(','), ...rows].join('\n');
}

export function downloadCsv(): void {
  const blob = new Blob([toCsv()], { type: 'text/csv;charset=utf-8' });
  const url = URL.createObjectURL(blob);
  const a = document.createElement('a');
  a.href = url;
  a.download = `mira-netlog-${new Date().toISOString().replace(/[:.]/g, '-')}.csv`;
  a.click();
  URL.revokeObjectURL(url);
}
