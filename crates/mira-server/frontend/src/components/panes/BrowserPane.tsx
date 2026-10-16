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
  MoreHorizontal,
  Square,
  SquareDashedMousePointer,
  X,
} from 'lucide-react';
import { cn } from '@/lib/utils';
import {
  attachChildWebview,
  childWebviewAvailable,
  trackElement,
  type ChildWebview,
} from '@/lib/childWebview';
import { Menu, MenuItem, MenuSeparator } from '@/components/ui/Menu';
import { pageIcon, setBrowserPage } from '@/lib/browserPage';
import { RoundBtn, UrlCapsule } from './BrowserChrome';
import { AgentCursor, AnnotateHint, AnnotationLayer, FittedFrame, type AgentPointer } from './BrowserLiveOverlay';

type Shot = { png_base64: string; width: number; height: number };
type ActionResult = { text: string; screenshot?: Shot };

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
  const [snapshot, setSnapshot] = useState<string | null>(null);
  const [busy, setBusy] = useState<null | string>(null);
  const [error, setError] = useState<string | null>(null);
  const inFlight = useRef(false);
  /** Where the agent is acting, for its cursor in the live view. */
  const [pointer, setPointer] = useState<AgentPointer | null>(null);
  const pointerSeq = useRef(0);
  const [annotating, setAnnotating] = useState(false);
  const [more, setMore] = useState(false);
  /** The page's icon, for the address capsule and the panel tab. */
  const [icon, setIcon] = useState<string | null>(null);
  useEffect(() => {
    if (!url) { setIcon(null); return; }
    let stale = false;
    // Give the page a moment to put its <link rel=icon> in place.
    const t = setTimeout(() => void pageIcon(url).then((i) => { if (!stale) setIcon(i); }), 700);
    return () => { stale = true; clearTimeout(t); };
  }, [url]);
  useEffect(() => setBrowserPage({ url, icon }), [url, icon]);
  useEffect(() => () => setBrowserPage({ url: '', icon: null }), []);
  /** The how-to under the Annotate button, shown when the mode turns on
   *  until the first mark (or a few seconds). */
  const [annotateHint, setAnnotateHint] = useState(false);
  useEffect(() => {
    setAnnotateHint(annotating);
    if (!annotating) return;
    const t = setTimeout(() => setAnnotateHint(false), 6000);
    return () => clearTimeout(t);
  }, [annotating]);
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
      let ev: { type: string; jpeg_base64?: string; width?: number; height?: number; url?: string; x?: number; y?: number; action?: string };
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
      } else if (ev.type === 'pointer' && ev.x !== undefined && ev.y !== undefined) {
        setPointer({ x: ev.x, y: ev.y, action: ev.action ?? 'click', seq: ++pointerSeq.current });
      } else if (ev.type === 'closed') {
        setFrame(null);
        setPointer(null);
        setAnnotating(false);
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

  // The page takes the pane's own size: no device presets, it simply
  // reflows as the pane is resized, at the screen's pixel density.
  const bodyRef = useRef<HTMLDivElement | null>(null);
  useEffect(() => {
    const el = bodyRef.current;
    if (mode !== 'cdp' || !el) return;
    let t: ReturnType<typeof setTimeout> | undefined;
    const report = () => {
      clearTimeout(t);
      t = setTimeout(() => {
        const { width, height } = el.getBoundingClientRect();
        if (width < 50 || height < 50) return;
        send({
          type: 'resize',
          width: Math.round(width),
          height: Math.round(height),
          scale: window.devicePixelRatio || 1,
        });
      }, 150);
    };
    const ro = new ResizeObserver(report);
    ro.observe(el);
    report();
    // Moving the window to a screen with a different pixel density doesn't
    // resize the pane, but the frames must follow it or they blur.
    let dpr: MediaQueryList | null = null;
    const watchDpr = () => {
      dpr?.removeEventListener('change', onDpr);
      dpr = window.matchMedia(`(resolution: ${window.devicePixelRatio}dppx)`);
      dpr.addEventListener('change', onDpr);
    };
    function onDpr() {
      report();
      watchDpr();
    }
    watchDpr();
    return () => {
      clearTimeout(t);
      ro.disconnect();
      dpr?.removeEventListener('change', onDpr);
    };
  }, [mode, send]);

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
      // The server's shape is `{ embeddable, reason, final_url }`.
      const j = (await res.json()) as { embeddable?: boolean; reason?: string; final_url?: string };
      return {
        ok: j.embeddable !== false,
        reason: j.reason ?? '',
        finalUrl: j.final_url || target,
      };
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

  const showSnapshot = snapshot !== null;

  return (
    <div className="flex h-full min-h-0 flex-col">
      {/* Header — outside the native view's rect on purpose. */}
      <div className="flex shrink-0 items-center gap-1 border-b border-border/60 px-2 py-1.5">
        <RoundBtn title="Back" onClick={() => void go('back')} disabled={!url && !frame}>
          <ArrowLeft className="size-3.5" />
        </RoundBtn>
        <RoundBtn title="Forward" onClick={() => void go('forward')} disabled={!url && !frame}>
          <ArrowRight className="size-3.5" />
        </RoundBtn>
        <RoundBtn title="Reload" onClick={() => void reload()} disabled={!url && !frame}>
          <RefreshCw className="size-3.5" />
        </RoundBtn>
        <span className="relative ml-0.5 shrink-0">
          <button
            type="button"
            title={mode === 'cdp' ? 'Annotate the page: notes go to your message' : 'Annotations work in the agent browser'}
            aria-pressed={annotating}
            disabled={mode !== 'cdp' || !frame || showSnapshot}
            onClick={() => setAnnotating((a) => !a)}
            className={cn(
              'flex h-7 shrink-0 items-center gap-1.5 rounded-full border px-2.5 text-[12px] font-medium transition-colors disabled:opacity-30',
              annotating
                ? 'border-mira-blue/70 bg-mira-blue/10 text-mira-blue'
                : 'border-border/70 bg-card text-foreground/80 hover:border-border hover:text-foreground',
            )}
          >
            <SquareDashedMousePointer className="size-3.5" />
            Annotate
          </button>
          {annotateHint && <AnnotateHint />}
        </span>
        <span className="w-1 shrink-0" />
        <UrlCapsule
          url={url}
          icon={icon}
          live={mode === 'cdp' ? live && !!frame : !!url}
          onNavigate={(target) => void navigate(target)}
        />
        <span className="relative shrink-0">
          <RoundBtn title="More" onClick={() => setMore((m) => !m)} active={more}>
            <MoreHorizontal className="size-3.5" />
          </RoundBtn>
          <Menu open={more} onOpenChange={setMore} label="Browser actions" minWidth={220}>
            <MenuItem
              label="Open in your browser"
              icon={<ExternalLink className="size-3.5" />}
              disabled={!url}
              onSelect={() => url && window.open(url, '_blank', 'noopener,noreferrer')}
            />
            <MenuItem
              label={showSnapshot ? 'Back to the page' : 'Show page text'}
              icon={showSnapshot ? <X className="size-3.5" /> : <FileText className="size-3.5" />}
              disabled={!url}
              onSelect={() => (showSnapshot ? setSnapshot(null) : void takeSnapshot())}
            />
            {mode === 'cdp' ? (
              <MenuItem
                label="Embed this page instead"
                hint="Frames the page, for sites that allow it"
                icon={<Globe className="size-3.5" />}
                disabled={!url}
                onSelect={() => {
                  setMode('iframe');
                  setError(null);
                  setFrameKey((k) => k + 1);
                }}
              />
            ) : (
              <MenuItem
                label="Show the agent browser"
                hint="The live browser Mira and your agents use"
                icon={<Camera className="size-3.5" />}
                onSelect={() => {
                  setMode('cdp');
                  setError(null);
                  if (url) void run('loading', { action: 'navigate', url });
                }}
              />
            )}
            <MenuSeparator />
            <MenuItem
              label="Close the browser"
              icon={<Square className="size-3" />}
              disabled={!url}
              onSelect={() => void shutdown()}
            />
          </Menu>
        </span>
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
      <div ref={bodyRef} className="min-h-0 flex-1 overflow-hidden bg-shade/30">
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
              <FittedFrame w={frame.w} h={frame.h}>
              <img
                src={frame.src}
                alt={url ? `Live view of ${url}` : 'Live view of the agent browser'}
                draggable={false}
                className="size-full cursor-default select-none"
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
              <AgentCursor pointer={pointer} w={frame.w} h={frame.h} />
              {annotating && (
                <AnnotationLayer
                  key={url}
                  frame={frame}
                  url={url}
                  onExit={() => setAnnotating(false)}
                  onMark={() => setAnnotateHint(false)}
                />
              )}
              </FittedFrame>
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
        <span className="flex size-10 items-center justify-center rounded-xl border border-border/60 bg-fg/[0.04] text-muted-foreground">
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
          className="rounded-md border border-border/60 px-2.5 py-1 text-[12px] text-foreground transition-colors hover:border-border hover:bg-fg/[0.05]"
        >
          Open
        </button>
      </div>
    );
  }
  return (
    <div className="flex h-full flex-col items-center justify-center gap-3 p-6 text-center">
      <span className="flex size-10 items-center justify-center rounded-xl border border-border/60 bg-fg/[0.04] text-muted-foreground">
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
        className="rounded-md border border-border/60 px-2.5 py-1 text-[12px] text-foreground transition-colors hover:border-border hover:bg-fg/[0.05]"
      >
        Open
      </button>
    </div>
  );
}
