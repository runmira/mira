/**
 * Native child webview, positioned over a DOM element.
 *
 * Why this exists: the browser pane needs a real browser — real DOM, real
 * text selection, real history — rendered *inside* the panel. An `<iframe>`
 * can't do that (most sites refuse framing), and a second OS window is not
 * "in the panel". Tauri can add a child webview to the existing window at a
 * tracked rect.
 *
 * The important consequence: the child webview is a **native view stacked
 * above the DOM**, not inside it. It therefore covers anything painted
 * beneath it, and it swallows every pointer event in its rect. Callers must
 * keep their controls outside the tracked element — see `BrowserPane`.
 *
 * Coordinates are logical pixels relative to the window's top-left, so they
 * come from `getBoundingClientRect()` directly. Every write is fire-and-
 * forget: rect syncing runs on scroll and resize, and awaiting each one
 * would queue up behind the compositor.
 */

export type Rect = { x: number; y: number; width: number; height: number };

/** True when a child webview can actually be created.
 *
 *  Deliberately does *not* check the parent window's label. That lives on
 *  the `Window` instance, and the global `__TAURI__.window` is the class, so
 *  a synchronous `.label` read is always `undefined` — which made this
 *  return false and silently downgraded the pane to the iframe path. The
 *  label is resolved asynchronously in `attachChildWebview` instead. */
export function childWebviewAvailable(): boolean {
  if (typeof window === 'undefined') return false;
  // `__MIRA_DESKTOP__` is injected by the app's init script, so it's the
  // reliable "this is the native shell" signal, matching `isDesktop()`.
  if (window.__MIRA_DESKTOP__ !== true) return false;
  return Boolean(window.__TAURI__?.webview?.Webview);
}

/** The parent window's label.
 *
 *  Falls back to `"main"`, which is the label `apps/desktop` builds its
 *  window with, so a failure to introspect doesn't cost us the feature. */
async function resolveParentLabel(): Promise<string> {
  try {
    const current = await window.__TAURI__?.window?.getCurrent?.();
    if (current?.label) return current.label;
  } catch {
    /* fall through to the known label */
  }
  return 'main';
}

/** Tauri webview labels allow `a-zA-Z0-9 / : _`. */
function safeLabel(kind: string, n: number): string {
  return `mira-browser-${kind.replace(/[^a-zA-Z0-9]/g, '')}-${n}`;
}

/** Monotonic per-kind counter so a relaunch never collides with a native
 *  view Tauri still thinks is alive. */
const seq = new Map<string, number>();
function nextSeq(kind: string): number {
  const n = (seq.get(kind) ?? 0) + 1;
  seq.set(kind, n);
  return n;
}

export type ChildWebview = {
  /** Point it at a new URL. */
  navigate: (url: string) => Promise<void>;
  /** Push a new rect. Cheap enough to call on every scroll frame. */
  setRect: (rect: Rect) => void;
  /** CSS-pixel zoom applied inside the page. */
  setZoom: (factor: number) => void;
  focus: () => void;
  /** Tear down. Safe to call twice. */
  close: () => void;
  /** Run JS in the page — used for history control where the webview API
   *  doesn't expose it. */
  eval: (code: string) => Promise<unknown>;
};

/**
 * Create a child webview tracking `element`.
 *
 * `kind` gives each browser/agent tab its own webview. Tauri exposes no
 * "get a handle to an existing child webview" on the global API, so there is
 * no reattach: one webview per attach, destroyed on dispose. `kind` is still
 * worth passing because it keeps the native view's identity predictable when
 * reading a crash log or looking at the OS window list.
 */
export async function attachChildWebview(
  kind: string,
  element: HTMLElement,
  initialUrl: string,
): Promise<ChildWebview> {
  const Webview = window.__TAURI__?.webview?.Webview;
  if (!Webview) throw new Error('child webviews unavailable');
  const parent = await resolveParentLabel();

  const rect = element.getBoundingClientRect();
  const wv = new Webview(parent, safeLabel(kind, nextSeq(kind)), {
    url: initialUrl,
    // Geometry at construction, not just via setPosition/setSize after:
    // otherwise the native view first paints at 0,0 and visibly jumps.
    x: Math.round(rect.left),
    y: Math.round(rect.top),
    width: Math.round(rect.width),
    height: Math.round(rect.height),
  });

  const applyRect = (r: Rect) => {
    void wv.setPosition(Math.round(r.x), Math.round(r.y));
    void wv.setSize(Math.round(r.width), Math.round(r.height));
  };

  let closed = false;
  return {
    async navigate(url: string) {
      // `eval` runs *inside* the page, so this is the page navigating
      // itself — cross-origin-safe, and it keeps the same native view alive
      // so scroll position survives a round trip through the back button.
      await wv.eval(
        `window.location.assign(${JSON.stringify(url)})`,
      );
    },
    setRect: applyRect,
    setZoom(factor: number) {
      void wv.setZoom(factor);
    },
    focus() {
      void wv.setFocus();
    },
    async eval(code: string) {
      return wv.eval(code);
    },
    close() {
      if (closed) return;
      closed = true;
      void wv.close();
    },
  };
}

/**
 * Keep a child webview glued to an element.
 *
 * Returns a disposer. A `ResizeObserver` alone isn't enough: the panel is
 * resizable *and* the column width animates, and the element's position
 * changes whenever anything above it reflows — so this also polls on rAF
 * while the rect is actually moving, and goes idle once it settles.
 */
export function trackElement(
  element: HTMLElement,
  apply: (rect: Rect) => void,
): () => void {
  let last = '';
  let raf = 0;
  let idleFrames = 0;

  const measure = () => {
    const r = element.getBoundingClientRect();
    const key = `${Math.round(r.left)},${Math.round(r.top)},${Math.round(
      r.width,
    )},${Math.round(r.height)}`;
    if (key !== last) {
      last = key;
      idleFrames = 0;
      apply({ x: r.left, y: r.top, width: r.width, height: r.height });
    } else if (idleFrames < 3) {
      // A few confirming frames, then stop burning rAF on a static panel.
      idleFrames++;
    }
    raf = requestAnimationFrame(measure);
  };

  raf = requestAnimationFrame(measure);

  const ro = new ResizeObserver(() => {
    idleFrames = 0;
  });
  ro.observe(element);

  return () => {
    cancelAnimationFrame(raf);
    ro.disconnect();
  };
}
