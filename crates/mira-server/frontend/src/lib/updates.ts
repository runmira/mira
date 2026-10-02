/**
 * Updates, for the toolbar's update button.
 *
 * - Desktop app: checks its channel's update feed (a signed build per
 *   channel, published by the desktop release workflow) and installs it.
 * - Browser: compares the server's version with the latest CLI release on
 *   GitHub; a page can't upgrade a CLI, so it offers the command instead.
 */
import { useCallback, useEffect, useRef, useState } from 'react';
import { isDesktop } from './desktop';

export type UpdateInfo = {
  version: string;
  current: string;
  channel: 'stable' | 'beta' | 'alpha';
  /** RFC 3339. */
  date: string | null;
  /** Release notes, markdown: the release's highlights. */
  notes: string | null;
  /** Where it comes from: the desktop app installs it, the CLI is told how. */
  target: 'desktop' | 'cli';
};

/** Release notes on the web, for "What's new" links. */
export const RELEASES_URL = 'https://runmira.dev/releases';

/** a > b, for plain or pre-release semver (`0.6.1-alpha.2`). */
export function newerVersion(a: string, b: string): boolean {
  const parse = (v: string) => {
    const [core, pre] = v.replace(/^v/, '').split('-', 2);
    return { nums: core.split('.').map((n) => Number(n) || 0), pre: pre ?? null };
  };
  const x = parse(a);
  const y = parse(b);
  for (let i = 0; i < 3; i++) {
    if ((x.nums[i] ?? 0) !== (y.nums[i] ?? 0)) return (x.nums[i] ?? 0) > (y.nums[i] ?? 0);
  }
  if (x.pre === y.pre) return false;
  if (x.pre === null) return true; // 1.0.0 > 1.0.0-beta
  if (y.pre === null) return false;
  return x.pre.localeCompare(y.pre, undefined, { numeric: true }) > 0;
}

async function currentVersion(): Promise<string | null> {
  try {
    if (isDesktop()) return ((await window.__TAURI__?.app?.getVersion?.()) as string | undefined) ?? null;
    const r = await fetch('/api/version');
    return r.ok ? ((await r.json()) as { version: string }).version : null;
  } catch {
    return null;
  }
}

/** The newest CLI release, when it's newer than this server. */
async function cliUpdate(current: string): Promise<UpdateInfo | null> {
  const r = await fetch('https://api.github.com/repos/runmira/mira/releases/latest', {
    headers: { Accept: 'application/vnd.github+json' },
  });
  if (!r.ok) return null;
  const rel = (await r.json()) as { tag_name: string; published_at?: string; body?: string };
  const version = rel.tag_name.replace(/^v/, '');
  if (!newerVersion(version, current)) return null;
  return { version, current, channel: 'stable', date: rel.published_at ?? null, notes: rel.body ?? null, target: 'cli' };
}

export type UpdatePhase =
  | { kind: 'idle' }
  | { kind: 'downloading'; downloaded: number; total: number | null }
  | { kind: 'restarting' }
  | { kind: 'error'; message: string };

/** How often a running app looks for a new build. */
const CHECK_EVERY_MS = 6 * 60 * 60 * 1000;
/** Window focus re-checks, but not more often than this. */
const FOCUS_MIN_GAP_MS = 30 * 60 * 1000;

function invoke<T>(cmd: string): Promise<T> {
  return window.__TAURI__!.core.invoke(cmd) as Promise<T>;
}

/** The pending update, if any, and the actions on it. */
export function useAppUpdate() {
  const [update, setUpdate] = useState<UpdateInfo | null>(null);
  const [phase, setPhase] = useState<UpdatePhase>({ kind: 'idle' });
  const [current, setCurrent] = useState<string | null>(null);
  const [checking, setChecking] = useState(false);
  const [checkedAt, setCheckedAt] = useState<number | null>(null);
  const [checkFailed, setCheckFailed] = useState(false);
  const lastCheck = useRef(0);
  const channel = (window.__MIRA_CHANNEL__ === 'alpha' || window.__MIRA_CHANNEL__ === 'beta' ? window.__MIRA_CHANNEL__ : 'stable') as UpdateInfo['channel'];

  useEffect(() => {
    void currentVersion().then(setCurrent);
  }, []);

  const check = useCallback(async () => {
    lastCheck.current = Date.now();
    setChecking(true);
    setCheckFailed(false);
    try {
      if (isDesktop()) {
        const u = await invoke<Omit<UpdateInfo, 'target'> | null>('check_update');
        setUpdate(u ? { ...u, target: 'desktop' } : null);
      } else {
        const now = current ?? (await currentVersion());
        setUpdate(now ? await cliUpdate(now) : null);
      }
      setCheckedAt(Date.now());
    } catch {
      // Offline, or nothing published yet: say so, try again later.
      setCheckFailed(true);
    } finally {
      setChecking(false);
    }
  }, [current]);

  useEffect(() => {
    // Not at launch itself: the app is busy starting its server.
    const first = window.setTimeout(() => void check(), 8_000);
    const every = window.setInterval(() => void check(), CHECK_EVERY_MS);
    const onFocus = () => {
      if (Date.now() - lastCheck.current > FOCUS_MIN_GAP_MS) void check();
    };
    window.addEventListener('focus', onFocus);
    return () => {
      window.clearTimeout(first);
      window.clearInterval(every);
      window.removeEventListener('focus', onFocus);
    };
  }, [check]);

  const install = useCallback(async () => {
    if (!window.__TAURI__) return;
    setPhase({ kind: 'downloading', downloaded: 0, total: null });
    const unlisten = await window.__TAURI__.event.listen('mira-update-progress', (e) => {
      const p = e.payload as { downloaded: number; total: number | null };
      setPhase({ kind: 'downloading', downloaded: p.downloaded, total: p.total });
    });
    try {
      await invoke('install_update');
      setPhase({ kind: 'restarting' });
      await invoke('restart_app');
    } catch (e) {
      setPhase({ kind: 'error', message: String((e as Error)?.message ?? e) });
    } finally {
      unlisten();
    }
  }, []);

  return { update, phase, install, check, current, channel, checking, checkedAt, checkFailed };
}

/** One shipped thing, from a `- **Title.** what it does` bullet. */
export type NoteItem = { title: string | null; body: string };
export type NoteSection = { heading: string | null; items: NoteItem[] };

/**
 * Release notes as sections of items, for the update popup. Notes are
 * written as markdown bullets under `##`/`###` headings; anything else
 * (intro paragraphs) is left to the caller's fallback.
 */
export function parseNotes(md: string): NoteSection[] {
  const sections: NoteSection[] = [];
  let current: NoteSection = { heading: null, items: [] };
  let item: NoteItem | null = null;
  const flush = () => {
    if (item) current.items.push(item);
    item = null;
  };
  for (const raw of md.split('\n')) {
    const line = raw.trimEnd();
    const heading = /^#{2,4}\s+(.*)$/.exec(line);
    const bullet = /^[-*]\s+(.*)$/.exec(line);
    if (heading) {
      flush();
      if (current.items.length) sections.push(current);
      current = { heading: heading[1].trim(), items: [] };
    } else if (bullet) {
      flush();
      const text = bullet[1];
      const titled = /^\*\*(.+?)\*\*\s*(.*)$/.exec(text);
      item = titled
        ? { title: titled[1].replace(/[.:]$/, ''), body: titled[2] }
        : { title: null, body: text };
    } else if (item && /^\s+\S/.test(raw)) {
      item.body += ` ${line.trim()}`;
    } else if (!line.trim()) {
      flush();
    }
  }
  flush();
  if (current.items.length) sections.push(current);
  return sections;
}
