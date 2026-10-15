import { useSelector } from '@tanstack/react-store';
import { GuidedWalkthrough } from './GuidedWalkthrough';
import { useWorkspaceUiField, useAppRuntime } from '../state/RuntimeProvider';
/**
 * "Review changes" drawer: every file this session changed, rendered with
 * the Pierre CodeView diff surface, with a file tree, per-file collapse,
 * revert, and inline line comments sent back to the agent as one message.
 */
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { MessageCircle, RotateCcw, ChevronDown, X } from 'lucide-react';
import { parseDiffFromFile, type FileDiffMetadata } from '@pierre/diffs';
import { cn } from '@/lib/utils';
import { getTurnFileDiff, getSessionChanges, revertFile, type SessionChange } from '../api';
import {
  buildFileDiffContentVersion,
  buildFileDiffIdentityKey,
  getDiffLineStat,
  getRenderablePatch,
  resolveFileDiffPath,
} from '@/lib/diffRender';
import type { MiraReviewComment } from '@/lib/reviewComment';
import { DiffFileTree, type DiffFileTreeEntry } from './diffs/DiffFileTree';
import { diffFileTreeEntries } from './diffs/diffFileTree.logic';
import { DiffStatLabel } from './diffs/DiffStatLabel';
import { ReviewDiffView, type ReviewDiffFile, type ReviewDiffViewHandle } from './diffs/ReviewDiffView';
import { ReviewSourceSelector, ReviewDisplayActions } from './diffs/ReviewToolbar';
import { useDiffOptions } from '../lib/diffPrefs';

export function ReviewChanges({
  open,
  embedded = false,
  onClose,
  onSendComments,
  onChanged,
  focusPath = null,
}: {
  open: boolean;
  embedded?: boolean;
  /** Open scrolled to this file (a path relative to the session cwd). */
  focusPath?: string | null;
  onClose: () => void;
  /** Send the composed review message to the agent. */
  onSendComments: (text: string) => void;
  /** Files were reverted — refresh anything showing diff stats. */
  onChanged: () => void;
}) {

  const [turnScope, setTurnScope] = useWorkspaceUiField('reviewTurn');
  const [turnUnavailable, setTurnUnavailable] = useState<{ path: string; error: string }[]>([]);
  const [turnFiles, setTurnFiles] = useState<FileDiffMetadata[]>([]);
  const loadGeneration = useRef(0);
  const [guided, setGuided] = useState(false);
  const requestRuntime = useAppRuntime();
  const owner = useSelector(requestRuntime.sessionState, state => state.active ?? state.selected);
  const currentOwner = useRef(owner);
  currentOwner.current = owner;
  const requestSession = () => owner ?? undefined;
  const [files, setFiles] = useState<SessionChange[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [comments, setComments] = useState<MiraReviewComment[]>([]);
  const [collapsed, setCollapsed] = useState<Set<string>>(new Set());
  const [confirmAll, setConfirmAll] = useState(false);
  const [selectedPath, setSelectedPath] = useState<string | null>(null);
  const [revealRequestId, setRevealRequestId] = useState(0);
  const [viewer, setViewer] = useState<ReviewDiffViewHandle | null>(null);
  // Diff layout (Settings → General → Diff).
  const diffOptions = useDiffOptions();
  const containerRef = useRef<HTMLDivElement>(null);
  const [paneWidth, setPaneWidth] = useState(0);
  useEffect(() => {
    const element = containerRef.current;
    if (!open || !element) return;
    const measure = () => { if (element.clientWidth > 0) setPaneWidth(element.clientWidth); };
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(element);
    return () => observer.disconnect();
  }, [open, embedded]);
  const narrow = embedded && paneWidth < 600;
  const displayOptions = useMemo(() => narrow ? { ...diffOptions, diffStyle: 'unified' as const } : diffOptions, [diffOptions, narrow]);

  const load = useCallback(() => {
    const generation = ++loadGeneration.current;
    setError(null); setFiles(null); setTurnFiles([]); setTurnUnavailable([]);
    if (turnScope && turnScope.session !== owner) { setTurnScope(null); return; }
    if (turnScope) {
      const { session, summary } = turnScope;
      Promise.allSettled(summary.files.map(async file => {
        if (file.binary) throw new Error('Binary file · no text diff available.');
        const diff = await getTurnFileDiff(session, summary.text, summary.occurrence, file.path);
        return parseDiffFromFile({ name: file.path, contents: diff.before }, { name: file.path, contents: diff.after });
      })).then(parsed => {
        if (generation !== loadGeneration.current) return;
        setTurnFiles(parsed.flatMap(result => result.status === 'fulfilled' ? [result.value] : []));
        setTurnUnavailable(parsed.flatMap((result, index) => result.status === 'rejected' ? [{ path: summary.files[index].path, error: String(result.reason?.message ?? result.reason) }] : []));
        setFiles(summary.files.map(file => ({ ...file, status: 'modified', diff: '' })));
      }).catch(e => { if (generation === loadGeneration.current) setError((e as Error).message); });
    } else {
      getSessionChanges(requestSession()).then(value => {
        if (generation === loadGeneration.current) setFiles(value);
      }).catch(e => { if (generation === loadGeneration.current) setError((e as Error).message); });
    }
  }, [turnScope, owner]);

  useEffect(() => {
    setGuided(false); setComments([]); setCollapsed(new Set()); setSelectedPath(null);
  }, [owner, turnScope]);

  useEffect(() => {
    if (open) { setConfirmAll(false); load(); }
    else { loadGeneration.current++; if (turnScope) setTurnScope(null); }
    return () => { loadGeneration.current++; };
  }, [open, load]);

  useEffect(() => {
    if (!open || embedded) return;
    const onKey = (e: KeyboardEvent) => { if (e.key === 'Escape') onClose(); };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [open, onClose, embedded]);

  /** Parse every session file's unified diff into Pierre file entries (diff order kept). */
  const reviewFiles: ReviewDiffFile[] = useMemo(() => {
    const out: ReviewDiffFile[] = [];
    for (const f of files ?? []) {
      const parsed = turnScope ? null : getRenderablePatch(f.diff, 'mira-review');
      const diffs = turnScope ? turnFiles.filter(file => resolveFileDiffPath(file) === f.path) : parsed?.kind === 'files' ? parsed.files : [];
      for (const fileDiff of diffs) {
        const fileKey = buildFileDiffIdentityKey(fileDiff);
        out.push({
          fileDiff,
          filePath: resolveFileDiffPath(fileDiff) || f.path,
          fileKey,
          fileVersion: buildFileDiffContentVersion(fileDiff),
          collapsed: collapsed.has(fileKey),
        });
      }
    }
    return out;
  }, [files, collapsed, turnScope, turnFiles]);

  const treeEntries: ReadonlyArray<DiffFileTreeEntry> = useMemo(
    () => diffFileTreeEntries(reviewFiles.map((f) => f.fileDiff)),
    [reviewFiles],
  );

  const stat = useMemo(
    () => getDiffLineStat(reviewFiles.map((f) => f.fileDiff)),
    [reviewFiles],
  );

  const fileByKey = useMemo(() => new Map(reviewFiles.map((f) => [f.fileKey, f])), [reviewFiles]);

  // Jump to the requested file once its diff has loaded. Keyed on the
  // request, so reopening on the same file jumps again.
  const focusedFor = useRef<string | null>(null);
  useEffect(() => {
    if (!open) {
      focusedFor.current = null;
      return;
    }
    if (!focusPath || !viewer || focusedFor.current === focusPath) return;
    const target = reviewFiles.find((f) => f.filePath === focusPath || f.filePath.endsWith(`/${focusPath}`));
    if (!target) return;
    focusedFor.current = focusPath;
    setSelectedPath(target.filePath);
    setRevealRequestId((n) => n + 1);
    setCollapsed((prev) => {
      if (!prev.has(target.fileKey)) return prev;
      const next = new Set(prev);
      next.delete(target.fileKey);
      return next;
    });
    viewer.scrollTo({ type: 'item', id: target.fileKey, align: 'start' });
  }, [open, focusPath, viewer, reviewFiles]);

  if (!open) return null;

  async function revert(paths: string[]) {
    if (turnScope) return;
    setError(null);
    const mutationOwner = requestSession();
    try {
      for (const p of paths) await revertFile(p, mutationOwner);
      if ((currentOwner.current ?? undefined) !== mutationOwner) return;
      setComments((prev) => prev.filter((c) => !paths.includes(c.filePath)));
      load();
      onChanged();
    } catch (e) {
      if ((currentOwner.current ?? undefined) !== mutationOwner) return;
      setError((e as Error).message);
      load();
      onChanged();
    }
  }

  function send() {
    const list = comments.filter((c) => c.text.trim());
    if (list.length === 0) return;
    const body = list
      .map((c) => {
        const quoted = c.code.split('\n').map((l) => `> ${l}`).join('\n');
        return `**${c.filePath}** ${c.rangeLabel}:\n${quoted}\n\n${c.text.trim()}`;
      })
      .join('\n\n---\n\n');
    onSendComments(`Review comments on your changes:\n\n${body}`);
    setComments([]);
    onClose();
  }

  const count = comments.filter((c) => c.text.trim()).length;
  const commentsIn = (fileKey: string) => comments.filter((c) => c.fileKey === fileKey).length;
  const allCollapsed = reviewFiles.length > 0 && reviewFiles.every((f) => collapsed.has(f.fileKey));
  const showNav = reviewFiles.length > 1;

  function selectTreeFile(path: string) {
    setSelectedPath(path);
    setRevealRequestId((n) => n + 1);
    const target = reviewFiles.find((f) => f.filePath === path);
    if (target) {
      setCollapsed((prev) => {
        if (!prev.has(target.fileKey)) return prev;
        const next = new Set(prev);
        next.delete(target.fileKey);
        return next;
      });
      viewer?.scrollTo({ type: 'item', id: target.fileKey, align: 'start' });
    }
  }

  return (
    <div ref={containerRef} className={embedded ? "review-container flex h-full min-h-0 min-w-0 flex-col" : "fixed inset-0 z-50 flex justify-end"}>
      {!embedded && <div className="absolute inset-0 bg-shade/40" onClick={onClose} />}
      <aside className={embedded ? "flex h-full min-h-0 min-w-0 flex-col bg-background" : "relative flex h-full w-full max-w-[1040px] animate-fade-in flex-col border-l border-border bg-background shadow-2xl"}>
        {embedded && <div className="flex shrink-0 flex-wrap items-center justify-between gap-2 border-b border-border/60 px-3 py-2"><ReviewSourceSelector /><ReviewDisplayActions narrow={narrow} /></div>}
        <header className="flex min-h-12 shrink-0 flex-wrap items-center gap-2 border-b border-border/60 bg-background px-4 py-2">
          <span className="text-[14px] font-semibold">{embedded ? '' : turnScope ? 'Review turn changes' : 'Review changes'}</span>
          {files && files.length > 0 && (
            <span className="flex items-center gap-2 text-[12px] text-muted-foreground">
              <span className="tabular-nums">
                {files.length} file{files.length === 1 ? '' : 's'}
              </span>
              <DiffStatLabel additions={stat.additions} deletions={stat.deletions} layout="inline" />
            </span>
          )}
          <div className="ml-auto flex flex-wrap items-center justify-end gap-1">
            {!turnScope && <HeaderButton onClick={() => setGuided(value => !value)}>{guided ? "Show files" : "Walk me through it"}</HeaderButton>}
            {!guided && reviewFiles.length > 0 && (
              <>
                <HeaderButton
                  onClick={() =>
                    setCollapsed(
                      allCollapsed ? new Set() : new Set(reviewFiles.map((f) => f.fileKey)),
                    )
                  }
                >
                  {allCollapsed ? 'Expand all' : 'Collapse all'}
                </HeaderButton>
                {!turnScope && (confirmAll ? (
                  <ConfirmInline
                    label={`Discard all ${reviewFiles.length} files?`}
                    onConfirm={() => {
                      setConfirmAll(false);
                      void revert(reviewFiles.map((f) => f.filePath));
                    }}
                    onCancel={() => setConfirmAll(false)}
                  />
                ) : (
                  <HeaderButton onClick={() => setConfirmAll(true)}>
                    <RotateCcw className="size-3.5" /> Revert all
                  </HeaderButton>
                ))}
              </>
            )}
            <button
              type="button"
              onClick={onClose}
              aria-label="Close"
              className={cn('ml-1 rounded-md p-1 text-muted-foreground hover:bg-accent/50 hover:text-foreground', embedded && 'hidden')}
            >
              <X className="size-4" />
            </button>
          </div>
        </header>

        {guided ? <div className="min-h-0 flex-1"><GuidedWalkthrough narrow={narrow} onChanged={() => { load(); onChanged(); }} onClose={() => setGuided(false)} /></div> : <>
        <div className="flex min-h-0 flex-1">
          {showNav && (
            <nav className={embedded ? "review-file-nav shrink-0 border-r border-border/60" : "hidden w-60 shrink-0 border-r border-border/60 md:block"}>
              <DiffFileTree
                entries={treeEntries}
                onSelectFile={selectTreeFile}
                selectedPath={selectedPath}
                revealRequestId={revealRequestId}
                ariaLabel="Changed files"
                className="h-full"
              />
            </nav>
          )}

          <div className="flex min-h-0 min-w-0 flex-1 flex-col bg-background">
            {error && <div className="px-5 py-3 text-[12.5px] text-destructive">{error}</div>}
            {files === null && !error && (
              <div className="flex flex-col gap-3 px-5 py-4">
                {[0, 1].map((i) => (
                  <div key={i} className="h-28 animate-pulse rounded-lg bg-secondary/50" />
                ))}
              </div>
            )}
            {files?.length === 0 && (
              <div className="py-16 text-center text-[13px] text-muted-foreground">
                {turnScope ? 'No changes in this turn.' : 'This session has no uncommitted changes.'}
              </div>
            )}
            {turnUnavailable.length > 0 && <div className="max-h-40 overflow-auto border-b border-border px-4 py-3 text-xs text-muted-foreground">{turnUnavailable.map(file => <p className="py-1" key={file.path}><span className="font-mono text-foreground">{file.path}</span> · {file.error}</p>)}</div>}
            {reviewFiles.length > 0 && (
              <ReviewDiffView
                codeViewKey={`mira-review:${reviewFiles.map((f) => f.fileKey).join('|')}`}
                files={reviewFiles}
                comments={comments}
                onAddComment={(c) => setComments((prev) => [...prev, c])}
                onUpdateCommentText={(id, text) =>
                  setComments((prev) => prev.map((c) => (c.id === id ? { ...c, text } : c)))
                }
                onRemoveComment={(id) => setComments((prev) => prev.filter((c) => c.id !== id))}
                viewerRef={setViewer}
                className="min-h-0 flex-1 overflow-auto"
                options={displayOptions}
                renderHeaderFilenameSuffix={() => null}
                renderHeaderPrefix={(_fileDiff, fileKey, isCollapsed) => (
                  <span className="flex shrink-0 items-center">
                    <ChevronDown
                      className={cn(
                        'size-3 text-muted-foreground transition-transform',
                        isCollapsed && '-rotate-90',
                      )}
                    />
                    {commentsIn(fileKey) > 0 && (
                      <span className="ml-1 flex items-center gap-1 rounded-full bg-mira-blue/15 px-1.5 text-[11px] text-mira-blue">
                        <MessageCircle fill="currentColor" className="size-3" />
                        {commentsIn(fileKey)}
                      </span>
                    )}
                  </span>
                )}
                renderHeaderMetadata={(fileDiff: FileDiffMetadata) => (
                  <FileHeaderMeta
                    fileDiff={fileDiff}
                    fileByKey={fileByKey}
                    onRevert={turnScope ? undefined : (path) => void revert([path])}
                  />
                )}
              />
            )}
          </div>
        </div>

        <footer className="flex flex-wrap items-center gap-3 border-t border-border px-5 py-3">
          <span className="text-[12px] text-muted-foreground">
            Select lines to comment · comments go to the agent as one message.
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
        </>}
      </aside>
    </div>
  );
}

function FileHeaderMeta({
  fileDiff,
  fileByKey,
  onRevert,
}: {
  fileDiff: FileDiffMetadata;
  fileByKey: Map<string, ReviewDiffFile>;
  onRevert?: (path: string) => void;
}) {
  const [confirming, setConfirming] = useState(false);
  const key = buildFileDiffIdentityKey(fileDiff);
  const entry = fileByKey.get(key);
  const path = entry?.filePath ?? resolveFileDiffPath(fileDiff);
  return (
    <span className="flex items-center gap-2">
      {onRevert && (confirming ? (
        <ConfirmInline
          label="Discard changes?"
          onConfirm={() => { setConfirming(false); onRevert(path); }}
          onCancel={() => setConfirming(false)}
        />
      ) : (
        <button
          type="button"
          onClick={() => setConfirming(true)}
          title="Discard this session's changes to the file"
          className="flex shrink-0 items-center gap-1 rounded-md px-2 py-1 font-sans text-[12px] text-muted-foreground hover:bg-accent/50 hover:text-foreground"
        >
          <RotateCcw className="size-3.5" />
          Revert
        </button>
      ))}
    </span>
  );
}

function HeaderButton({ onClick, children }: { onClick: () => void; children: React.ReactNode }) {
  return (
    <button
      type="button"
      onClick={onClick}
      className="flex min-h-8 shrink-0 items-center gap-1 whitespace-nowrap rounded-full bg-secondary px-2.5 py-1.5 text-[12px] text-muted-foreground hover:bg-accent/50 hover:text-foreground"
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
    <span className="flex items-center gap-1.5 font-sans text-[12px]">
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
