import { useEffect, useMemo, useState, useRef } from 'react';
import {
  ChevronDown,
  ChevronRight,
  File,
  FileCode,
  FileCode2,
  FileJson,
  FileTerminal,
  FileText,
  Folder,
  FolderOpen,
  Loader,
  SquareArrowOutUpRight,
} from 'lucide-react';
import hljs from 'highlight.js';
import { parseDiffFromFile } from '@pierre/diffs';
import type { DiffLine, DiffPreview } from '../types';
import { browse } from '../api';
import { readFile } from '../api';
import { cn } from '@/lib/utils';
import { openInEditor } from '@/lib/editors';
import { useFileIcons } from '@/lib/fileIcons';
import { StyledDiffCodeView } from './diffs/StyledDiffCodeView';
import { useDiffOptions } from '../lib/diffPrefs';

export type FilePanelTab = {
  id: string;
  path: string;
  diff: DiffPreview | null;
  /** Line to reveal (1-based), from a `path:42` reference. */
  line?: number | null;
  /** Bumped on every open, so revealing the same line again scrolls again. */
  reveal?: number;
};

/* ------------------------------------------------------------------ */
/* Language detection                                                    */
/* ------------------------------------------------------------------ */

const EXT_TO_LANG: Record<string, string> = {
  ts: 'typescript', tsx: 'typescript', mts: 'typescript', cts: 'typescript',
  js: 'javascript', jsx: 'javascript', mjs: 'javascript', cjs: 'javascript',
  rs: 'rust', py: 'python', rb: 'ruby', go: 'go', java: 'java',
  c: 'c', cc: 'cpp', cpp: 'cpp', cxx: 'cpp', h: 'cpp', hpp: 'cpp',
  cs: 'csharp', swift: 'swift', kt: 'kotlin', kts: 'kotlin',
  dart: 'dart', scala: 'scala', hs: 'haskell',
  css: 'css', scss: 'scss', less: 'less',
  html: 'xml', htm: 'xml', xml: 'xml', svg: 'xml',
  json: 'json', json5: 'json', jsonc: 'json',
  yaml: 'yaml', yml: 'yaml', toml: 'ini',
  md: 'markdown', mdx: 'markdown',
  sh: 'bash', bash: 'bash', zsh: 'bash', fish: 'bash',
  ps1: 'powershell', bat: 'dos', cmd: 'dos',
  sql: 'sql', graphql: 'graphql', gql: 'graphql',
  lua: 'lua', vim: 'vim', ex: 'elixir', exs: 'elixir',
  ml: 'ocaml', mli: 'ocaml', clj: 'clojure', lisp: 'lisp',
  tf: 'hcl', hcl: 'hcl', proto: 'protobuf',
  r: 'r', jl: 'julia', nix: 'nix',
};

export function langFrom(path: string): string {
  const name = path.split('/').pop()?.toLowerCase() ?? '';
  if (name === 'dockerfile') return 'dockerfile';
  if (name === 'makefile' || name === 'gnumakefile') return 'makefile';
  const ext = name.split('.').pop() ?? '';
  return EXT_TO_LANG[ext] ?? 'plaintext';
}

function extFrom(path: string): string {
  const name = path.split('/').pop()?.toLowerCase() ?? '';
  return name.split('.').pop() ?? '';
}

/* ------------------------------------------------------------------ */
/* File icon — lucide icons tinted by language                          */
/* ------------------------------------------------------------------ */

type LangMeta = { color: string; Icon: typeof FileCode };

const LANG_META: Record<string, LangMeta> = {
  typescript:  { color: '#3178c6', Icon: FileCode2 },
  javascript:  { color: '#c8a400', Icon: FileCode2 },
  rust:        { color: '#e36e2a', Icon: FileCode },
  python:      { color: '#3572a5', Icon: FileCode },
  go:          { color: '#00acd7', Icon: FileCode },
  ruby:        { color: '#cc342d', Icon: FileCode },
  java:        { color: '#b07219', Icon: FileCode },
  kotlin:      { color: '#7f52ff', Icon: FileCode },
  swift:       { color: '#f05138', Icon: FileCode },
  css:         { color: '#563d7c', Icon: FileCode },
  scss:        { color: '#c6538c', Icon: FileCode },
  less:        { color: '#1d365d', Icon: FileCode },
  xml:         { color: '#e44d26', Icon: FileCode },
  json:        { color: '#8a9aa8', Icon: FileJson },
  yaml:        { color: '#cb171e', Icon: FileCode },
  markdown:    { color: '#6b9ce8', Icon: FileText },
  bash:        { color: '#4ec94e', Icon: FileTerminal },
  powershell:  { color: '#5391fe', Icon: FileTerminal },
  cpp:         { color: '#f34b7d', Icon: FileCode },
  c:           { color: '#a8b9cc', Icon: FileCode },
  csharp:      { color: '#178600', Icon: FileCode },
  dart:        { color: '#0175c2', Icon: FileCode },
  haskell:     { color: '#5e5086', Icon: FileCode },
  elixir:      { color: '#6e4a7e', Icon: FileCode },
  sql:         { color: '#e38c00', Icon: FileCode },
  graphql:     { color: '#e10098', Icon: FileCode },
  lua:         { color: '#7f8abe', Icon: FileCode },
  dockerfile:  { color: '#0db7ed', Icon: FileCode },
  nix:         { color: '#7e7eff', Icon: FileCode },
  hcl:         { color: '#844fba', Icon: FileCode },
};

/**
 * Per-file icon from the Material Icon Theme set. Falls back to the
 * hand-rolled lucide + colour map for anything the collection doesn't carry,
 * and to a plain glyph if even that is unknown.
 */
function FileTypeIcon({ path, className }: { path: string; className?: string }) {
  const { fileIcon } = useFileIcons();
  const dataUri = fileIcon(basename(path));

  if (dataUri) {
    return (
      <img
        src={dataUri}
        alt=""
        className={cn('shrink-0', className)}
        draggable={false}
      />
    );
  }

  const meta = LANG_META[langFrom(path)];
  if (!meta) return <File className={cn('shrink-0', className)} style={{ color: '#4a5568' }} />;
  const { Icon, color } = meta;
  return <Icon className={cn('shrink-0', className)} style={{ color }} />;
}

/** Last path segment, for icon lookup by file/folder name. */
function basename(p: string): string {
  return p.split('/').filter(Boolean).pop() ?? p;
}

/**
 * Themed folder icon, or the previous lucide glyph while the icon chunk is
 * still in flight.
 */
function folderIconFor(
  name: string,
  open: boolean,
  lookup: (name: string, open?: boolean) => string | null,
) {
  const dataUri = lookup(name, open);
  if (dataUri) {
    return (
      <img
        src={dataUri}
        alt=""
        className="size-[14px] shrink-0"
        draggable={false}
      />
    );
  }
  return open ? (
    <FolderOpen className="size-[14px] shrink-0" style={{ color: '#4da6ff' }} />
  ) : (
    <Folder className="size-[14px] shrink-0" style={{ color: '#4da6ff', opacity: 0.7 }} />
  );
}

/* ------------------------------------------------------------------ */
/* Breadcrumb (VS Code style with > separators)                         */
/* ------------------------------------------------------------------ */

function Breadcrumb({ path, cwd }: { path: string; cwd: string }) {
  const cwdName = cwd.split('/').filter(Boolean).pop() ?? '';
  const rel = path.startsWith(cwd + '/')
    ? path.slice(cwd.length + 1)
    : path.startsWith(cwd)
      ? path.slice(cwd.length)
      : path;
  const relParts = rel.split('/').filter(Boolean);
  if (relParts.length === 0) relParts.push(path.split('/').pop() ?? path);

  const allParts = cwdName ? [cwdName, ...relParts] : relParts;
  const fileName = allParts[allParts.length - 1];
  const dirParts = allParts.slice(0, -1);

  return (
    <div className="flex min-w-0 items-center overflow-hidden text-[12px]">
      {dirParts.map((part, i) => (
        <span key={i} className="flex shrink-0 items-center">
          <span className="text-muted-foreground/50">{part}</span>
          <ChevronRight className="mx-0.5 size-3 shrink-0 text-muted-foreground/30" />
        </span>
      ))}
      <span className="flex min-w-0 items-center gap-1.5 font-medium text-foreground">
        <FileTypeIcon path={path} className="size-3.5" />
        <span className="truncate">{fileName}</span>
      </span>
    </div>
  );
}

/* ------------------------------------------------------------------ */
/* File tree                                                             */
/* ------------------------------------------------------------------ */

type DirEntry = { name: string; path: string; isDir: boolean };
type DirState = { entries: DirEntry[]; expanded: boolean };

function FileTree({
  cwd,
  activePath,
  onSelect,
}: {
  cwd: string;
  activePath: string;
  onSelect: (path: string) => void;
}) {
  const [dirs, setDirs] = useState<Map<string, DirState>>(new Map());
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    setLoading(true);
    browse(cwd, false, true)
      .then((v) =>
        setDirs(
          new Map([[
            v.path,
            {
              entries: v.entries.map((e) => ({
                name: e.name,
                path: e.path,
                isDir: e.is_dir,
              })),
              expanded: true,
            },
          ]]),
        ),
      )
      .catch(() => {})
      .finally(() => setLoading(false));
  }, [cwd]);

  async function openDir(path: string) {
    const existing = dirs.get(path);
    if (existing) {
      setDirs((prev) => {
        const next = new Map(prev);
        next.set(path, { ...existing, expanded: !existing.expanded });
        return next;
      });
    } else {
      try {
        const v = await browse(path, false, true);
        setDirs((prev) => {
          const next = new Map(prev);
          next.set(path, {
            entries: v.entries.map((e) => ({
              name: e.name,
              path: e.path,
              isDir: e.is_dir,
            })),
            expanded: true,
          });
          return next;
        });
      } catch { /* ignore */ }
    }
  }

  if (loading) {
    return (
      <div className="flex items-center gap-1.5 px-3 py-2 text-[11.5px] text-muted-foreground">
        <Loader className="size-3 animate-spin" />
        <span>Loading…</span>
      </div>
    );
  }

  return (
    <div className="py-1">
      <DirNode
        path={cwd}
        depth={0}
        activePath={activePath}
        dirs={dirs}
        onOpenDir={openDir}
        onSelectFile={onSelect}
      />
    </div>
  );
}

function DirNode({
  path, depth, activePath, dirs, onOpenDir, onSelectFile,
}: {
  path: string;
  depth: number;
  activePath: string;
  dirs: Map<string, DirState>;
  onOpenDir: (path: string) => Promise<void>;
  onSelectFile: (path: string) => void;
}) {
  const state = dirs.get(path);
  if (!state?.expanded) return null;

  const sorted = [...state.entries].sort((a, b) => {
    if (a.isDir !== b.isDir) return a.isDir ? -1 : 1;
    return a.name.localeCompare(b.name);
  });

  const { folderIcon } = useFileIcons();

  return (
    <>
      {sorted.map((e) => {
        if (e.isDir) {
          const childState = dirs.get(e.path);
          const isOpen = childState?.expanded ?? false;
          return (
            <div key={e.path}>
              <button
                type="button"
                style={{ paddingLeft: depth * 14 + 6 }}
                className="group flex w-full items-center gap-1.5 py-[3px] pr-2 text-left text-[12px] text-muted-foreground/75 transition-colors hover:bg-fg/[0.04] hover:text-foreground"
                onClick={() => void onOpenDir(e.path)}
              >
                <span className="flex size-3 shrink-0 items-center justify-center text-muted-foreground/35">
                  {isOpen
                    ? <ChevronDown className="size-3" />
                    : <ChevronRight className="size-3" />}
                </span>
                {folderIconFor(basename(e.path), isOpen, folderIcon)}
                <span className="min-w-0 truncate">{e.name}</span>
              </button>
              {isOpen && (
                <DirNode
                  path={e.path}
                  depth={depth + 1}
                  activePath={activePath}
                  dirs={dirs}
                  onOpenDir={onOpenDir}
                  onSelectFile={onSelectFile}
                />
              )}
            </div>
          );
        }
        const isActive = e.path === activePath;
        return (
          <button
            key={e.path}
            type="button"
            style={{ paddingLeft: depth * 14 + 22 }}
            className={cn(
              'flex w-full items-center gap-1.5 py-[3px] pr-2 text-left text-[12px] transition-colors',
              isActive
                ? 'bg-fg/[0.07] text-foreground'
                : 'text-muted-foreground/65 hover:bg-fg/[0.04] hover:text-foreground',
            )}
            onClick={() => onSelectFile(e.path)}
            title={e.path}
          >
            <FileTypeIcon path={e.path} className="size-[14px]" />
            <span className="min-w-0 truncate">{e.name}</span>
          </button>
        );
      })}
    </>
  );
}

/* ------------------------------------------------------------------ */
/* Shared constants & hljs helper                                        */
/* ------------------------------------------------------------------ */

const BG  = 'var(--fp-code)';    /* atom-one-dark / light paper, per theme */
const TNG = 'var(--fp-gutter)';  /* gutter line-number color */

/** Split hljs HTML by newlines, closing and reopening any open spans at each
 *  boundary so every fragment is valid standalone HTML. This fixes multi-line
 *  constructs (block comments, strings) that hljs wraps in a single <span>
 *  spanning several newlines — naive .split('\n') would leave unclosed tags
 *  on every continuation line, making those lines render with no color. */
function splitHtmlLines(html: string): string[] {
  const result: string[] = [];
  const stack: string[] = [];
  let line = '';
  let i = 0;
  while (i < html.length) {
    if (html[i] === '\n') {
      for (let k = stack.length - 1; k >= 0; k--) line += '</span>';
      result.push(line);
      line = stack.join('');
      i++;
    } else if (html[i] === '<') {
      const end = html.indexOf('>', i);
      if (end < 0) { line += html.slice(i); break; }
      const tag = html.slice(i, end + 1);
      if (/^<\/span/i.test(tag)) { stack.pop(); line += tag; }
      else if (/^<span/i.test(tag)) { stack.push(tag); line += tag; }
      else { line += tag; }
      i = end + 1;
    } else {
      line += html[i++];
    }
  }
  if (line) result.push(line);
  return result;
}

/** Run hljs on `text` and return one valid HTML string per line. */
export function highlightLines(text: string, lang: string): string[] {
  let html: string;
  try {
    if (lang !== 'plaintext' && hljs.getLanguage(lang)) {
      html = hljs.highlight(text, { language: lang }).value;
    } else {
      html = hljs.highlightAuto(text).value;
    }
  } catch {
    html = text.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
  }
  return splitHtmlLines(html);
}

/* ------------------------------------------------------------------ */
/* Syntax-highlighted code viewer                                        */
/* ------------------------------------------------------------------ */

/** Code line height: text-[12.5px] × leading-[1.65]. */
const LINE_PX = 12.5 * 1.65;
/** The code area's top padding (py-4). */
const PAD_PX = 16;

function CodeViewer({ content, lang, line, reveal }: { content: string; lang: string; line?: number | null; reveal?: number }) {
  const hlLines = useMemo(() => highlightLines(content, lang), [content, lang]);
  const scroller = useRef<HTMLDivElement | null>(null);
  const target = line && line >= 1 && line <= hlLines.length ? line : null;

  // Bring the referenced line into view, a third of the way down so its
  // context shows above it.
  useEffect(() => {
    const el = scroller.current;
    if (!el || !target) return;
    const y = PAD_PX + (target - 1) * LINE_PX;
    el.scrollTo({ top: Math.max(0, y - el.clientHeight / 3), behavior: 'smooth' });
  }, [target, reveal, hlLines.length]);

  return (
    <div ref={scroller} className="h-full overflow-auto font-mono text-[12.5px] leading-[1.65]" style={{ background: BG }}>
      <div className="relative flex py-4">
        {target && (
          <div
            key={reveal}
            aria-hidden
            className="mira-line-flash pointer-events-none absolute inset-x-0"
            style={{
              top: PAD_PX + (target - 1) * LINE_PX,
              height: LINE_PX,
              background: 'rgba(122, 162, 247, 0.13)',
              boxShadow: 'inset 2px 0 0 #7aa2f7',
            }}
          />
        )}
        {/* Gutter — no border, just muted numbers on same dark bg */}
        <div
          className="sticky left-0 z-10 shrink-0 select-none pr-5 text-right"
          style={{ background: BG, color: TNG, minWidth: '3.5rem', paddingLeft: '1rem' }}
          aria-hidden
        >
          {hlLines.map((_, i) => (
            <div key={i} style={i + 1 === target ? { color: 'var(--fp-target)' } : undefined}>{i + 1}</div>
          ))}
        </div>
        {/* Code */}
        <pre className="min-w-0 flex-1 pr-8" style={{ background: BG, overflow: 'hidden' }}>
          <code
            className={`hljs${lang !== 'plaintext' ? ` language-${lang}` : ''}`}
            style={{ background: 'transparent', padding: 0 }}
            // eslint-disable-next-line react/no-danger
            dangerouslySetInnerHTML={{ __html: hlLines.join('\n') }}
          />
        </pre>
      </div>
    </div>
  );
}

/* ------------------------------------------------------------------ */
/* Diff viewer — syntax highlighting preserved, indicator bar on left   */
/* ------------------------------------------------------------------ */

/* ------------------------------------------------------------------ */
/* Diff viewer — Pierre CodeView surface                                 */
/* ------------------------------------------------------------------ */

function DiffViewer({ path, lines }: { path: string; lines: DiffLine[] }) {
  // Follows Settings → Appearance → Diffs.
  const diffOptions = useDiffOptions();
  /* Reconstruct old/new file contents from the preview lines, then let
   * Pierre diff them — the same FileDiffMetadata the review drawer renders. */
  const fileDiff = useMemo(() => {
    const oldText = lines
      .map((l) => (l.tag === 'ctx' || l.tag === 'del' ? l.text : null))
      .filter((t): t is string => t !== null)
      .join('\n');
    const newText = lines
      .map((l) => (l.tag === 'ctx' || l.tag === 'add' ? l.text : null))
      .filter((t): t is string => t !== null)
      .join('\n');
    try {
      return parseDiffFromFile(
        { name: path, contents: oldText },
        { name: path, contents: newText },
        { context: Infinity },
      );
    } catch {
      return null;
    }
  }, [path, lines]);

  if (!fileDiff) {
    return <div className="p-4 font-mono text-[12px] text-muted-foreground">No diff to show.</div>;
  }

  return (
    <StyledDiffCodeView
      className="h-full min-h-0 overflow-auto"
      items={[{ id: path, type: 'diff', fileDiff, collapsed: false }]}
      options={diffOptions}
      renderHeaderFilenameSuffix={() => null}
      renderHeaderPrefix={() => null}
    />
  );
}

/* ------------------------------------------------------------------ */
/* FilePanelBody                                                         */
/* ------------------------------------------------------------------ */

type Props = {
  tab: FilePanelTab;
  cwd: string;
  /** When provided, clicking a file in the explorer opens it as a new
   *  tab in the panel rather than replacing the current view. */
  onOpenFile?: (path: string, diff: DiffPreview | null) => void;
};

export function FilePanelBody({ tab, cwd, onOpenFile }: Props) {
  const { folderIcon } = useFileIcons();
  const [content, setContent] = useState<string | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [viewPath, setViewPath] = useState(tab.path);
  const [viewDiff, setViewDiff] = useState<DiffPreview | null>(tab.diff);
  // Explorer closed by default.
  const [treeVisible, setTreeVisible] = useState(false);

  // Follow the tab, including a diff handed to a tab that is already open:
  // re-opening a file from an edit (after it was opened from a read) used
  // to keep the plain file view, because only a new tab id re-synced.
  // eslint-disable-next-line react-hooks/exhaustive-deps
  useEffect(() => {
    setViewPath(tab.path);
    setViewDiff(tab.diff);
  }, [tab.id, tab.diff]);

  useEffect(() => {
    if (viewDiff) {
      setContent(null);
      setLoadError(null);
      return;
    }
    let cancelled = false;
    setLoading(true);
    setLoadError(null);
    readFile(viewPath)
      .then((v) => { if (!cancelled) setContent(v.content); })
      .catch((e) => { if (!cancelled) setLoadError(String((e as Error).message)); })
      .finally(() => { if (!cancelled) setLoading(false); });
    return () => { cancelled = true; };
  }, [viewPath, viewDiff]);

  const lang = langFrom(viewPath);
  const ext  = extFrom(viewPath);
  const meta = LANG_META[lang];

  function handleSelectFile(path: string) {
    if (onOpenFile) {
      onOpenFile(path, null);
    } else {
      setViewPath(path);
      setViewDiff(null);
    }
  }

  return (
    <div className="flex h-full min-w-0 flex-col overflow-hidden" style={{ background: BG }}>
      {/* VS Code-style header */}
      <div
        className="flex h-9 shrink-0 items-center gap-2 border-b px-3"
        style={{ background: 'var(--fp-header)', borderColor: 'var(--fp-border)' }}
      >
        <button
          type="button"
          title={treeVisible ? 'Hide explorer' : 'Show explorer'}
          onClick={() => setTreeVisible((v) => !v)}
          className="shrink-0 rounded p-1 transition-colors"
        >
          {folderIconFor('', treeVisible, folderIcon)}
        </button>

        <div className="min-w-0 flex-1 overflow-hidden">
          <Breadcrumb path={viewPath} cwd={cwd} />
        </div>

        {meta ? (
          <span
            className="shrink-0 rounded px-1.5 py-0.5 font-mono text-[10px] font-semibold leading-none"
            style={{ background: meta.color + '22', color: meta.color, border: `1px solid ${meta.color}44` }}
          >
            {ext === 'tsx' ? 'TSX' : ext === 'jsx' ? 'JSX' : lang}
          </span>
        ) : viewDiff ? (
          <span className="shrink-0 rounded bg-mira-blue/15 px-1.5 py-0.5 text-[10px] font-semibold text-mira-blue">
            {viewDiff.kind}
          </span>
        ) : null}
        <button
          type="button"
          title="Open in preferred editor (Settings → General)"
          onClick={() => {
            openInEditor(viewPath).catch(() => {
              setLoadError('Could not open in editor — pick one in Settings → General.');
            });
          }}
          className="shrink-0 rounded p-1 text-muted-foreground transition-colors hover:bg-fg/10 hover:text-foreground"
        >
          <SquareArrowOutUpRight className="size-3.5" />
        </button>
      </div>

      {/* Body */}
      <div className="flex min-h-0 flex-1 overflow-hidden">
        {treeVisible && (
          <div
            className="flex w-52 shrink-0 flex-col overflow-hidden border-r"
            style={{ background: 'var(--fp-tree)', borderColor: 'var(--fp-border)' }}
          >
            <div
              className="shrink-0 px-3 pb-1 pt-2.5 text-[9px] font-semibold uppercase tracking-[0.14em]"
              style={{ color: 'var(--fp-gutter)' }}
            >
              Explorer
            </div>
            <div className="min-h-0 flex-1 overflow-y-auto">
              <FileTree cwd={cwd} activePath={viewPath} onSelect={handleSelectFile} />
            </div>
          </div>
        )}

        <div className="min-h-0 min-w-0 flex-1">
          {viewDiff ? (
            <DiffViewer path={viewPath} lines={viewDiff.lines} />
          ) : loading ? (
            <div
              className="flex items-center gap-2 p-4 font-mono text-[12px]"
              style={{ color: TNG }}
            >
              <Loader className="size-3.5 animate-spin" />
              Loading…
            </div>
          ) : loadError ? (
            <div className="p-4 font-mono text-[12px] text-destructive">{loadError}</div>
          ) : content !== null ? (
            <CodeViewer content={content} lang={lang} line={tab.line} reveal={tab.reveal} />
          ) : null}
        </div>
      </div>
    </div>
  );
}
