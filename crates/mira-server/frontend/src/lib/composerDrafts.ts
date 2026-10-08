/**
 * Per-session composer drafts.
 *
 * The composer is one contenteditable surface that outlives a single
 * session: you type a long prompt, click another chat in the sidebar,
 * and come back — and the text is gone. That is the bug this fixes.
 *
 * Design notes, since the naive version is wrong in three ways:
 *
 *  - **One blob, not one key per session.** `localStorage` writes are
 *    synchronous and hit disk; a key per session would mean unbounded
 *    key sprawl plus a `storage`-event storm. One versioned record,
 *    pruned on write.
 *  - **Pruned on every write.** Drafts are only interesting while the
 *    conversation is fresh, so entries expire (14 days) and the map is
 *    capped at the 40 most recently touched sessions. Otherwise this
 *    quietly becomes an unbounded transcript archive in localStorage.
 *  - **Never throws.** Private mode, a full quota, a corrupted blob —
 *    a draft is a convenience, so every failure degrades to "no draft"
 *    instead of taking the composer down with it.
 *
 * Persisting (not just remembering in memory) is deliberate: the point
 * is surviving a session switch, and a reload with the app closed.
 */

/** Bumped when the stored shape changes; an older version is dropped. */
const STORAGE_KEY = 'mira.composer.drafts.v1';

/** A draft older than this is dropped on the next write. */
const MAX_AGE_MS = 14 * 24 * 60 * 60 * 1000;
/** Most-recently-written drafts kept. Past this the oldest go. */
const MAX_ENTRIES = 40;
/** A single draft longer than this is not worth storing (a pasted file). */
const MAX_TEXT = 64 * 1024;
/** Debounce for keystroke writes: one localStorage hit per pause. */
const SAVE_DEBOUNCE_MS = 400;

export type ComposerDrafts = Record<string, { text: string; at: number }>;

function read(): ComposerDrafts {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (!raw) return {};
    const parsed = JSON.parse(raw) as ComposerDrafts;
    if (!parsed || typeof parsed !== 'object') return {};
    // Shape-check one entry: a hand-edited or half-migrated blob must
    // not make `text` undefined downstream.
    return Object.fromEntries(
      Object.entries(parsed).filter(
        (e): e is [string, { text: string; at: number }] =>
          !!e[1] && typeof e[1].text === 'string' && typeof e[1].at === 'number',
      ),
    );
  } catch {
    return {};
  }
}

function write(entries: ComposerDrafts): void {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(entries));
  } catch {
    /* private mode, quota — the draft is a nicety, not a promise */
  }
}

/** Drop expired entries, then the least recently written beyond the cap. */
function pruned(entries: ComposerDrafts, now = Date.now()): ComposerDrafts {
  const live = Object.entries(entries)
    .filter(([, v]) => now - v.at < MAX_AGE_MS && v.text.trim() !== '')
    .sort((a, b) => b[1].at - a[1].at)
    .slice(0, MAX_ENTRIES);
  return Object.fromEntries(live) as ComposerDrafts;
}

/**
 * The draft key for a session. A chat that has no id yet still needs
 * somewhere to put its text, so unsaved chats share one bucket — the
 * draft is sent (and therefore cleared) on submit either way.
 */
export function draftKey(sessionId: string | null | undefined): string {
  return sessionId || '__unsaved__';
}

/** The stored draft for a session, or `''` when there is none. */
export function readDraft(key: string): string {
  const entry = read()[key];
  return entry && Date.now() - entry.at < MAX_AGE_MS ? entry.text : ''; 
}

/** Store (or, for blank text, drop) a session's draft. */
export function writeDraft(key: string, text: string): void {
  try {
    const entries = read();
    if (text.trim() === '') {
      if (!(key in entries)) return; // nothing to do, and nothing to prune
      delete entries[key];
    } else {
      // A pasted file is not a draft worth keeping; the tail is more
      // useful than a quota error on the next write.
      const capped = text.length > MAX_TEXT ? text.slice(0, MAX_TEXT) : text;
      entries[key] = { text: capped, at: Date.now() };
    }
    write(pruned(entries));
  } catch {
    /* never let bookkeeping break typing */
  }
}

/** Forget one session's draft — called when its prompt is sent. */
export function clearDraft(key: string): void {
  const timer = pending.get(key); if (timer) clearTimeout(timer);
  pending.delete(key); queued.delete(key);
  writeDraft(key, '');
}

/* ------------------------------------------------------------------ */
/* Debounced writes                                                     */
/* ------------------------------------------------------------------ */

const pending = new Map<string, ReturnType<typeof setTimeout>>();
const queued = new Map<string, string>();

function flush(key: string) {
  const text = queued.get(key) ?? '';
  queued.delete(key);
  writeDraft(key, text);
}

/**
 * Queue a write for `key`, coalescing bursts. Kept per key so switching
 * sessions mid-pause flushes the right one and leaves no stray timer
 * writing an empty draft over real text.
 */
export function queueDraft(key: string, text: string): void {
  queued.set(key, text);
  const existing = pending.get(key);
  if (existing) clearTimeout(existing);
  pending.set(
    key,
    setTimeout(() => {
      pending.delete(key);
      flush(key);
    }, SAVE_DEBOUNCE_MS),
  );
}

/** Write every queued draft now. For `pagehide`/`visibilitychange`. */
export function flushDrafts(): void {
  for (const key of [...pending.keys()]) {
    const t = pending.get(key);
    if (t) clearTimeout(t);
    pending.delete(key);
    flush(key);
  }
}

if (typeof window !== 'undefined') {
  // `pagehide` fires where `beforeunload` is unreliable (iOS, Tauri),
  // and the composer is exactly where losing the last few seconds
  // would be noticed.
  window.addEventListener('pagehide', flushDrafts);
  document.addEventListener('visibilitychange', () => {
    if (document.visibilityState === 'hidden') flushDrafts();
  });
}
