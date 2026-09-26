import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { ArrowCounterClockwise, CaretDown, ChatCircle, X } from '@phosphor-icons/react';
import { cn } from '@/lib/utils';
import { getSessionChanges, revertFile, type SessionChange } from '../api';
import { highlightLines, langFrom } from './FilePanel';

/**
 * "Review changes" drawer: every file this session changed, as a full
 * diff against HEAD. Revert files, or click lines (shift-click for a
 * range) to leave comments and send them all back to the agent as one
 * message.
 */
export function ReviewChanges({
  open,
  onClose,
  onSendComments,
  onChanged,
}: {
  open: boolean;
  onClose: () => void;
  /** Send the composed review message to the agent. */
  onSendComments: (text: string) => void;
  /** Files were reverted — refresh anything showing diff stats. */
  onChanged: () => void;
}) {
  const [files, setFiles] = useState<SessionChange[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [comments, setComments] = useState<Map<string, ReviewComment>>(new Map());
  const [collapsed, setCollapsed] = useState<Set<string>>(new Set());
  const [confirmAll, setConfirmAll] = useState(false);
  const sectionRefs = useRef(new Map<string, HTMLElement>());

  const load = useCallback(() => {
    setError(null);
    getSessionChanges().then(setFiles).catch((e) => setError((e as Error).message));
  }, []);

  useEffect(() => {
    if (open) {
      setConfirmAll(false);
      load();
    }
  }, [open, load]);

  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => { if (e.key === 'Escape') onClose(); };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [open, onClose]);

  const stats = useMemo(() => {
    const m = new Map<string, { adds: number; dels: number }>();
    for (const f of files ?? []) {
      const rows = parseUnifiedDiff(f.diff);
      m.set(f.path, {
        adds: rows.filter((r) => r.kind === 'add').length,
        dels: rows.filter((r) => r.kind === 'del').length,
      });
    }
    return m;
  }, [files]);

  if (!open) return null;

  async function revert(paths: string[]) {
    setError(null);
    try {
      for (const p of paths) await revertFile(p);
      setComments((prev) => new Map([...prev].filter(([, c]) => !paths.includes(c.path))));
      load();
      onChanged();
    } catch (e) {
      setError((e as Error).message);
      load();
      onChanged();
    }
  }

  function send() {
    const list = [...comments.values()].filter((c) => c.text.trim());
    if (list.length === 0) return;
    const body = list
      .map((c) => {
        const where = c.startLine === c.endLine ? `line ${c.startLine}` : `lines ${c.startLine}–${c.endLine}`;
        const quoted = (c.code || '(empty line)').split('\n').map((l) => `> ${l}`).join('\n');
        return `**${c.path}** ${where}:\n${quoted}\n\n${c.text.trim()}`;
      })
      .join('\n\n---\n\n');
    onSendComments(`Review comments on your changes:\n\n${body}`);
    setComments(new Map());
    onClose();
  }

  const count = [...comments.values()].filter((c) => c.text.trim()).length;
  const commentsIn = (path: string) => [...comments.values()].filter((c) => c.path === path).length;
  const totalAdds = [...stats.values()].reduce((a, s) => a + s.adds, 0);
  const totalDels = [...stats.values()].reduce((a, s) => a + s.dels, 0);
  const allCollapsed = !!files?.length && files.every((f) => collapsed.has(f.path));
  const showNav = (files?.length ?? 0) > 3;

  return (
    <div className="fixed inset-0 z-50 flex justify-end">
      <div className="absolute inset-0 bg-black/40" onClick={onClose} />
      <aside className="relative flex h-full w-full max-w-[1040px] animate-fade-in flex-col border-l border-border bg-background shadow-2xl">
        <header className="flex flex-wrap items-center gap-3 border-b border-border px-5 py-3">
          <span className="text-[14px] font-semibold">Review changes</span>
          {files && files.length > 0 && (
            <span className="text-[12px] text-muted-foreground">
              {files.length} file{files.length === 1 ? '' : 's'} ·{' '}
              <span className="font-mono text-green-400/80">+{totalAdds}</span>{' '}
              <span className="font-mono text-red-400/80">−{totalDels}</span>
            </span>
          )}
          <div className="ml-auto flex items-center gap-1">
            {files && files.length > 0 && (
              <>
                <HeaderButton
                  onClick={() => setCollapsed(allCollapsed ? new Set() : new Set(files.map((f) => f.path)))}
                >
                  {allCollapsed ? 'Expand all' : 'Collapse all'}
                </HeaderButton>
                {confirmAll ? (
                  <ConfirmInline
                    label={`Discard all ${files.length} files?`}
                    onConfirm={() => { setConfirmAll(false); void revert(files.map((f) => f.path)); }}
                    onCancel={() => setConfirmAll(false)}
                  />
                ) : (
                  <HeaderButton onClick={() => setConfirmAll(true)}>
                    <ArrowCounterClockwise className="size-3.5" /> Revert all
                  </HeaderButton>
                )}
              </>
            )}
            <button
              type="button"
              onClick={onClose}
              aria-label="Close"
              className="ml-1 rounded-md p-1 text-muted-foreground hover:bg-accent/50 hover:text-foreground"
            >
              <X className="size-4" />
            </button>
          </div>
        </header>

        <div className="flex min-h-0 flex-1">
          {showNav && files && (
            <nav className="hidden w-56 shrink-0 overflow-y-auto border-r border-border py-2 md:block">
              {files.map((f) => {
                const s = stats.get(f.path);
                const n = commentsIn(f.path);
                return (
                  <button
                    key={f.path}
                    type="button"
                    onClick={() => {
                      setCollapsed((prev) => { const next = new Set(prev); next.delete(f.path); return next; });
                      sectionRefs.current.get(f.path)?.scrollIntoView({ behavior: 'smooth', block: 'start' });
                    }}
                    title={f.path}
                    className="flex w-full items-center gap-2 px-3 py-1 text-left text-[12px] text-muted-foreground hover:bg-accent/40 hover:text-foreground"
                  >
                    <span className="min-w-0 flex-1 truncate font-mono">{f.path.split('/').pop()}</span>
                    {n > 0 && <ChatCircle weight="fill" className="size-3 shrink-0 text-mira-blue" />}
                    <span className="shrink-0 font-mono text-[10.5px]">
                      <span className="text-green-400/70">+{s?.adds}</span>{' '}
                      <span className="text-red-400/70">−{s?.dels}</span>
                    </span>
                  </button>
                );
              })}
            </nav>
          )}

          <div className="min-w-0 flex-1 overflow-y-auto px-5 py-4">
            {error && <div className="mb-3 text-[12.5px] text-destructive">{error}</div>}
            {files === null && !error && (
              <div className="flex flex-col gap-3">
                {[0, 1].map((i) => (
                  <div key={i} className="h-28 animate-pulse rounded-lg bg-secondary/50" />
                ))}
              </div>
            )}
            {files?.length === 0 && (
              <div className="py-16 text-center text-[13px] text-muted-foreground">
                This session has no uncommitted changes.
              </div>
            )}
            <div className="flex flex-col gap-4">
              {files?.map((f) => (
                <FileDiff
                  key={f.path}
                  file={f}
                  sectionRef={(el) => {
                    if (el) sectionRefs.current.set(f.path, el);
                    else sectionRefs.current.delete(f.path);
                  }}
                  open={!collapsed.has(f.path)}
                  onToggle={() =>
                    setCollapsed((prev) => {
                      const next = new Set(prev);
                      if (next.has(f.path)) next.delete(f.path); else next.add(f.path);
                      return next;
                    })
                  }
                  comments={comments}
                  setComments={setComments}
                  onRevert={() => void revert([f.path])}
                />
              ))}
            </div>
          </div>
        </div>

        <footer className="flex items-center gap-3 border-t border-border px-5 py-3">
          <span className="text-[12px] text-muted-foreground">
            Click a line to comment · shift-click for a range. Comments go to the agent as one message.
          </span>
          <button
            type="button"
            onClick={send}
            disabled={count === 0}
            className="ml-auto shrink-0 rounded-md bg-foreground px-3 py-1.5 text-[12.5px] font-medium text-background disabled:opacity-30"
          >
            Send {count > 0 ? `${count} ` : ''}comment{count === 1 ? '' : 's'}
          </button>
        </footer>
      </aside>
    </div>
  );
}

function HeaderButton({ onClick, children }: { onClick: () => void; children: React.ReactNode }) {
  return (
    <button
      type="button"
      onClick={onClick}
      className="flex items-center gap-1 rounded-md px-2 py-1 text-[12px] text-muted-foreground hover:bg-accent/50 hover:text-foreground"
    >
      {children}
    </button>
  );
}

/** Inline "are you sure" — replaces the button it came from. */
function ConfirmInline({
  label,
  onConfirm,
  onCancel,
}: {
  label: string;
  onConfirm: () => void;
  onCancel: () => void;
}) {
  return (
    <span className="flex items-center gap-1.5 text-[12px]">
      <span className="text-muted-foreground">{label}</span>
      <button
        type="button"
        onClick={onConfirm}
        className="rounded-md bg-destructive/90 px-2 py-0.5 font-medium text-white hover:bg-destructive"
      >
        Revert
      </button>
      <button
        type="button"
        onClick={onCancel}
        className="rounded-md px-2 py-0.5 text-muted-foreground hover:text-foreground"
      >
        Cancel
      </button>
    </span>
  );
}

type ReviewComment = {
  path: string;
  /** Row indices in the file's parsed diff (inclusive). */
  from: number;
  to: number;
  startLine: number;
  endLine: number;
  code: string;
  text: string;
};

type DiffRow =
  | { kind: 'hunk'; text: string }
  | { kind: 'ctx' | 'add' | 'del'; text: string; oldNo: number | null; newNo: number | null };

type CodeRow = Extract<DiffRow, { oldNo: number | null }>;

/** Parse a unified diff into renderable rows (file headers dropped). */
export function parseUnifiedDiff(diff: string): DiffRow[] {
  const rows: DiffRow[] = [];
  let oldNo = 0;
  let newNo = 0;
  let inHunk = false;
  for (const line of diff.split('\n')) {
    const h = /^@@ -(\d+)(?:,\d+)? \+(\d+)(?:,\d+)? @@(.*)$/.exec(line);
    if (h) {
      oldNo = Number(h[1]);
      newNo = Number(h[2]);
      inHunk = true;
      rows.push({ kind: 'hunk', text: line });
      continue;
    }
    if (!inHunk || line.startsWith('\\')) continue;
    if (line.startsWith('+')) rows.push({ kind: 'add', text: line.slice(1), oldNo: null, newNo: newNo++ });
    else if (line.startsWith('-')) rows.push({ kind: 'del', text: line.slice(1), oldNo: oldNo++, newNo: null });
    else if (line.startsWith(' ')) rows.push({ kind: 'ctx', text: line.slice(1), oldNo: oldNo++, newNo: newNo++ });
  }
  return rows;
}

function FileDiff({
  file,
  sectionRef,
  open,
  onToggle,
  comments,
  setComments,
  onRevert,
}: {
  file: SessionChange;
  sectionRef: (el: HTMLElement | null) => void;
  open: boolean;
  onToggle: () => void;
  comments: Map<string, ReviewComment>;
  setComments: React.Dispatch<React.SetStateAction<Map<string, ReviewComment>>>;
  onRevert: () => void;
}) {
  const [confirming, setConfirming] = useState(false);
  const anchor = useRef<number | null>(null);
  const rows = useMemo(() => parseUnifiedDiff(file.diff), [file.diff]);
  // Highlight the diff's lines as one block so multi-line constructs
  // (strings, comments) color correctly, then map back row by row.
  const html = useMemo(() => {
    const code = rows.filter((r) => r.kind !== 'hunk');
    const lines = highlightLines(code.map((r) => r.text).join('\n'), langFrom(file.path));
    const out = new Map<number, string>();
    let k = 0;
    rows.forEach((r, i) => { if (r.kind !== 'hunk') out.set(i, lines[k++] ?? ''); });
    return out;
  }, [rows, file.path]);

  const adds = rows.filter((r) => r.kind === 'add').length;
  const dels = rows.filter((r) => r.kind === 'del').length;
  const mine = [...comments.entries()].filter(([, c]) => c.path === file.path);
  const commented = new Set<number>();
  for (const [, c] of mine) for (let i = c.from; i <= c.to; i++) commented.add(i);

  function clickRow(i: number, shift: boolean) {
    let from = i;
    let to = i;
    if (shift && anchor.current != null) {
      from = Math.min(anchor.current, i);
      to = Math.max(anchor.current, i);
    }
    const start = anchor.current;
    anchor.current = i;
    const key = `${file.path}#${from}-${to}`;
    setComments((prev) => {
      const next = new Map(prev);
      // Extending a range replaces the untouched one-line comment the
      // first click opened.
      if (shift && start != null) {
        const single = `${file.path}#${start}-${start}`;
        if (single !== key && !next.get(single)?.text.trim()) next.delete(single);
      }
      const existing = next.get(key);
      if (existing) {
        // Clicking an untouched comment's line again closes it.
        if (!existing.text.trim()) next.delete(key);
        return next;
      }
      const span = rows.slice(from, to + 1).filter((r): r is CodeRow => r.kind !== 'hunk');
      if (span.length === 0) return prev;
      const lineNo = (r: CodeRow) => (r.newNo ?? r.oldNo)!;
      next.set(key, {
        path: file.path,
        from,
        to,
        startLine: lineNo(span[0]),
        endLine: lineNo(span[span.length - 1]),
        code: span.map((r) => r.text).join('\n'),
        text: '',
      });
      return next;
    });
  }

  return (
    <section ref={sectionRef} className="scroll-mt-2 rounded-lg border border-border">
      <div className="sticky top-0 z-10 flex items-center gap-2 rounded-t-lg border-b border-border bg-secondary px-3 py-2">
        <button type="button" onClick={onToggle} className="flex min-w-0 flex-1 items-center gap-2 text-left">
          <CaretDown className={cn('size-3 shrink-0 text-muted-foreground transition-transform', !open && '-rotate-90')} />
          <span className="truncate font-mono text-[12.5px]">{file.path}</span>
          {file.status !== 'modified' && (
            <span className="rounded bg-accent/60 px-1.5 text-[10.5px] uppercase tracking-wide text-muted-foreground">
              {file.status}
            </span>
          )}
          <span className="shrink-0 font-mono text-[11.5px]">
            <span className="text-green-400/80">+{adds}</span> <span className="text-red-400/80">−{dels}</span>
          </span>
          {mine.length > 0 && (
            <span className="flex shrink-0 items-center gap-1 rounded-full bg-mira-blue/15 px-1.5 text-[11px] text-mira-blue">
              <ChatCircle weight="fill" className="size-3" />
              {mine.length}
            </span>
          )}
        </button>
        {confirming ? (
          <ConfirmInline
            label="Discard changes?"
            onConfirm={() => { setConfirming(false); onRevert(); }}
            onCancel={() => setConfirming(false)}
          />
        ) : (
          <button
            type="button"
            onClick={() => setConfirming(true)}
            title="Discard this session's changes to the file"
            className="flex shrink-0 items-center gap-1 rounded-md px-2 py-1 text-[12px] text-muted-foreground hover:bg-accent/50 hover:text-foreground"
          >
            <ArrowCounterClockwise className="size-3.5" />
            Revert
          </button>
        )}
      </div>
      {open && (
        <div className="overflow-x-auto rounded-b-lg font-mono text-[12px] leading-[1.55]">
          {rows.length === 0 && <div className="px-3 py-2 text-muted-foreground">No textual diff (binary file?).</div>}
          {rows.map((r, i) => {
            if (r.kind === 'hunk') {
              return (
                <div key={i} className="bg-accent/30 px-3 py-0.5 text-muted-foreground/70">{r.text}</div>
              );
            }
            const ending = mine.filter(([, c]) => c.to === i);
            return (
              <div key={i}>
                <div
                  onClick={(e) => clickRow(i, e.shiftKey)}
                  className={cn(
                    'group flex cursor-pointer select-none whitespace-pre hover:brightness-125',
                    r.kind === 'add' && 'bg-green-500/10',
                    r.kind === 'del' && 'bg-red-500/10',
                    commented.has(i) && 'bg-mira-blue/[0.08]',
                  )}
                >
                  <span className="relative w-10 shrink-0 pr-2 text-right text-muted-foreground/40">
                    {commented.has(i) && (
                      <span className="absolute left-1 top-1/2 size-1.5 -translate-y-1/2 rounded-full bg-mira-blue" />
                    )}
                    {r.oldNo ?? ''}
                  </span>
                  <span className="w-10 shrink-0 pr-2 text-right text-muted-foreground/40">{r.newNo ?? ''}</span>
                  <span className={cn(
                    'w-4 shrink-0',
                    r.kind === 'add' ? 'text-green-400' : r.kind === 'del' ? 'text-red-400' : 'text-transparent',
                  )}>
                    {r.kind === 'add' ? '+' : r.kind === 'del' ? '−' : ' '}
                  </span>
                  <span className="hljs !bg-transparent !p-0 pr-4" dangerouslySetInnerHTML={{ __html: html.get(i) ?? '' }} />
                  <ChatCircle className="ml-auto mr-2 size-3.5 shrink-0 self-center text-muted-foreground opacity-0 group-hover:opacity-60" />
                </div>
                {ending.map(([key, c]) => (
                  <div key={key} className="border-y border-border bg-secondary/40 px-3 py-2 font-sans">
                    {c.startLine !== c.endLine && (
                      <div className="mb-1 text-[11px] text-muted-foreground">
                        Lines {c.startLine}–{c.endLine}
                      </div>
                    )}
                    <textarea
                      autoFocus
                      value={c.text}
                      onChange={(e) =>
                        setComments((prev) => new Map(prev).set(key, { ...c, text: e.target.value }))
                      }
                      placeholder="Comment for the agent…"
                      rows={2}
                      className="w-full resize-none rounded-md border border-border bg-background px-2 py-1.5 text-[13px] outline-none focus:border-foreground/30"
                    />
                    <button
                      type="button"
                      onClick={() => setComments((prev) => { const n = new Map(prev); n.delete(key); return n; })}
                      className="mt-1 text-[11.5px] text-muted-foreground hover:text-foreground"
                    >
                      Remove
                    </button>
                  </div>
                ))}
              </div>
            );
          })}
        </div>
      )}
    </section>
  );
}
