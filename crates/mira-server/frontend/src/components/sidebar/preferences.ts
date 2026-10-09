export const COLLAPSED_KEY = 'mira.sidebar.collapsed-projects';

export const RECENTS_OPEN_KEY = 'mira.sidebar.recents-open';

export const SETTLED_OPEN_KEY = 'mira.sidebar.settled-open';

export const PER_GROUP_LIMIT = 5;

export const RECENTS_LIMIT = 3;

export const SHOW_MORE_KEY = 'mira.sidebar.expanded-projects';

export const FORKS_KEY = 'mira.sidebar.hidden-forks';

export const SCROLL_KEY = 'mira.sidebar.scroll';

export const NEEDS_MORE_KEY = 'mira.sidebar.needs-more';

export const RECENTS_MORE_KEY = 'mira.sidebar.recents-more';

/** The settled shelf is history: a page at a time. */
export const SETTLED_PAGE = 10;

/* ---------- collapsed-state persistence ---------- */

export function loadSet(key: string): Set<string> {
  try {
    const value: unknown = JSON.parse(localStorage.getItem(key) || '[]');
    return new Set(
      Array.isArray(value) ? value.filter((v): v is string => typeof v === 'string') : [],
    );
  } catch {
    return new Set();
  }
}

export function saveSet(key: string, value: Set<string>) {
  try {
    localStorage.setItem(key, JSON.stringify([...value]));
  } catch {
    /* optional storage */
  }
}

export function loadScroll(): number {
  try {
    return Math.max(0, Number(localStorage.getItem(SCROLL_KEY)) || 0);
  } catch {
    return 0;
  }
}

export function loadCollapsed(): Set<string> {
  try {
    const raw = localStorage.getItem(COLLAPSED_KEY);
    if (!raw) return new Set();
    const arr = JSON.parse(raw);
    return new Set(Array.isArray(arr) ? arr : []);
  } catch {
    return new Set();
  }
}

export function persistCollapsed(set: Set<string>) {
  try {
    localStorage.setItem(COLLAPSED_KEY, JSON.stringify([...set]));
  } catch {
    /* private-mode etc; ignore */
  }
}

export function loadFlag(key: string, fallback: boolean): boolean {
  try {
    const v = localStorage.getItem(key);
    return v === null ? fallback : v === '1';
  } catch {
    return fallback;
  }
}

export function saveFlag(key: string, value: boolean) {
  try {
    localStorage.setItem(key, value ? '1' : '0');
  } catch {
    /* private-mode etc; ignore */
  }
}

export const UNREAD_KEY = 'mira.sidebar.unread';

export function loadUnread(): Set<string> {
  try {
    const v = JSON.parse(localStorage.getItem(UNREAD_KEY) ?? '[]');
    return new Set(Array.isArray(v) ? v.filter((x): x is string => typeof x === 'string') : []);
  } catch {
    return new Set();
  }
}

export function saveUnread(ids: Set<string>) {
  try {
    localStorage.setItem(UNREAD_KEY, JSON.stringify([...ids].slice(-200)));
  } catch {
    /* private mode */
  }
}
