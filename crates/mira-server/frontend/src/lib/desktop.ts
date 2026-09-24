/**
 * Bridge to the Mira desktop app (apps/desktop). The app injects
 * `window.__MIRA_DESKTOP__` and Tauri's global API into the page; in a
 * normal browser every helper here is a no-op.
 */

type Unlisten = () => void;

interface TauriGlobal {
  core: { invoke: (cmd: string, args?: Record<string, unknown>) => Promise<unknown> };
  event: {
    listen: (event: string, cb: (e: { payload: unknown }) => void) => Promise<Unlisten>;
  };
}

declare global {
  interface Window {
    __MIRA_DESKTOP__?: boolean;
    __TAURI__?: TauriGlobal;
  }
}

/** Where the system browser sends the user after sign-in. Must be listed
 *  in the Supabase project's allowed redirect URLs. */
export const DESKTOP_AUTH_REDIRECT = 'mira://auth-callback';

export function isDesktop(): boolean {
  return window.__MIRA_DESKTOP__ === true && window.__TAURI__ !== undefined;
}

export async function openExternal(url: string): Promise<void> {
  if (window.__TAURI__) {
    await window.__TAURI__.core.invoke('open_external', { url });
  } else {
    window.open(url, '_blank', 'noopener,noreferrer');
  }
}

/** Calls `cb` with each `mira://auth-callback?...` URL the app receives. */
export async function onAuthCallback(cb: (url: URL) => void): Promise<Unlisten> {
  if (!window.__TAURI__) return () => {};
  return window.__TAURI__.event.listen('mira-auth-callback', (e) => {
    try {
      cb(new URL(String(e.payload)));
    } catch {
      /* not a URL; ignore */
    }
  });
}
