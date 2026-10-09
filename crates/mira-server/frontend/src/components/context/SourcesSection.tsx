import { AnimatePresence, m } from 'framer-motion';
import {
  ArrowUpRight,
  BookMarked,
  FileCode2,
  GitBranch,
  GitCommitHorizontal,
  Globe2,
  ListChecks,
} from 'lucide-react';
import { useMemo, useState } from 'react';
import { prettyUrl } from '../../lib/refs';
import type { Entry } from '../../transcript/entries';
import { SectionHeader, itemVariants } from './SectionHeader';
export function extractSources(entries: Entry[]): string[] {
  const seen = new Set<string>();
  const urls: string[] = [];
  const re = /https?:\/\/[^\s<>"'*`()}\]]+/g;
  for (const e of entries) {
    if (e.kind !== 'msg' || e.msg.role !== 'assistant') continue;
    const text = e.msg.content ?? '';
    for (const m of text.matchAll(re)) {
      // Trailing sentence punctuation and markdown emphasis aren't part of
      // the link; `#` and `%` can be (fragments, escapes).
      const url = m[0].replace(/[.,;:!?*_`'"]+$/, '');
      if (!seen.has(url)) {
        seen.add(url);
        urls.push(url);
      }
    }
  }
  return urls;
}

/* ------------------------------------------------------------------ */
/* Sources                                                             */
/* ------------------------------------------------------------------ */

export const SOURCES_SHOWN = 8;

export const SOURCE_KIND_ICON = {
  branch: GitBranch,
  file: FileCode2,
  commit: GitCommitHorizontal,
  list: ListChecks,
  repo: BookMarked,
  page: Globe2,
} as const;

/** What a link is, as a title and a quieter second line: GitHub links by
 *  what they point at (repo, then branch / file / commit / list), other
 *  sites by name, then path. */
export function describeSource(url: string): {
  title: string;
  detail: string;
  kind: keyof typeof SOURCE_KIND_ICON;
  origin: string | null;
} {
  let origin: string | null = null;
  let host = url;
  try {
    const u = new URL(url);
    origin = u.origin;
    host = u.hostname.replace(/^www\./, '');
  } catch {
    /* not a URL: show it as is */
  }
  const pretty = prettyUrl(url);
  if (!pretty) return { title: url, detail: '', kind: 'page', origin };
  const [head, ...tail] = pretty.label.split(' · ');
  if (host === 'github.com') {
    const detail = tail.join(' · ');
    const what: Record<string, string> = {
      branch: 'Branch',
      file: 'File',
      commit: 'Commit',
      list: '',
      repo: 'Repository',
      page: '',
    };
    const label = pretty.kind === 'commit' ? (head.split('@')[1] ?? '') : detail;
    return {
      title: pretty.kind === 'commit' ? head.split('@')[0] : head,
      detail: capitalize([what[pretty.kind], label].filter(Boolean).join(' · ')) || 'GitHub',
      kind: pretty.kind,
      origin,
    };
  }
  const slash = pretty.label.indexOf('/');
  return {
    title: slash > 0 ? pretty.label.slice(0, slash) : pretty.label,
    detail: slash > 0 ? pretty.label.slice(slash + 1) : '',
    kind: pretty.kind,
    origin,
  };
}

export function capitalize(s: string): string {
  return s ? s[0].toUpperCase() + s.slice(1) : s;
}

export function SourceRow({ url }: { url: string }) {
  const { title, detail, kind, origin } = useMemo(() => describeSource(url), [url]);
  const [iconFailed, setIconFailed] = useState(false);
  const KindIcon = SOURCE_KIND_ICON[kind];
  return (
    <a
      href={url}
      target="_blank"
      rel="noopener noreferrer"
      title={url}
      className="group -mx-2 flex items-center gap-2.5 rounded-lg px-2 py-1.5 transition-colors hover:bg-fg/[0.05]"
    >
      {/* A light tile, so dark favicons (GitHub's) stay visible. */}
      <span className="grid size-6 shrink-0 place-items-center rounded-md bg-white shadow-[0_0_0_1px_rgb(var(--fg-rgb)/0.1)]">
        {origin && !iconFailed ? (
          <img
            src={`${origin}/favicon.ico`}
            alt=""
            referrerPolicy="no-referrer"
            className="size-3.5"
            onError={() => setIconFailed(true)}
          />
        ) : (
          <Globe2 className="size-3.5 text-neutral-500" />
        )}
      </span>
      <span className="min-w-0 flex-1">
        <span className="block truncate text-[12.5px] font-medium text-foreground/90 group-hover:text-foreground">
          {title}
        </span>
        {detail && (
          <span className="mt-px flex min-w-0 items-center gap-1 text-[11px] text-muted-foreground">
            <KindIcon className="size-3 shrink-0 opacity-70" />
            <span className="truncate">{detail}</span>
          </span>
        )}
      </span>
      <ArrowUpRight className="size-3.5 shrink-0 text-muted-foreground opacity-0 transition-opacity group-hover:opacity-100" />
    </a>
  );
}

export function SourcesSection({ sources }: { sources: string[] }) {
  const [open, setOpen] = useState(true);
  const [all, setAll] = useState(false);
  const shown = all ? sources : sources.slice(0, SOURCES_SHOWN);

  return (
    <div>
      <SectionHeader label="Sources" open={open} onToggle={() => setOpen((v) => !v)} />
      <AnimatePresence initial={false}>
        {open && (
          <m.div
            initial={{ height: 0, opacity: 0 }}
            animate={{ height: 'auto', opacity: 1, transition: { duration: 0.16 } }}
            exit={{ height: 0, opacity: 0, transition: { duration: 0.12 } }}
            // -mx-2 px-2: room for the rows' full-width, rounded hover.
            className="-mx-2 overflow-hidden px-2"
          >
            <div className="flex flex-col gap-0.5 pb-1">
              <AnimatePresence>
                {shown.map((url, i) => (
                  <m.div
                    key={url}
                    custom={i}
                    variants={itemVariants}
                    initial="hidden"
                    animate="visible"
                  >
                    <SourceRow url={url} />
                  </m.div>
                ))}
              </AnimatePresence>
              {sources.length > SOURCES_SHOWN && (
                <button
                  type="button"
                  onClick={() => setAll((v) => !v)}
                  className="self-start py-[3px] text-[11.5px] text-fg/35 transition-colors hover:text-fg/70"
                >
                  {all ? 'Show fewer' : `Show ${sources.length - SOURCES_SHOWN} more`}
                </button>
              )}
            </div>
          </m.div>
        )}
      </AnimatePresence>
    </div>
  );
}
