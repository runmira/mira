/**
 * Bridge to the Mira desktop app (apps/desktop). The app injects
 * `window.__MIRA_DESKTOP__` and Tauri's global API into the page; in a
 * normal browser every helper here is a no-op.
 */

type Unlisten = () => void;

/** The slice of Tauri's `webview` module that the browser pane uses. Kept
 *  structural rather than pulling in `@tauri-apps/api` — the desktop build
 *  sets `withGlobalTauri`, so `window.__TAURI__.webview` is already here and
 *  the npm package isn't in the web build's dependency graph. */
interface TauriWebviewHandle {
  setPosition: (x: number, y: number) => Promise<void>;
  setSize: (width: number, height: number) => Promise<void>;
  setFocus: () => Promise<void>;
  setZoom: (factor: number) => Promise<void>;
  close: () => Promise<void>;
  eval: (code: string) => Promise<unknown>;
}

interface TauriWebviewCtor {
  new (
    parentLabel: string,
    label: string,
    options: Record<string, unknown>,
  ): TauriWebviewHandle;
  getAll: () => Promise<{ label: string }[]>;
}

interface TauriGlobal {
  core: { invoke: (cmd: string, args?: Record<string, unknown>) => Promise<unknown> };
  event: {
    listen: (event: string, cb: (e: { payload: unknown }) => void) => Promise<Unlisten>;
  };
  window?: {
    // The global bundle exposes the `Window` *class*, not an instance, so
    // `.label` is undefined on it. The parent window's label only comes
    // back from the async `getCurrent()`.
    getCurrent?: () => Promise<{ label: string }>;
    /** The current window, for appearance (`setTheme`). */
    getCurrentWindow?: () => { setTheme?: (theme: 'light' | 'dark' | null) => Promise<void> };
  };
  /** App metadata (`core:app:default`). */
  app?: { getVersion?: () => Promise<string> };
  webview?: {
    Webview: TauriWebviewCtor;
    /** This page's own webview, for page zoom. */
    getCurrentWebview?: () => { setZoom?: (scale: number) => Promise<void> };
  };
}

declare global {
  interface Window {
    __MIRA_DESKTOP__?: boolean;
    __TAURI__?: TauriGlobal;
    /**
     * Set by the app's init script when the native window has no title bar
     * of its own — the app draws its own header, with the system traffic
     * lights floating over it and vibrancy behind the sidebar.
     */
    __MIRA_CHROME__?: { hiddenTitleBar: boolean; translucent: boolean };
    /** The desktop app's release channel, set by its init script. */
    __MIRA_CHANNEL__?: string;
  }
}

/** True when the app is drawing its own header in place of the native macOS
 *  title bar, so the UI has to reserve room for the traffic lights and can
 *  let the window's vibrancy show through the sidebar. */
export function hasHiddenTitleBar(): boolean {
  if (typeof window === 'undefined') return false;
  return window.__MIRA_DESKTOP__ === true && window.__MIRA_CHROME__?.hiddenTitleBar === true;
}

/** Padding that clears the macOS traffic lights, in the same shape the
 *  traffic lights occupy. The header is also a drag region, so the window
 *  stays movable with no title bar. */
export const TRAFFIC_LIGHT_INSET = { top: 28, left: 78 } as const;

export type Channel = 'stable' | 'beta' | 'alpha';

/** Which desktop build this is. Alpha and beta are separate apps with
 *  their own `mira-alpha://` / `mira-beta://` schemes. */
export function desktopChannel(): Channel {
  const c = typeof window === 'undefined' ? undefined : window.__MIRA_CHANNEL__;
  return c === 'alpha' || c === 'beta' ? c : 'stable';
}

/** Where the system browser sends the user after sign-in. Every channel's
 *  scheme must be listed in the Supabase project's allowed redirect URLs. */
export function desktopAuthRedirect(): string {
  const channel = desktopChannel();
  return `${channel === 'stable' ? 'mira' : `mira-${channel}`}://auth-callback`;
}

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

/** Calls `cb` with each `mira[-channel]://auth-callback?...` URL the app receives. */
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
