import { AnimatePresence, m } from 'framer-motion';
import {
  ArrowUp,
  ArrowUpRight,
  CornerDownLeft,
  GitBranch,
  GitMerge,
  GitPullRequest,
} from 'lucide-react';
import { useEffect, useRef, useState } from 'react';
import type { BranchPrView, GitStatusView, SessionDiffView, SessionFile } from '../../api';
import { cn } from '../../lib/utils';
import { Popover, PopoverContent, PopoverTrigger } from '../ui/popover';
import { fmtNum } from './ProgressSection';
import { SectionHeader } from './SectionHeader';
export function prStateColor(state: string, isDraft: boolean): string {
  if (isDraft) return '#8b949e';
  if (state === 'OPEN') return '#3fb950';
  if (state === 'MERGED') return '#a371f7';
  return '#f85149'; // CLOSED
}

/* ------------------------------------------------------------------ */
/* Commit dialog                                                       */
/* ------------------------------------------------------------------ */

export function CommitDialog({
  branch,
  gitStatus,
  sessionDiff,
  onCommit,
  onPush,
  onClose,
}: {
  branch: string;
  gitStatus: GitStatusView;
  sessionDiff: SessionDiffView;
  onCommit: (message: string, includeUnstaged: boolean, pushAfter: boolean) => Promise<void>;
  onPush: () => Promise<void>;
  onClose: () => void;
}) {
  const [message, setMessage] = useState('');
  // The session's brand-new files are untracked, and a plain commit only
  // stages tracked changes — so default to including them when it made any.
  const newFiles = sessionDiff.untracked ?? 0;
  const [includeUnstaged, setIncludeUnstaged] = useState(newFiles > 0);
  const [loading, setLoading] = useState<null | 'commit' | 'commit-push' | 'push'>(null);
  const [error, setError] = useState<string | null>(null);
  const textareaRef = useRef<HTMLTextAreaElement>(null);

  useEffect(() => {
    textareaRef.current?.focus();
  }, []);

  async function handleCommit(pushAfter: boolean) {
    setError(null);
    setLoading(pushAfter ? 'commit-push' : 'commit');
    try {
      await onCommit(message, includeUnstaged, pushAfter);
      onClose();
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setLoading(null);
    }
  }

  async function handlePush() {
    setError(null);
    setLoading('push');
    try {
      await onPush();
      onClose();
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setLoading(null);
    }
  }

  const hasAhead = gitStatus.ahead > 0;

  return (
    <div className="flex flex-col" style={{ backgroundColor: 'var(--float-card-solid)' }}>
      {/* Branch row */}
      <div className="flex items-center gap-2 px-3 py-2.5 border-b border-fg/[0.07]">
        <GitBranch className="size-3.5 shrink-0 text-fg/40" />
        <span className="text-[13px] font-medium text-fg/70">{branch}</span>
      </div>

      {/* Commit message */}
      <div className="px-3 pt-2.5 pb-2 border-b border-fg/[0.07]">
        <textarea
          ref={textareaRef}
          value={message}
          onChange={(e) => setMessage(e.target.value)}
          onKeyDown={(e) => {
            if ((e.metaKey || e.ctrlKey) && e.key === 'Enter') {
              e.preventDefault();
              void handleCommit(false);
            }
          }}
          placeholder="Commit message (leave blank to generate)…"
          rows={3}
          className="w-full resize-none rounded-md bg-fg/[0.06] px-2.5 py-2 text-[12.5px] text-fg/80 placeholder-fg/25 outline-none focus:ring-1 focus:ring-fg/20 transition-shadow"
        />
      </div>

      {/* Include untracked (new) files toggle */}
      {newFiles > 0 && (
        <button
          type="button"
          onClick={() => setIncludeUnstaged((v) => !v)}
          className="flex items-center justify-between gap-2 px-3 py-2.5 border-b border-fg/[0.07] hover:bg-fg/[0.04] transition-colors text-left"
        >
          <span className="text-[12.5px] text-fg/60">
            Include new files <span className="text-fg/30">({newFiles})</span>
          </span>
          <div className="flex items-center gap-2">
            <div
              className={cn(
                'w-8 h-4 rounded-full transition-colors flex items-center',
                includeUnstaged ? 'bg-blue-500' : 'bg-fg/[0.12]',
              )}
            >
              <div
                className={cn(
                  'size-3 rounded-full bg-white shadow transition-transform mx-0.5',
                  includeUnstaged ? 'translate-x-4' : 'translate-x-0',
                )}
              />
            </div>
          </div>
        </button>
      )}

      {/* Error */}
      {error && (
        <div className="px-3 py-2 text-[11.5px] text-red-400/80 border-b border-fg/[0.07]">
          {error}
        </div>
      )}

      {/* Action rows */}
      <button
        type="button"
        disabled={!!loading}
        onClick={() => void handleCommit(false)}
        className="flex items-center justify-between px-3 py-2.5 border-b border-fg/[0.07] hover:bg-fg/[0.04] disabled:opacity-50 transition-colors"
      >
        <span className="text-[13px] font-medium text-fg/75">
          {loading === 'commit' ? 'Committing…' : 'Commit'}
        </span>
        <span className="flex items-center gap-0.5 text-[11px] text-fg/25">
          <span>⌘</span>
          <CornerDownLeft className="size-3" />
        </span>
      </button>

      <button
        type="button"
        disabled={!!loading}
        onClick={() => void handleCommit(true)}
        className="flex items-center justify-between px-3 py-2.5 border-b border-fg/[0.07] hover:bg-fg/[0.04] disabled:opacity-50 transition-colors"
      >
        <span className="text-[13px] font-medium text-fg/75">
          {loading === 'commit-push' ? 'Committing & pushing…' : 'Commit and push'}
        </span>
      </button>

      <button
        type="button"
        disabled={!!loading || !hasAhead}
        onClick={() => void handlePush()}
        className="flex items-center justify-between px-3 py-2.5 hover:bg-fg/[0.04] disabled:opacity-40 transition-colors"
      >
        <span className={cn('text-[13px] font-medium', hasAhead ? 'text-fg/75' : 'text-fg/30')}>
          {loading === 'push' ? 'Pushing…' : hasAhead ? `Push ↑${gitStatus.ahead}` : 'Push'}
        </span>
      </button>
    </div>
  );
}

/* ------------------------------------------------------------------ */
/* Environment                                                         */
/* ------------------------------------------------------------------ */

/** True when this session has something to act on in the repo: its own
 *  uncommitted changes, commits it made that aren't pushed, or a PR for
 *  the branch. Branch and environment pickers already live under the
 *  composer, so they aren't repeated here. */
export function workspaceHasContent(
  gitStatus: GitStatusView | null,
  sessionDiff: SessionDiffView,
  branchPr: BranchPrView | null,
  sessionCommitted: boolean,
): boolean {
  if (!gitStatus?.in_repo) return false;
  return sessionDiff.files.length > 0 || (sessionCommitted && gitStatus.ahead > 0) || !!branchPr;
}

export function WorkspaceSection({
  gitStatus,
  sessionDiff,
  branchPr,
  sessionCommitted,
  onPush,
  onCommit,
  onReview,
}: {
  gitStatus: GitStatusView;
  sessionDiff: SessionDiffView;
  branchPr: BranchPrView | null;
  sessionCommitted: boolean;
  onReview: (path?: string) => void;
  onPush: () => Promise<void>;
  onCommit: (message: string, includeUnstaged: boolean, pushAfter: boolean) => Promise<void>;
}) {
  const [open, setOpen] = useState(true);
  const [commitOpen, setCommitOpen] = useState(false);
  const [pushing, setPushing] = useState(false);

  // Only what this session did: its own uncommitted files, and pushes for
  // commits it made — not whatever else happens to be dirty in the repo.
  const pending = sessionDiff.files.length;
  const canPush = sessionCommitted && gitStatus.ahead > 0;

  return (
    <div>
      <SectionHeader label="Changes" open={open} onToggle={() => setOpen((v) => !v)} />
      <AnimatePresence initial={false}>
        {open && (
          <m.div
            initial={{ height: 0, opacity: 0 }}
            animate={{ height: 'auto', opacity: 1, transition: { duration: 0.16 } }}
            exit={{ height: 0, opacity: 0, transition: { duration: 0.12 } }}
            // -mx-2 px-2: room for the rows' full-width, rounded hover.
            className="-mx-2 overflow-hidden px-2"
          >
            <div className="flex flex-col gap-1.5 pb-1.5">
              {pending > 0 && (
                <div className="flex flex-col gap-2 rounded-lg bg-fg/[0.035] p-2.5">
                  {/* Summary on its own line, actions below: the panel is
                      narrow, and sharing a row squeezed the summary. */}
                  <div className="flex items-center gap-2">
                    <GitMerge className="size-3.5 shrink-0 text-fg/35" />
                    <span className="min-w-0 flex-1 truncate text-[12.5px] font-medium text-fg/75">
                      {pending} file{pending === 1 ? '' : 's'} changed
                    </span>
                    <span className="flex shrink-0 gap-1.5 font-mono text-[11px]">
                      <span className="text-green-500/90 dark:text-green-400/80">
                        +{fmtNum(sessionDiff.added)}
                      </span>
                      <span className="text-red-500/90 dark:text-red-400/80">
                        −{fmtNum(sessionDiff.removed)}
                      </span>
                    </span>
                  </div>
                  <div className="grid grid-cols-2 gap-1.5">
                    <button
                      type="button"
                      onClick={() => onReview()}
                      className="h-7 rounded-md border border-fg/[0.1] text-[12px] font-medium text-fg/70 transition-colors hover:bg-fg/[0.06] hover:text-fg/90"
                    >
                      Review
                    </button>
                    <Popover open={commitOpen} onOpenChange={setCommitOpen}>
                      <PopoverTrigger asChild>
                        <button
                          type="button"
                          className="h-7 rounded-md bg-foreground/90 text-[12px] font-medium text-background transition-opacity hover:opacity-90"
                        >
                          Commit
                        </button>
                      </PopoverTrigger>
                      <PopoverContent
                        className="w-72 p-0 overflow-hidden border-fg/[0.10]"
                        align="end"
                        side="bottom"
                        style={{ backgroundColor: 'var(--float-card-solid)' }}
                      >
                        <CommitDialog
                          branch={gitStatus.branch ?? 'detached HEAD'}
                          gitStatus={gitStatus}
                          sessionDiff={sessionDiff}
                          onCommit={onCommit}
                          onPush={onPush}
                          onClose={() => setCommitOpen(false)}
                        />
                      </PopoverContent>
                    </Popover>
                  </div>
                </div>
              )}
              {pending > 0 && (
                <ChangedFiles files={sessionDiff.files} onOpen={(p) => onReview(p)} />
              )}

              {pending === 0 && canPush && (
                <button
                  type="button"
                  disabled={pushing}
                  onClick={async () => {
                    setPushing(true);
                    try {
                      await onPush();
                    } finally {
                      setPushing(false);
                    }
                  }}
                  className="flex items-center gap-2 rounded-lg bg-fg/[0.035] px-2.5 py-2 text-left transition-colors hover:bg-fg/[0.06] disabled:opacity-50"
                >
                  <ArrowUp className="size-3.5 shrink-0 text-fg/35" />
                  <span className="flex-1 text-[12.5px] font-medium text-fg/70">
                    {pushing
                      ? 'Pushing…'
                      : `Push ${gitStatus.ahead} commit${gitStatus.ahead === 1 ? '' : 's'}`}
                  </span>
                </button>
              )}

              {branchPr && (
                <a
                  href={branchPr.url}
                  target="_blank"
                  rel="noopener noreferrer"
                  title={branchPr.title}
                  className="-mx-2 flex items-center gap-2 rounded px-2 py-1 text-fg/60 transition-colors hover:bg-fg/[0.04] hover:text-fg/85"
                >
                  <GitPullRequest
                    className="size-3.5 shrink-0"
                    style={{ color: prStateColor(branchPr.state, branchPr.isDraft) }}
                  />
                  <span className="shrink-0 text-[12.5px] text-fg/45">#{branchPr.number}</span>
                  <span className="min-w-0 flex-1 truncate text-[12.5px]">{branchPr.title}</span>
                  <ArrowUpRight className="size-3 shrink-0 text-fg/30" />
                </a>
              )}
            </div>
          </m.div>
        )}
      </AnimatePresence>
    </div>
  );
}

export const FILES_SHOWN = 6;

export const STATUS_MARK: Record<
  SessionFile['status'],
  { letter: string; className: string; title: string }
> = {
  added: { letter: 'A', className: 'text-green-400/80', title: 'New file' },
  modified: { letter: 'M', className: 'text-amber-300/80', title: 'Modified' },
  deleted: { letter: 'D', className: 'text-red-400/80', title: 'Deleted' },
};

/** The chat's changed files; each opens the review drawer on its diff. */
export function ChangedFiles({
  files,
  onOpen,
}: {
  files: SessionFile[];
  onOpen: (path: string) => void;
}) {
  const [all, setAll] = useState(false);
  const shown = all ? files : files.slice(0, FILES_SHOWN);
  return (
    <div className="flex flex-col">
      {shown.map((f) => {
        const slash = f.path.lastIndexOf('/');
        const dir = slash >= 0 ? f.path.slice(0, slash + 1) : '';
        const name = f.path.slice(slash + 1);
        const mark = STATUS_MARK[f.status];
        return (
          <button
            key={f.path}
            type="button"
            onClick={() => onOpen(f.path)}
            title={f.path}
            className="-mx-2 flex min-w-0 items-center gap-2 rounded-md px-2 py-[3px] text-left transition-colors hover:bg-fg/[0.05]"
          >
            <span
              className={cn(
                'w-3 shrink-0 text-center font-mono text-[10.5px] font-semibold',
                mark.className,
              )}
              title={mark.title}
            >
              {mark.letter}
            </span>
            {/* Name first and never cut; the folder gives way. */}
            <span className="flex min-w-0 flex-1 items-baseline gap-1.5 text-[12.5px]">
              <span
                className={cn(
                  'shrink-0 text-fg/75',
                  f.status === 'deleted' && 'line-through decoration-fg/25',
                )}
              >
                {name}
              </span>
              {dir && <span className="min-w-0 truncate text-[11px] text-fg/30">{dir}</span>}
            </span>
            <span className="flex shrink-0 gap-1 font-mono text-[10.5px]">
              {f.binary ? (
                <span className="text-fg/30">bin</span>
              ) : (
                <>
                  {f.added > 0 && <span className="text-green-400/70">+{fmtNum(f.added)}</span>}
                  {f.removed > 0 && <span className="text-red-400/70">−{fmtNum(f.removed)}</span>}
                </>
              )}
            </span>
          </button>
        );
      })}
      {files.length > FILES_SHOWN && (
        <button
          type="button"
          onClick={() => setAll((v) => !v)}
          className="self-start py-[3px] text-[11.5px] text-fg/35 transition-colors hover:text-fg/70"
        >
          {all ? 'Show fewer' : `Show ${files.length - FILES_SHOWN} more`}
        </button>
      )}
    </div>
  );
}
