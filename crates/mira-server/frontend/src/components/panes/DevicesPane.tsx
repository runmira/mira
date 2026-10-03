/**
 * Device preview: one page in several viewport sizes side by side — phone,
 * tablet, laptop — each scaled to fit the pane, reloaded together. For
 * checking a dev server's responsive layout without resizing anything.
 *
 * Frames are plain iframes at the device's CSS size, so this shows layout
 * (widths, breakpoints), not a phone's user agent or touch input. The URL
 * is shared with the Processes pane's "Preview" button.
 */
import { useEffect, useMemo, useRef, useState } from 'react';
import { ExternalLink, Laptop, Monitor, RotateCw, Smartphone, Tablet } from 'lucide-react';
import { openExternal } from '@/lib/desktop';
import { setPreviewUrl, usePreviewUrl } from '@/lib/previewUrl';
import { cn } from '@/lib/utils';
import { PaneBar, PaneEmpty, PaneIconButton } from './paneUi';

type Device = {
  id: string;
  label: string;
  w: number;
  h: number;
  kind: 'phone' | 'tablet' | 'laptop' | 'desktop';
};

const DEVICES: Device[] = [
  { id: 'iphone', label: 'iPhone 15', w: 393, h: 852, kind: 'phone' },
  { id: 'pixel', label: 'Pixel 8', w: 412, h: 915, kind: 'phone' },
  { id: 'ipad', label: 'iPad mini', w: 744, h: 1133, kind: 'tablet' },
  { id: 'laptop', label: 'Laptop', w: 1280, h: 800, kind: 'laptop' },
  { id: 'desktop', label: 'Desktop', w: 1440, h: 900, kind: 'desktop' },
];

const ICON = {
  phone: Smartphone,
  tablet: Tablet,
  laptop: Laptop,
  desktop: Monitor,
} as const;

const PICK_KEY = 'mira.devicePreviewDevices';
const DEFAULT_PICK = ['iphone', 'ipad', 'laptop'];

function loadPick(): string[] {
  try {
    const v = JSON.parse(localStorage.getItem(PICK_KEY) ?? 'null') as unknown;
    if (Array.isArray(v) && v.every((x) => typeof x === 'string') && v.length) return v as string[];
  } catch {
    /* ignore */
  }
  return DEFAULT_PICK;
}

type Port = { port: number; command: string };

export function DevicesPane() {
  const url = usePreviewUrl();
  const [draft, setDraft] = useState(url);
  const [pick, setPick] = useState<string[]>(loadPick);
  const [landscape, setLandscape] = useState(false);
  const [nonce, setNonce] = useState(0);
  const [blocked, setBlocked] = useState<string | null>(null);
  const [ports, setPorts] = useState<Port[]>([]);
  const stage = useRef<HTMLDivElement | null>(null);
  const [stageH, setStageH] = useState(400);

  useEffect(() => setDraft(url), [url]);

  useEffect(() => {
    try {
      localStorage.setItem(PICK_KEY, JSON.stringify(pick));
    } catch {
      /* ignore */
    }
  }, [pick]);

  // Fit the frames to the pane's height.
  useEffect(() => {
    const el = stage.current;
    if (!el) return;
    const ro = new ResizeObserver(() => setStageH(el.clientHeight));
    ro.observe(el);
    setStageH(el.clientHeight);
    return () => ro.disconnect();
  }, [url]);

  // A site that refuses framing shows a blank box; say why instead.
  useEffect(() => {
    setBlocked(null);
    if (!url) return;
    let live = true;
    fetch(`/api/browser/embeddable?url=${encodeURIComponent(url)}`)
      .then((r) => (r.ok ? r.json() : null))
      .then((j: { embeddable?: boolean; reason?: string } | null) => {
        if (live && j && j.embeddable === false) setBlocked(j.reason ?? 'This site doesn’t allow being shown in a frame.');
      })
      .catch(() => {});
    return () => {
      live = false;
    };
  }, [url, nonce]);

  // With no URL yet, offer what's listening.
  useEffect(() => {
    if (url) return;
    let live = true;
    fetch('/api/processes')
      .then((r) => r.json())
      .then((j: { ports?: Port[]; self_port?: number }) => {
        if (live) setPorts((j.ports ?? []).filter((p) => p.port !== j.self_port).slice(0, 8));
      })
      .catch(() => {});
    return () => {
      live = false;
    };
  }, [url]);

  const devices = useMemo(() => DEVICES.filter((d) => pick.includes(d.id)), [pick]);

  function go(raw: string) {
    let v = raw.trim();
    if (!v) return;
    if (/^\d+$/.test(v)) v = `http://localhost:${v}`;
    else if (!/^https?:\/\//.test(v)) v = `${/^(localhost|127\.|0\.0\.0\.0|\[)/.test(v) ? 'http' : 'https'}://${v}`;
    setPreviewUrl(v);
    setNonce((n) => n + 1);
  }

  function toggle(id: string) {
    setPick((p) => (p.includes(id) ? (p.length > 1 ? p.filter((x) => x !== id) : p) : DEVICES.map((d) => d.id).filter((x) => x === id || p.includes(x))));
  }

  // Label row + padding around each frame.
  const avail = Math.max(160, stageH - 64);

  return (
    <div className="flex h-full min-h-0 flex-col">
      <PaneBar>
        <form
          className="flex h-7 min-w-0 flex-1 items-center rounded-md border border-border/70 bg-background/60 px-2 focus-within:border-mira-blue/50"
          onSubmit={(e) => {
            e.preventDefault();
            go(draft);
          }}
        >
          <input
            value={draft}
            onChange={(e) => setDraft(e.target.value)}
            placeholder="localhost:5173"
            className="min-w-0 flex-1 bg-transparent font-mono text-[12px] text-foreground outline-none placeholder:text-muted-foreground/60"
          />
        </form>
        <PaneIconButton title="Reload all" onClick={() => setNonce((n) => n + 1)} disabled={!url}>
          <RotateCw className="size-3.5" />
        </PaneIconButton>
        <PaneIconButton title="Open in your browser" onClick={() => void openExternal(url)} disabled={!url}>
          <ExternalLink className="size-3.5" />
        </PaneIconButton>
      </PaneBar>
      <div className="flex shrink-0 flex-wrap items-center gap-1 border-b border-border/60 px-2.5 py-1.5">
        {DEVICES.map((d) => {
          const Icon = ICON[d.kind];
          const on = pick.includes(d.id);
          return (
            <button
              key={d.id}
              type="button"
              onClick={() => toggle(d.id)}
              title={`${d.label} · ${d.w}×${d.h}`}
              className={cn(
                'inline-flex h-6 items-center gap-1 rounded-md px-2 text-[11.5px] transition-colors',
                on ? 'bg-fg/[0.1] text-foreground' : 'text-muted-foreground hover:bg-fg/[0.05] hover:text-foreground',
              )}
            >
              <Icon className="size-3" />
              {d.label}
            </button>
          );
        })}
        <span className="flex-1" />
        <button
          type="button"
          onClick={() => setLandscape((v) => !v)}
          title="Rotate phones and tablets"
          className={cn(
            'inline-flex h-6 items-center gap-1 rounded-md px-2 text-[11.5px] transition-colors',
            landscape ? 'bg-fg/[0.1] text-foreground' : 'text-muted-foreground hover:bg-fg/[0.05] hover:text-foreground',
          )}
        >
          <Smartphone className={cn('size-3 transition-transform', landscape && 'rotate-90')} />
          {landscape ? 'Landscape' : 'Portrait'}
        </button>
      </div>

      <div ref={stage} className="min-h-0 flex-1 overflow-auto bg-fg/[0.025]">
        {!url ? (
          <PaneEmpty icon={<Smartphone className="size-4" />} title="Preview your app on devices">
            Enter your dev server’s address above, or press Preview on one in the Processes pane.
            {ports.length > 0 && (
              <span className="mt-3 flex flex-wrap justify-center gap-1.5">
                {ports.map((p) => (
                  <button
                    key={p.port}
                    type="button"
                    onClick={() => go(String(p.port))}
                    className="rounded-md border border-border/70 bg-background/60 px-2 py-1 font-mono text-[11.5px] text-foreground/90 hover:bg-fg/[0.05]"
                  >
                    :{p.port} <span className="font-sans text-muted-foreground">{p.command}</span>
                  </button>
                ))}
              </span>
            )}
          </PaneEmpty>
        ) : blocked ? (
          <PaneEmpty icon={<Monitor className="size-4" />} title="This page can’t be shown here">
            {blocked}
          </PaneEmpty>
        ) : (
          <div className="flex min-h-full w-max items-start gap-5 p-4">
            {devices.map((d) => {
              const rotate = landscape && (d.kind === 'phone' || d.kind === 'tablet');
              const w = rotate ? d.h : d.w;
              const h = rotate ? d.w : d.h;
              const bezel = d.kind === 'phone' ? 10 : d.kind === 'tablet' ? 12 : 6;
              const scale = Math.min(1, avail / (h + bezel * 2));
              return (
                <figure key={d.id} className="flex shrink-0 flex-col items-center gap-2">
                  <figcaption className="flex items-center gap-1.5 text-[11px] text-muted-foreground">
                    <span className="font-medium text-foreground/80">{d.label}</span>
                    <span className="tabular-nums text-muted-foreground/70">
                      {w}×{h}
                      {scale < 1 && ` · ${Math.round(scale * 100)}%`}
                    </span>
                  </figcaption>
                  <div style={{ width: (w + bezel * 2) * scale, height: (h + bezel * 2) * scale }}>
                    <div
                      className={cn(
                        'origin-top-left bg-neutral-900 shadow-lg shadow-shade/20 ring-1 ring-black/10 dark:bg-neutral-800 dark:ring-white/10',
                        d.kind === 'phone' ? 'rounded-[44px]' : d.kind === 'tablet' ? 'rounded-[28px]' : 'rounded-[10px]',
                      )}
                      style={{ width: w + bezel * 2, height: h + bezel * 2, padding: bezel, transform: `scale(${scale})` }}
                    >
                      <iframe
                        key={`${d.id}:${nonce}:${rotate}`}
                        src={url}
                        title={`${d.label} preview`}
                        width={w}
                        height={h}
                        className={cn(
                          'block bg-white',
                          d.kind === 'phone' ? 'rounded-[34px]' : d.kind === 'tablet' ? 'rounded-[16px]' : 'rounded-[5px]',
                        )}
                      />
                    </div>
                  </div>
                </figure>
              );
            })}
          </div>
        )}
      </div>
    </div>
  );
}
