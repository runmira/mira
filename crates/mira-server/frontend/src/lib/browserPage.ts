import { useSyncExternalStore } from 'react';

/**
 * The page the browser pane is showing, for chrome outside the pane: the
 * panel's "Browser" tab wears the site's icon. The pane publishes; anything
 * may read.
 */
export type BrowserPage = { url: string; icon: string | null };

let page: BrowserPage = { url: '', icon: null };
const listeners = new Set<() => void>();

export function setBrowserPage(next: BrowserPage): void {
  if (next.url === page.url && next.icon === page.icon) return;
  page = next;
  listeners.forEach((l) => l());
}

export function useBrowserPage(): BrowserPage {
  return useSyncExternalStore(
    (l) => {
      listeners.add(l);
      return () => listeners.delete(l);
    },
    () => page,
  );
}

/** The icon a page declares (`<link rel="icon">`), asked of the agent
 *  browser; the origin's `/favicon.ico` when it declares none. Loaded from
 *  the site itself: no third-party favicon service learns what's browsed. */
export async function pageIcon(url: string): Promise<string | null> {
  let origin: string;
  try {
    const u = new URL(url);
    if (u.protocol !== 'http:' && u.protocol !== 'https:') return null;
    origin = u.origin;
  } catch {
    return null;
  }
  try {
    const res = await fetch('/api/browser/action', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({
        action: {
          action: 'evaluate',
          expression:
            "(document.querySelector('link[rel~=\"icon\"][href]')||document.querySelector('link[rel=\"apple-touch-icon\"][href]'))?.href||''",
        },
      }),
    });
    if (res.ok) {
      const { text } = (await res.json()) as { text?: string };
      if (text && /^(https?:\/\/|data:image\/)/.test(text)) return text;
    }
  } catch {
    /* fall back to the conventional path */
  }
  return `${origin}/favicon.ico`;
}
