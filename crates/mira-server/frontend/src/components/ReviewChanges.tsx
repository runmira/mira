/**
 * "Review changes" drawer: every file this session changed, rendered with
 * the Pierre CodeView diff surface, with a file tree, per-file collapse,
 * revert, and inline line comments sent back to the agent as one message.
 */
import { useCallback, useEffect, useMemo, useState } from 'react';
import { MessageCircle, RotateCcw, ChevronDown, X } from 'lucide-react';
import type { FileDiffMetadata } from '@pierre/diffs';
import { cn } from '@/lib/utils';
import { PREF_KEYS, useStringPref } from '@/lib/prefs';
import { getSessionChanges, revertFile, type SessionChange } from '../api';
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
  const [comments, setComments] = useState<MiraReviewComment[]>([]);
  const [collapsed, setCollapsed] = useState<Set<string>>(new Set());
  const [confirmAll, setConfirmAll] = useState(false);
  const [selectedPath, setSelectedPath] = useState<string | null>(null);
  const [revealRequestId, setRevealRequestId] = useState(0);
  const [viewer, setViewer] = useState<ReviewDiffViewHandle | null>(null);
  // Diff layout (Settings → General → Diff).
  const [diffLayout] = useStringPref(PREF_KEYS.diffLayout, 'unified');

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

  /** Parse every session file's unified diff into Pierre file entries (diff order kept). */
  const reviewFiles: ReviewDiffFile[] = useMemo(() => {
    const out: ReviewDiffFile[] = [];
    for (const f of files ?? []) {
      const parsed = getRenderablePatch(f.diff, 'mira-review');
      if (parsed?.kind !== 'files') continue;
      for (const fileDiff of parsed.files) {
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
  }, [files, collapsed]);

  const treeEntries: ReadonlyArray<DiffFileTreeEntry> = useMemo(
    () => diffFileTreeEntries(reviewFiles.map((f) => f.fileDiff)),
    [reviewFiles],
  );

  const stat = useMemo(
    () => getDiffLineStat(reviewFiles.map((f) => f.fileDiff)),
    [reviewFiles],
  );

  const fileByKey = useMemo(() => new Map(reviewFiles.map((f) => [f.fileKey, f])), [reviewFiles]);

  if (!open) return null;

  async function revert(paths: string[]) {
    setError(null);
    try {
      for (const p of paths) await revertFile(p);
      setComments((prev) => prev.filter((c) => !paths.includes(c.filePath)));
      load();
      onChanged();
    } catch (e) {
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
    <div className="fixed inset-0 z-50 flex justify-end">
      <div className="absolute inset-0 bg-black/40" onClick={onClose} />
      <aside className="relative flex h-full w-full max-w-[1040px] animate-fade-in flex-col border-l border-border bg-background shadow-2xl">
        <header className="flex h-10 min-h-10 shrink-0 flex-wrap items-center gap-2 border-b border-border/60 bg-background px-4">
          <span className="text-[14px] font-semibold">Review changes</span>
          {files && files.length > 0 && (
            <span className="flex items-center gap-2 text-[12px] text-muted-foreground">
              <span className="tabular-nums">
                {files.length} file{files.length === 1 ? '' : 's'}
              </span>
              <DiffStatLabel additions={stat.additions} deletions={stat.deletions} layout="inline" />
            </span>
          )}
          <div className="ml-auto flex items-center gap-1">
            {reviewFiles.length > 0 && (
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
                {confirmAll ? (
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
          {showNav && (
            <nav className="hidden w-60 shrink-0 border-r border-border/60 md:block">
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

          <div className="min-h-0 min-w-0 flex-1 bg-background">
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
                This session has no uncommitted changes.
              </div>
            )}
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
                className="h-full min-h-0 overflow-auto"
                options={{ diffStyle: diffLayout === 'split' ? 'split' : 'unified' }}
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
                    onRevert={(path) => void revert([path])}
                  />
                )}
              />
            )}
          </div>
        </div>

        <footer className="flex items-center gap-3 border-t border-border px-5 py-3">
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
  onRevert: (path: string) => void;
}) {
  const [confirming, setConfirming] = useState(false);
  const key = buildFileDiffIdentityKey(fileDiff);
  const entry = fileByKey.get(key);
  const path = entry?.filePath ?? resolveFileDiffPath(fileDiff);
  const s = getDiffLineStat([fileDiff]);
  return (
    <span className="flex items-center gap-2">
      <DiffStatLabel additions={s.additions} deletions={s.deletions} layout="inline" />
      {confirming ? (
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
      )}
    </span>
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
