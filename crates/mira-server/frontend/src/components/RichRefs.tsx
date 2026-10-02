/**
 * Inline references in a reply, each drawn as what it is: a file you can
 * open at its line, a commit with its message, a color, a key combo, a
 * name you can jump to, a link with its preview. One visual family — a
 * quiet pill per kind, details on hover — so a dense answer stays
 * readable instead of turning into a wall of badges.
 */
import { useState } from 'react';
import { GitCommitHorizontal, Globe2, SquareFunction } from 'lucide-react';
import { HoverCard } from '@/components/ui/hover-card';
import { useFileIcons } from '@/lib/fileIcons';
import { cn } from '@/lib/utils';

export type OpenFile = (path: string, line?: number | null) => void;

/* ---------------------------------------------------------------- */
/* Shared fetch cache                                                */
/* ---------------------------------------------------------------- */

const cache = new Map<string, Promise<unknown>>();
function cached<T>(key: string, load: () => Promise<T>): Promise<T> {
  let p = cache.get(key) as Promise<T> | undefined;
  if (!p) {
    p = load();
    cache.set(key, p);
  }
  return p;
}
async function getJson<T>(url: string): Promise<T | null> {
  try {
    const r = await fetch(url);
    if (!r.ok || r.status === 204) return null;
    return (await r.json()) as T;
  } catch {
    return null;
  }
}

function ago(unixSecs: number): string {
  const s = Math.max(0, Date.now() / 1000 - unixSecs);
  if (s < 3600) return `${Math.max(1, Math.round(s / 60))}m ago`;
  if (s < 86400) return `${Math.round(s / 3600)}h ago`;
  if (s < 86400 * 30) return `${Math.round(s / 86400)}d ago`;
  return new Date(unixSecs * 1000).toLocaleDateString();
}

/* ---------------------------------------------------------------- */
/* File                                                              */
/* ---------------------------------------------------------------- */

export function FileChip({ path, line, onOpen }: { path: string; line: number | null; onOpen?: OpenFile }) {
  const { fileIcon } = useFileIcons();
  const slash = path.lastIndexOf('/');
  const dir = slash >= 0 ? path.slice(0, slash + 1) : '';
  const name = path.slice(slash + 1);
  const icon = fileIcon(name);
  const label = (
    <>
      {icon ? (
        <img src={icon} alt="" draggable={false} className="size-3.5 shrink-0 self-center" />
      ) : null}
      {dir && <span className="max-w-[18ch] truncate text-[#7f88b0]">{dir}</span>}
      <span className="text-[#c0caf5]">{name}</span>
      {line != null && <span className="text-[#e0af68]">:{line}</span>}
    </>
  );
  const cls =
    'md-ref inline-flex max-w-full items-baseline gap-1 rounded-md border border-[#7aa2f7]/20 bg-[#7aa2f7]/[0.08] px-1.5 py-px align-baseline font-mono text-[0.85em]';
  if (!onOpen) return <span className={cls} title={path}>{label}</span>;
  return (
    <button
      type="button"
      title={line != null ? `Open ${path} at line ${line}` : `Open ${path}`}
      onClick={() => onOpen(path, line)}
      className={cn(cls, 'cursor-pointer transition-colors hover:border-[#7aa2f7]/45 hover:bg-[#7aa2f7]/[0.14]')}
    >
      {label}
    </button>
  );
}

/* ---------------------------------------------------------------- */
/* Commit                                                            */
/* ---------------------------------------------------------------- */

type Commit = {
  sha: string;
  short: string;
  subject: string;
  body: string;
  author: string;
  date: number;
  files: { path: string; added: number | null; removed: number | null }[];
};

export function CommitChip({ sha }: { sha: string }) {
  const [commit, setCommit] = useState<Commit | null | undefined>(undefined);
  const load = () => {
    if (commit !== undefined) return;
    void cached(`commit:${sha}`, () => getJson<Commit>(`/api/git/commit/${sha}`)).then(setCommit);
  };
  const files = commit?.files ?? [];
  const shown = files.slice(0, 6);
  const max = Math.max(1, ...shown.map((f) => (f.added ?? 0) + (f.removed ?? 0)));
  return (
    <HoverCard
      onOpen={load}
      trigger={
        <span
          tabIndex={0}
          className="md-ref inline-flex items-baseline gap-1 rounded-md border border-[#e0af68]/25 bg-[#e0af68]/[0.08] px-1.5 py-px align-baseline font-mono text-[0.85em] text-[#e0af68] outline-none focus-visible:ring-1 focus-visible:ring-[#e0af68]/50"
        >
          <GitCommitHorizontal className="size-3.5 shrink-0 translate-y-[2px]" />
          {sha.slice(0, 7)}
        </span>
      }
    >
      {commit ? (
        <div>
          <div className="flex items-center gap-2 text-[11.5px] text-muted-foreground">
            <span className="font-mono text-[#e0af68]">{commit.short}</span>
            <span className="min-w-0 flex-1 truncate">{commit.author}</span>
            <span className="shrink-0">{ago(commit.date)}</span>
          </div>
          <div className="mt-1.5 text-[13.5px] font-semibold leading-snug text-foreground">{commit.subject}</div>
          {commit.body && (
            <div className="mt-1 line-clamp-3 whitespace-pre-line text-[12px] leading-relaxed text-muted-foreground">
              {commit.body}
            </div>
          )}
          {shown.length > 0 && (
            <div className="mt-2.5 space-y-1 border-t border-border/50 pt-2">
              {shown.map((f) => {
                const a = f.added ?? 0;
                const r = f.removed ?? 0;
                return (
                  <div key={f.path} className="flex items-center gap-2 font-mono text-[11px]">
                    <span className="min-w-0 flex-1 truncate text-foreground/75" title={f.path}>
                      {f.path}
                    </span>
                    <span className="shrink-0 tabular-nums text-emerald-400">+{a}</span>
                    <span className="shrink-0 tabular-nums text-red-400">−{r}</span>
                    {/* Proportion bar, GitHub-style. */}
                    <span className="flex h-1.5 w-12 shrink-0 overflow-hidden rounded-full bg-white/[0.06]">
                      <span className="bg-emerald-400/80" style={{ width: `${(a / max) * 100}%` }} />
                      <span className="bg-red-400/80" style={{ width: `${(r / max) * 100}%` }} />
                    </span>
                  </div>
                );
              })}
              {files.length > shown.length && (
                <div className="text-[11px] text-muted-foreground">and {files.length - shown.length} more</div>
              )}
            </div>
          )}
        </div>
      ) : commit === null ? (
        <div className="text-muted-foreground">Not a commit in this repository.</div>
      ) : null}
    </HoverCard>
  );
}

/* ---------------------------------------------------------------- */
/* Color                                                             */
/* ---------------------------------------------------------------- */

export function ColorChip({ color, children }: { color: string; children: React.ReactNode }) {
  return (
    <code className="md-inline inline-flex items-baseline gap-1.5">
      <span
        aria-hidden
        className="inline-block size-[0.8em] shrink-0 translate-y-[1px] rounded-[3px] ring-1 ring-white/25"
        // A checkerboard under the color, so translucent colors read as such.
        style={{
          background: `linear-gradient(${color}, ${color}), repeating-conic-gradient(#808080 0 25%, #c0c0c0 0 50%) 0 0 / 6px 6px`,
        }}
      />
      {children}
    </code>
  );
}

/* ---------------------------------------------------------------- */
/* Keys                                                              */
/* ---------------------------------------------------------------- */

export function KeysChip({ keys }: { keys: string[] }) {
  return (
    <span className="inline-flex items-baseline gap-0.5 align-baseline">
      {keys.map((k, i) => (
        <kbd key={i} className="md-key">
          {k}
        </kbd>
      ))}
    </span>
  );
}

/* ---------------------------------------------------------------- */
/* Symbol                                                            */
/* ---------------------------------------------------------------- */

type Hit = { path: string; line: number; preview: string };

/** A name in the code. Looked up on hover; becomes a link to its
 *  definition only if it has one, so plain identifiers stay quiet. */
export function SymbolRef({ name, onOpen, children }: { name: string; onOpen?: OpenFile; children: React.ReactNode }) {
  const [hits, setHits] = useState<Hit[] | undefined>(undefined);
  const load = () => {
    if (hits !== undefined || !onOpen) return;
    void cached(`symbol:${name}`, () => getJson<Hit[]>(`/api/symbol?name=${encodeURIComponent(name)}`)).then((h) =>
      setHits(h ?? []),
    );
  };
  const found = hits && hits.length > 0;
  return (
    <HoverCard
      onOpen={load}
      disabled={!found}
      trigger={
        <code
          className={cn(
            'md-inline',
            found && 'cursor-pointer underline decoration-dotted decoration-[#bb9af7]/60 underline-offset-[3px]',
          )}
          onClick={found ? () => onOpen?.(hits![0].path, hits![0].line) : undefined}
        >
          {children}
        </code>
      }
    >
      {found ? (
        <div>
          <div className="mb-1.5 flex items-center gap-1.5 text-[11.5px] text-muted-foreground">
            <SquareFunction className="size-3.5 text-[#bb9af7]" />
            {hits!.length === 1 ? 'Defined in' : `${hits!.length} definitions`}
          </div>
          <div className="space-y-1">
            {hits!.map((h) => (
              <button
                key={`${h.path}:${h.line}`}
                type="button"
                onClick={() => onOpen?.(h.path, h.line)}
                className="block w-full rounded-md px-1.5 py-1 text-left transition-colors hover:bg-white/[0.06]"
              >
                <div className="truncate font-mono text-[11px] text-[#7f88b0]">
                  {h.path}
                  <span className="text-[#e0af68]">:{h.line}</span>
                </div>
                <div className="truncate font-mono text-[11.5px] text-foreground/85">{h.preview}</div>
              </button>
            ))}
          </div>
        </div>
      ) : null}
    </HoverCard>
  );
}

/* ---------------------------------------------------------------- */
/* Link                                                              */
/* ---------------------------------------------------------------- */

type Preview = { url: string; title: string | null; description: string | null; image: string | null; site_name: string | null };

/** An external link: the site's icon in front, and on hover a preview from
 *  the page's own Open Graph tags. */
export function LinkRef({ href, children }: { href: string; children: React.ReactNode }) {
  const [preview, setPreview] = useState<Preview | null | undefined>(undefined);
  const [iconFailed, setIconFailed] = useState(false);
  let host = '';
  let origin = '';
  try {
    const u = new URL(href);
    host = u.hostname.replace(/^www\./, '');
    origin = u.origin;
  } catch {
    /* not absolute */
  }
  const load = () => {
    if (preview !== undefined || !origin) return;
    void cached(`unfurl:${href}`, () => getJson<Preview>(`/api/unfurl?url=${encodeURIComponent(href)}`)).then(setPreview);
  };
  return (
    <HoverCard
      onOpen={load}
      disabled={!origin}
      className="w-[21rem] overflow-hidden p-0"
      trigger={
        <a href={href} target="_blank" rel="noreferrer" className="md-link">
          {origin &&
            (iconFailed ? (
              <Globe2 className="mr-1 inline size-3.5 -translate-y-px text-[#7dcfff]/70" />
            ) : (
              <img
                src={`${origin}/favicon.ico`}
                alt=""
                referrerPolicy="no-referrer"
                onError={() => setIconFailed(true)}
                className="mr-1 inline size-3.5 -translate-y-px rounded-sm"
              />
            ))}
          {children}
        </a>
      }
    >
      {preview ? (
        <a href={href} target="_blank" rel="noreferrer" className="block no-underline">
          {preview.image && (
            <img
              src={preview.image}
              alt=""
              referrerPolicy="no-referrer"
              onError={(e) => (e.currentTarget.style.display = 'none')}
              className="aspect-[1.91/1] w-full border-b border-border/50 object-cover"
            />
          )}
          <div className="p-3">
            <div className="text-[11px] text-muted-foreground">{preview.site_name ?? host}</div>
            {preview.title && <div className="mt-0.5 line-clamp-2 text-[13.5px] font-semibold leading-snug text-foreground">{preview.title}</div>}
            {preview.description && (
              <div className="mt-1 line-clamp-3 text-[12px] leading-relaxed text-muted-foreground">{preview.description}</div>
            )}
          </div>
        </a>
      ) : null}
    </HoverCard>
  );
}
