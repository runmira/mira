import { useCallback, useEffect, useRef, useState } from 'react';
import {
  ArrowLeft,
  ArrowRight,
  Camera,
  ExternalLink,
  FileText,
  Globe,
  LoaderCircle,
  RefreshCw,
  Square,
  X,
} from 'lucide-react';
import { cn } from '@/lib/utils';
import {
  attachChildWebview,
  childWebviewAvailable,
  trackElement,
  type ChildWebview,
} from '@/lib/childWebview';

type Shot = { png_base64: string; width: number; height: number };
type ActionResult = { text: string; screenshot?: Shot };

/** CSS-pixel viewport presets, applied as in-page zoom on the native
 *  webview. It's not true device emulation the way CDP's
 *  `setDeviceMetricsOverride` is — see the note on `zoomFor`. */
const VIEWPORTS = [
  { id: 'fluid', label: 'Fluid', w: 0, h: 0 },
  { id: 'phone', label: 'Phone', w: 390, h: 844 },
  { id: 'tablet', label: 'Tablet', w: 834, h: 1112 },
  { id: 'laptop', label: 'Laptop', w: 1280, h: 800 },
  { id: 'desktop', label: 'Desktop', w: 1680, h: 1050 },
] as const;

type ViewportId = (typeof VIEWPORTS)[number]['id'];

const START_URL = 'https://example.com';

/**
 * How the pane renders a page, best available first.
 *
 * - `native` — a Tauri child webview: a real browser view inside the main
 *   window, same OS window, real DOM. Desktop only.
 * - `iframe` — a real framed page, which is the best a plain browser tab can
 *   do. Works for the large set of sites that permit framing.
 * - `cdp` — the agent browser, live: the one Mira's model and external
 *   agents drive, streamed frame by frame, with clicks and keys passed
 *   through so the user can take over (a login, a captcha). The default,
 *   because it is the browser the agents are actually using.
 */
type Mode = 'native' | 'iframe' | 'cdp';

/** One live frame of the agent browser. `w`/`h` are CSS pixels. */
type LiveFrame = { src: string; w: number; h: number };

/** DOM key names → the browser tool's key syntax. */
const KEY_NAMES: Record<string, string> = {
  Enter: 'Return', Backspace: 'BackSpace', Tab: 'Tab', Escape: 'Escape', Delete: 'Delete',
  ArrowUp: 'Up', ArrowDown: 'Down', ArrowLeft: 'Left', ArrowRight: 'Right',
  Home: 'Home', End: 'End', PageUp: 'Page_Up', PageDown: 'Page_Down', ' ': 'space',
};

export function BrowserPane() {
  const [mode, setMode] = useState<Mode>('cdp');
  const [frame, setFrame] = useState<LiveFrame | null>(null);
  const [live, setLive] = useState(false);
  const [url, setUrl] = useState('');
  const [draft, setDraft] = useState('');
  const [snapshot, setSnapshot] = useState<string | null>(null);
  const [viewport, setViewport] = useState<ViewportId>('laptop');
  const [busy, setBusy] = useState<null | string>(null);
  const [error, setError] = useState<string | null>(null);
  const inFlight = useRef(false);
  /** Bumped to force the <iframe> to remount (see `reload`). */
  const [frameKey, setFrameKey] = useState(0);

  /** The element the native webview is positioned over. Must be empty of
   *  anything the user needs to click — the native view covers the DOM and
   *  eats pointer events in its rect. */
  const surfaceRef = useRef<HTMLDivElement | null>(null);
  const wvRef = useRef<ChildWebview | null>(null);

  /* ---------------- native webview lifecycle ---------------- */

  useEffect(() => {
    if (mode !== 'native') return;
    const el = surfaceRef.current;
    if (!el) return;

    let disposed = false;
    let stopTracking: (() => void) | null = null;
    const initial = url || START_URL;

    attachChildWebview('user', el, initial)
      .then((wv) => {
        if (disposed) {
          wv.close();
          return;
        }
        wvRef.current = wv;
        setUrl((u) => u || initial);
        stopTracking = trackElement(el, (r) => wv.setRect(r));
      })
      .catch((e: unknown) => {
        // Most likely a missing capability or a blocked remote URL. The CDP
        // path still works, so degrade instead of showing a dead pane.
        if (!disposed) {
          setMode('cdp');
          setError(
            `Embedded browser unavailable (${e instanceof Error ? e.message : String(e)}). Falling back to the agent browser.`,
          );
        }
      });

    return () => {
      disposed = true;
      stopTracking?.();
      wvRef.current?.close();
      wvRef.current = null;
    };
    // Intentionally mount-only: re-creating the native view on every url
    // change would reload the page. Navigation goes through `wv.navigate`.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [mode]);

  /* ---------------- live agent browser ---------------- */

  useEffect(() => {
    if (mode !== 'cdp') return;
    const es = new EventSource('/api/browser/live');
    es.onopen = () => setLive(true);
    es.onerror = () => setLive(false);
    es.onmessage = (m) => {
      let ev: { type: string; jpeg_base64?: string; width?: number; height?: number; url?: string };
      try {
        ev = JSON.parse(m.data);
      } catch {
        return;
      }
      if (ev.type === 'frame' && ev.jpeg_base64) {
        setFrame({ src: `data:image/jpeg;base64,${ev.jpeg_base64}`, w: ev.width ?? 0, h: ev.height ?? 0 });
      } else if (ev.type === 'navigated' && ev.url && ev.url !== 'about:blank') {
        // The agent navigated: the address bar follows it.
        setUrl(ev.url);
      } else if (ev.type === 'closed') {
        setFrame(null);
      }
    };
    return () => {
      es.close();
      setLive(false);
    };
  }, [mode]);

  const send = useCallback((input: Record<string, unknown>) => {
    void fetch('/api/browser/input', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify(input),
    }).catch(() => {});
  }, []);

  /** A point on the frame image in the page's CSS pixels. */
  function toPage(e: { clientX: number; clientY: number; currentTarget: Element }) {
    const r = e.currentTarget.getBoundingClientRect();
    if (!frame || r.width === 0 || r.height === 0) return null;
    return {
      x: ((e.clientX - r.left) / r.width) * frame.w,
      y: ((e.clientY - r.top) / r.height) * frame.h,
    };
  }

  const wheelAcc = useRef({ dx: 0, dy: 0, t: 0 as ReturnType<typeof setTimeout> | 0 });

  /* ---------------- CDP actions ---------------- */

  const run = useCallback(
    async (label: string, action: Record<string, unknown>) => {
      if (inFlight.current) return null;
      inFlight.current = true;
      setBusy(label);
      setError(null);
      try {
        const res = await fetch('/api/browser/action', {
          method: 'POST',
          headers: { 'content-type': 'application/json' },
          body: JSON.stringify({ action }),
        });
        if (!res.ok) {
          const body = await res.json().catch(() => null);
          throw new Error(body?.error ?? `HTTP ${res.status}`);
        }
        return (await res.json()) as ActionResult;
      } catch (e) {
        setError(e instanceof Error ? e.message : String(e));
        return null;
      } finally {
        inFlight.current = false;
        setBusy(null);
      }
    },
    [],
  );


  /* ---------------- viewport presets ----------------
   *
   * The native webview is a fixed-size native view; asking it to emulate a
   * 390px viewport isn't something the granted permission set exposes. What
   * we can do is scale the rendered page, which is honest about being a
   * *preview* rather than a real device emulation. The CDP path does get
   * true emulation via `set_viewport`.
   */
  function zoomFor(vp: ViewportId): number {
    const preset = VIEWPORTS.find((v) => v.id === vp)!;
    if (preset.w === 0 || mode !== 'native') return 1;
    // Scale so the preset's width is what's actually shown in the pane.
    const el = surfaceRef.current;
    if (!el) return 1;
    return Math.min(1, preset.w / Math.max(1, el.clientWidth));
  }

  /** Ask the server whether the origin will let us frame it. A browser
   *  can't answer this itself — reading the headers is cross-origin by
   *  definition — and a blank frame with no explanation is worse than
   *  falling back automatically. */
  async function probeFrameable(target: string): Promise<{
    ok: boolean;
    reason: string;
    finalUrl: string;
  }> {
    try {
      const res = await fetch(
        `/api/browser/embeddable?url=${encodeURIComponent(target)}`,
      );
      if (!res.ok) return { ok: true, reason: '', finalUrl: target };
      return (await res.json()) as { ok: boolean; reason: string; finalUrl: string };
    } catch {
      // Probe unreachable: assume it frames and let the frame speak for
      // itself rather than blocking the user.
      return { ok: true, reason: '', finalUrl: target };
    }
  }

  async function navigate(target: string) {
    const clean = target.trim();
    if (!clean) return;
    setUrl(clean);
    setDraft('');
    setSnapshot(null);
    setError(null);

    if (mode === 'native') {
      try {
        await wvRef.current?.navigate(clean);
        return;
      } catch (e) {
        setError(e instanceof Error ? e.message : String(e));
        return;
      }
    }

    if (mode === 'iframe') {
      const probe = await probeFrameable(clean);
      if (probe.ok) {
        setUrl(probe.finalUrl || clean);
        return;
      }
      // Refused. Switch to the agent browser and say why, rather than
      // showing an empty frame.
      setMode('cdp');
      setError(`${probe.reason} Showing it in the agent browser instead.`);
    }

    // The live view shows the result as it loads; nothing to capture.
    await run('loading', { action: 'navigate', url: clean });
  }

  async function go(dir: 'back' | 'forward') {
    if (mode === 'iframe') {
      // A cross-origin frame's history is off-limits, so the only honest
      // back/forward here is one the user typed.
      return;
    }
    if (mode === 'native') {
      await wvRef.current?.eval(`history.go(${dir === 'back' ? -1 : 1})`);
      return;
    }
    await run(dir, { action: dir });
  }

  async function reload() {
    if (mode === 'iframe') {
      // A cross-origin frame can't be told to reload, and re-assigning the
      // same `src` is a no-op. Changing the element's `key` remounts it,
      // which is the only thing that actually re-fetches.
      setFrameKey((k) => k + 1);
      return;
    }
    if (mode === 'native') {
      await wvRef.current?.eval('location.reload()');
      return;
    }
    await run('reloading', { action: 'reload' });
  }

  async function takeSnapshot() {
    if (mode === 'native') {
      // `eval` can't read a cross-origin document, so this only works on
      // pages the webview can script. Surface the failure instead of
      // silently showing nothing.
      try {
        const text = await wvRef.current?.eval('document.body.innerText');
        setSnapshot(typeof text === 'string' ? text : String(text ?? ''));
      } catch {
        setError('Could not read this page — cross-origin pages are not scriptable.');
      }
      return;
    }
    const out = await run('snapshot', { action: 'snapshot' });
    if (out) setSnapshot(out.text || '(page produced no text)');
  }

  async function shutdown() {
    if (mode === 'native') {
      wvRef.current?.close();
      wvRef.current = null;
    } else {
      await run('close', { action: 'close' });
    }
    setFrame(null);
    setSnapshot(null);
    setUrl('');
  }

  function changeViewport(id: ViewportId) {
    setViewport(id);
    if (mode === 'cdp') {
      const preset = VIEWPORTS.find((v) => v.id === id)!;
      void run('viewport', {
        action: 'set_viewport',
        width: preset.w,
        height: preset.h,
        mobile: id === 'phone',
      });
    } else {
      wvRef.current?.setZoom(zoomFor(id));
      wvRef.current?.focus();
    }
  }

  const showSnapshot = snapshot !== null;

  return (
    <div className="flex h-full min-h-0 flex-col">
      {/* URL bar — outside the native view's rect on purpose. */}
      <form
        className="flex shrink-0 items-center gap-1 border-b border-border/60 px-2 py-1.5"
        onSubmit={(e) => {
          e.preventDefault();
          void navigate(draft || url || START_URL);
        }}
      >
        <IconBtn title="Back" onClick={() => void go('back')} disabled={!url && !frame}>
          <ArrowLeft className="size-3.5" />
        </IconBtn>
        <IconBtn title="Forward" onClick={() => void go('forward')} disabled={!url && !frame}>
          <ArrowRight className="size-3.5" />
        </IconBtn>
        <IconBtn title="Reload" onClick={() => void reload()} disabled={!url && !frame}>
          <RefreshCw className="size-3.5" />
        </IconBtn>

        <input
          value={draft || url}
          onChange={(e) => setDraft(e.target.value)}
          placeholder="Search or enter address"
          className="min-w-0 flex-1 rounded-md border border-border/60 bg-transparent px-2 py-0.5 text-[12px] text-foreground outline-none transition-colors placeholder:text-muted-foreground/60 focus:border-border"
        />

        <IconBtn
          title="Open in your browser"
          onClick={() => url && window.open(url, '_blank', 'noopener,noreferrer')}
          disabled={!url}
        >
          <ExternalLink className="size-3.5" />
        </IconBtn>
        <IconBtn title="Close browser" onClick={() => void shutdown()} disabled={!url}>
          <Square className="size-3" />
        </IconBtn>
      </form>

      {/* viewport presets + per-mode actions */}
      <div className="flex shrink-0 items-center gap-1 border-b border-border/40 px-2 py-1">
        {VIEWPORTS.map((v) => (
          <button
            key={v.id}
            type="button"
            onClick={() => changeViewport(v.id)}
            className={cn(
              'rounded px-1.5 py-0.5 text-[11px] transition-colors',
              viewport === v.id
                ? 'bg-white/[0.1] text-foreground'
                : 'text-muted-foreground hover:bg-white/[0.05] hover:text-foreground',
            )}
          >
            {v.label}
          </button>
        ))}
        <span className="flex-1" />
        <span
          className="mr-1 text-[10.5px] text-muted-foreground/60"
          title={
            mode === 'native'
              ? 'Native webview inside the app window. Viewport presets scale the page.'
              : mode === 'iframe'
                ? 'Framed page. Works for sites that allow embedding; back and reload are limited on cross-origin pages.'
                : 'The browser Mira and your agents use, live. Click, scroll and type here to take over.'
          }
        >
          {mode === 'cdp' ? (
            <span className="inline-flex items-center gap-1.5">
              <span className={cn('size-1.5 rounded-full', live && frame ? 'animate-pulse bg-emerald-400' : 'bg-muted-foreground/40')} />
              {live && frame ? 'live' : 'agent'}
            </span>
          ) : mode === 'native' ? 'embedded' : 'framed'}
        </span>
        {mode === 'cdp' ? (
          <IconBtn
            title="Try embedding this page instead"
            onClick={() => {
              setMode('iframe');
              setError(null);
                        setFrameKey((k) => k + 1);
            }}
            disabled={!url}
          >
            <Globe className="size-3.5" />
          </IconBtn>
        ) : (
          <IconBtn
            title="Show the agent browser instead"
            onClick={() => {
              setMode('cdp');
              setError(null);
              if (url) void run('loading', { action: 'navigate', url });
            }}
          >
            <Camera className="size-3.5" />
          </IconBtn>
        )}
        <IconBtn title="Extract page text" onClick={() => void takeSnapshot()} disabled={!url}>
          <FileText className="size-3.5" />
        </IconBtn>
        {showSnapshot && (
          <IconBtn title="Close text view" onClick={() => setSnapshot(null)}>
            <X className="size-3.5" />
          </IconBtn>
        )}
      </div>

      {busy && (
        <div className="flex shrink-0 items-center gap-1.5 border-b border-border/40 px-2.5 py-1 text-[11.5px] text-muted-foreground">
          <LoaderCircle className="size-3 animate-spin" />
          {busy}…
        </div>
      )}
      {error && (
        <div className="shrink-0 border-b border-destructive/30 bg-destructive/10 px-2.5 py-1.5 text-[11.5px] text-destructive">
          {error}
        </div>
      )}

      {/* body */}
      <div className="min-h-0 flex-1 overflow-hidden bg-black/30">
        {!url && !(mode === 'cdp' && frame) && (
          <EmptyState live={mode === 'cdp'} onStart={() => void navigate(START_URL)} />
        )}

        {url && showSnapshot && (
          <div className="h-full overflow-auto">
            <pre className="whitespace-pre-wrap break-words p-3 font-mono text-[11.5px] leading-relaxed text-foreground/85">
              {snapshot}
            </pre>
          </div>
        )}

        {url && !showSnapshot && mode === 'native' && (
          // The native webview is painted over this element. Keep it empty.
          <div ref={surfaceRef} className="h-full w-full bg-white" />
        )}

        {url && !showSnapshot && mode === 'iframe' && (
          <iframe
            key={frameKey}
            src={url}
            title={url}
            // A framed page is untrusted: no same-origin access, no top
            // navigation, no plugin content. `allow-scripts` is required
            // for most real sites to work at all.
            sandbox="allow-scripts allow-forms allow-popups allow-same-origin allow-downloads"
            referrerPolicy="no-referrer"
            className="size-full border-0 bg-white"
          />
        )}

        {(url || frame) && !showSnapshot && mode === 'cdp' && (
          <div
            tabIndex={0}
            className="flex h-full w-full items-center justify-center outline-none focus-visible:ring-1 focus-visible:ring-mira-blue/40"
            onKeyDown={(e) => {
              if (!frame) return;
              const mod = e.metaKey ? 'meta+' : e.ctrlKey ? 'ctrl+' : '';
              if (!mod && e.key.length === 1) {
                send({ type: 'text', text: e.key });
              } else if (KEY_NAMES[e.key] || (mod && e.key.length === 1)) {
                send({ type: 'key', key: `${mod}${e.altKey ? 'alt+' : ''}${KEY_NAMES[e.key] ?? e.key.toLowerCase()}` });
              } else {
                return;
              }
              e.preventDefault();
            }}
          >
            {frame ? (
              <img
                src={frame.src}
                alt={url ? `Live view of ${url}` : 'Live view of the agent browser'}
                draggable={false}
                className="max-h-full max-w-full cursor-default select-none object-contain"
                onClick={(e) => {
                  const p = toPage(e);
                  (e.currentTarget.parentElement as HTMLElement | null)?.focus();
                  if (p) send({ type: 'click', ...p });
                }}
                onWheel={(e) => {
                  const p = toPage(e);
                  if (!p) return;
                  // Coalesce a burst of wheel events into one scroll.
                  const acc = wheelAcc.current;
                  acc.dx += e.deltaX;
                  acc.dy += e.deltaY;
                  if (acc.t) return;
                  acc.t = setTimeout(() => {
                    send({ type: 'scroll', ...p, dx: acc.dx, dy: acc.dy });
                    acc.dx = 0;
                    acc.dy = 0;
                    acc.t = 0;
                  }, 60);
                }}
              />
            ) : (
              <p className="flex items-center gap-2 p-3 text-[12.5px] text-muted-foreground">
                <LoaderCircle className="size-3.5 animate-spin" /> Starting the browser…
              </p>
            )}
          </div>
        )}
      </div>
    </div>
  );
}

function EmptyState({ onStart, live }: { onStart: () => void; live?: boolean }) {
  const native = childWebviewAvailable();
  if (live) {
    return (
      <div className="flex h-full flex-col items-center justify-center gap-3 p-6 text-center">
        <span className="flex size-10 items-center justify-center rounded-xl border border-border/60 bg-white/[0.04] text-muted-foreground">
          <Globe className="size-5" />
        </span>
        <div>
          <p className="text-[13px] font-medium text-foreground">The agent browser</p>
          <p className="mt-1 max-w-[34ch] text-[12px] text-muted-foreground">
            Mira and your agents browse here. Open a page, or ask in the chat — you&apos;ll watch it live, and can click or type to take over.
          </p>
        </div>
        <button
          type="button"
          onClick={onStart}
          className="rounded-md border border-border/60 px-2.5 py-1 text-[12px] text-foreground transition-colors hover:border-border hover:bg-white/[0.05]"
        >
          Open
        </button>
      </div>
    );
  }
  return (
    <div className="flex h-full flex-col items-center justify-center gap-3 p-6 text-center">
      <span className="flex size-10 items-center justify-center rounded-xl border border-border/60 bg-white/[0.04] text-muted-foreground">
        <Globe className="size-5" />
      </span>
      <div>
        <p className="text-[13px] font-medium text-foreground">No browser open</p>
        <p className="mt-1 max-w-[30ch] text-[12px] text-muted-foreground">
          {native
            ? 'Opens a real embedded webview right here in the panel.'
            : 'Frames the page in the panel. Sites that block embedding fall back to the agent browser automatically.'}
        </p>
      </div>
      <button
        type="button"
        onClick={onStart}
        className="rounded-md border border-border/60 px-2.5 py-1 text-[12px] text-foreground transition-colors hover:border-border hover:bg-white/[0.05]"
      >
        Open
      </button>
    </div>
  );
}

function IconBtn({
  children,
  title,
  onClick,
  disabled,
}: {
  children: React.ReactNode;
  title: string;
  onClick: () => void;
  disabled?: boolean;
}) {
  return (
    <button
      type="button"
      title={title}
      onClick={onClick}
      disabled={disabled}
      className="flex size-6 shrink-0 items-center justify-center rounded text-muted-foreground transition-colors hover:bg-white/[0.05] hover:text-foreground disabled:opacity-30"
    >
      {children}
    </button>
  );
}
