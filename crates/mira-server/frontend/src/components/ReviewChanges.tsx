import { useCallback, useEffect, useState } from 'react';
import { ArrowCounterClockwise, CaretDown, ChatCircle, X } from '@phosphor-icons/react';
import { cn } from '@/lib/utils';
import { getSessionChanges, revertFile, type SessionChange } from '../api';

/**
 * "Review changes" drawer: every file this session changed, as a full
 * diff against HEAD. Revert a file, or click lines to leave comments and
 * send them all back to the agent as one message.
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

  const load = useCallback(() => {
    setError(null);
    getSessionChanges().then(setFiles).catch((e) => setError((e as Error).message));
  }, []);

  useEffect(() => {
    if (open) load();
  }, [open, load]);

  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => { if (e.key === 'Escape') onClose(); };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [open, onClose]);

  if (!open) return null;

  async function revert(path: string) {
    if (!window.confirm(`Discard this session's changes to ${path}?`)) return;
    try {
      await revertFile(path);
      setComments((prev) => new Map([...prev].filter(([, c]) => c.path !== path)));
      load();
      onChanged();
    } catch (e) {
      setError((e as Error).message);
    }
  }

  function send() {
    const list = [...comments.values()].filter((c) => c.text.trim());
    if (list.length === 0) return;
    const body = list
      .map((c) => `**${c.path}** line ${c.line}:\n> ${c.code || '(empty line)'}\n\n${c.text.trim()}`)
      .join('\n\n---\n\n');
    onSendComments(`Review comments on your changes:\n\n${body}`);
    setComments(new Map());
    onClose();
  }

  const count = [...comments.values()].filter((c) => c.text.trim()).length;

  return (
    <div className="fixed inset-0 z-50 flex justify-end">
      <div className="absolute inset-0 bg-black/40" onClick={onClose} />
      <aside className="relative flex h-full w-full max-w-[860px] animate-fade-in flex-col border-l border-border bg-background shadow-2xl">
        <header className="flex items-center gap-3 border-b border-border px-5 py-3">
          <span className="text-[14px] font-semibold">Review changes</span>
          {files && (
            <span className="text-[12px] text-muted-foreground">
              {files.length} file{files.length === 1 ? '' : 's'}
            </span>
          )}
          <button
            type="button"
            onClick={onClose}
            aria-label="Close"
            className="ml-auto rounded-md p-1 text-muted-foreground hover:bg-accent/50 hover:text-foreground"
          >
            <X className="size-4" />
          </button>
        </header>

        <div className="flex-1 overflow-y-auto px-5 py-4">
          {error && <div className="mb-3 text-[12.5px] text-destructive">{error}</div>}
          {files === null && !error && <div className="text-[13px] text-muted-foreground">Loading…</div>}
          {files?.length === 0 && (
            <div className="text-[13px] text-muted-foreground">This session has no uncommitted changes.</div>
          )}
          <div className="flex flex-col gap-4">
            {files?.map((f) => (
              <FileDiff
                key={f.path}
                file={f}
                comments={comments}
                setComments={setComments}
                onRevert={() => void revert(f.path)}
              />
            ))}
          </div>
        </div>

        <footer className="flex items-center gap-3 border-t border-border px-5 py-3">
          <span className="text-[12px] text-muted-foreground">
            Click a line to comment. Comments go to the agent as one message.
          </span>
          <button
            type="button"
            onClick={send}
            disabled={count === 0}
            className="ml-auto rounded-md bg-foreground px-3 py-1.5 text-[12.5px] font-medium text-background disabled:opacity-30"
          >
            Send {count > 0 ? `${count} ` : ''}comment{count === 1 ? '' : 's'}
          </button>
        </footer>
      </aside>
    </div>
  );
}

type ReviewComment = { path: string; line: number; code: string; text: string };

type DiffRow =
  | { kind: 'hunk'; text: string }
  | { kind: 'ctx' | 'add' | 'del'; text: string; oldNo: number | null; newNo: number | null };

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
  comments,
  setComments,
  onRevert,
}: {
  file: SessionChange;
  comments: Map<string, ReviewComment>;
  setComments: React.Dispatch<React.SetStateAction<Map<string, ReviewComment>>>;
  onRevert: () => void;
}) {
  const [open, setOpen] = useState(true);
  const rows = parseUnifiedDiff(file.diff);
  const adds = rows.filter((r) => r.kind === 'add').length;
  const dels = rows.filter((r) => r.kind === 'del').length;

  function toggleComment(i: number, r: Extract<DiffRow, { oldNo: number | null }>) {
    const key = `${file.path}#${i}`;
    setComments((prev) => {
      const next = new Map(prev);
      if (next.has(key) && !next.get(key)!.text.trim()) next.delete(key);
      else if (!next.has(key))
        next.set(key, { path: file.path, line: (r.newNo ?? r.oldNo)!, code: r.text.trim(), text: '' });
      return next;
    });
  }

  return (
    <section className="overflow-hidden rounded-lg border border-border">
      <div className="flex items-center gap-2 bg-secondary/50 px-3 py-2">
        <button type="button" onClick={() => setOpen((v) => !v)} className="flex min-w-0 flex-1 items-center gap-2 text-left">
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
        </button>
        <button
          type="button"
          onClick={onRevert}
          title="Discard this session's changes to the file"
          className="flex shrink-0 items-center gap-1 rounded-md px-2 py-1 text-[12px] text-muted-foreground hover:bg-accent/50 hover:text-foreground"
        >
          <ArrowCounterClockwise className="size-3.5" />
          Revert
        </button>
      </div>
      {open && (
        <div className="overflow-x-auto font-mono text-[12px] leading-[1.55]">
          {rows.length === 0 && <div className="px-3 py-2 text-muted-foreground">No textual diff (binary file?).</div>}
          {rows.map((r, i) => {
            if (r.kind === 'hunk') {
              return (
                <div key={i} className="bg-accent/30 px-3 py-0.5 text-muted-foreground/70">{r.text}</div>
              );
            }
            const key = `${file.path}#${i}`;
            const c = comments.get(key);
            return (
              <div key={i}>
                <div
                  onClick={() => toggleComment(i, r)}
                  className={cn(
                    'group flex cursor-pointer whitespace-pre hover:brightness-125',
                    r.kind === 'add' && 'bg-green-500/10',
                    r.kind === 'del' && 'bg-red-500/10',
                  )}
                >
                  <span className="w-10 shrink-0 select-none pr-2 text-right text-muted-foreground/40">{r.oldNo ?? ''}</span>
                  <span className="w-10 shrink-0 select-none pr-2 text-right text-muted-foreground/40">{r.newNo ?? ''}</span>
                  <span className={cn(
                    'w-4 shrink-0 select-none',
                    r.kind === 'add' ? 'text-green-400' : r.kind === 'del' ? 'text-red-400' : 'text-transparent',
                  )}>
                    {r.kind === 'add' ? '+' : r.kind === 'del' ? '−' : ' '}
                  </span>
                  <span className="pr-4">{r.text}</span>
                  <ChatCircle className="ml-auto mr-2 size-3.5 shrink-0 self-center text-muted-foreground opacity-0 group-hover:opacity-60" />
                </div>
                {c && (
                  <div className="border-y border-border bg-secondary/40 px-3 py-2 font-sans">
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
                )}
              </div>
            );
          })}
        </div>
      )}
    </section>
  );
}
