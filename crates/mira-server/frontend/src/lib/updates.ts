/**
 * In-app updates for the desktop app. The app checks its channel's update
 * feed (a signed build per channel, published by the desktop release
 * workflow) and the page offers it. In a browser there are no updates to
 * offer: everything here stays idle.
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
};

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
  const lastCheck = useRef(0);

  const check = useCallback(async () => {
    if (!isDesktop()) return;
    lastCheck.current = Date.now();
    try {
      setUpdate(await invoke<UpdateInfo | null>('check_update'));
    } catch {
      // Offline, or the feed isn't published yet: try again later, quietly.
    }
  }, []);

  useEffect(() => {
    if (!isDesktop()) return;
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

  return { update, phase, install, check };
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
