import { useState, useEffect, useMemo, useRef } from 'react';
import { motion, AnimatePresence } from 'framer-motion';
import {
  ArrowUp,
  GitBranch,
  Check,
  ArrowRight,
  Circle,
  ChevronDown,
  ChevronRight,
  GitMerge,
  GitPullRequest,
  CornerDownLeft,
  PanelRightOpen,
  ArrowUpRight,
  PanelRightClose,
  FileCode2,
  GitCommitHorizontal,
  ListChecks,
  BookMarked,
  Globe2,
} from 'lucide-react';
import { cn } from '../lib/utils';
import { prettyUrl } from '../lib/refs';
import type { GitStatusView, SessionDiffView, SessionFile, BranchPrView } from '../api';
import type { TaskItem } from '../types';
import type { SubagentStreamState, Entry } from '../App';
import { Popover, PopoverContent, PopoverTrigger } from './ui/popover';
import { SubagentFace } from './SubagentFace';
import { personaName, useSubagents, type Subagent } from '../lib/subagents';

/* ------------------------------------------------------------------ */
/* Helpers                                                             */
/* ------------------------------------------------------------------ */

function fmtNum(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}k`;
  return String(n);
}

function extractSources(entries: Entry[]): string[] {
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
      if (!seen.has(url)) { seen.add(url); urls.push(url); }
    }
  }
  return urls;
}

function formatAgentName(raw: string): string {
  return raw
    .split(/[-_\s]+/)
    .map((w) => w.charAt(0).toUpperCase() + w.slice(1))
    .join(' ');
}

function agentDisplayInfo(
  state: SubagentStreamState,
  roster: Subagent[] | undefined,
): { name: string; badge: string | null; description: string } {
  const rawCat = state.agentCategory ?? null;
  const promptStr = (state.prompt ?? '').trim();

  // The subagent's own persona, the same name its transcript row and
  // panel tab show.
  const type = state.agentName ?? null;
  const name = type ? personaName(roster?.find((r) => r.name === type), type) : 'Helper';
  const badge = rawCat ? formatAgentName(rawCat) : null;
  const description = promptStr.split('\n')[0].replace(/^#+ /, '');

  return {
    name,
    badge,
    description: description.length > 44 ? description.slice(0, 44) + '…' : description,
  };
}

function prStateColor(state: string, isDraft: boolean): string {
  if (isDraft) return '#8b949e';
  if (state === 'OPEN') return '#3fb950';
  if (state === 'MERGED') return '#a371f7';
  return '#f85149'; // CLOSED
}

/* ------------------------------------------------------------------ */
/* Animation variants                                                  */
/* ------------------------------------------------------------------ */

const itemVariants = {
  hidden: { opacity: 0, x: -4 },
  visible: (i: number) => ({
    opacity: 1, x: 0,
    transition: { duration: 0.16, delay: i * 0.035 },
  }),
};

/* ------------------------------------------------------------------ */
/* Section header                                                      */
/* ------------------------------------------------------------------ */

function SectionHeader({
  label,
  right,
  open,
  onToggle,
}: {
  label: string;
  right?: React.ReactNode;
  open: boolean;
  onToggle: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onToggle}
      className="flex w-full items-center justify-between py-1.5 text-left group"
    >
      <span className="text-[11px] font-bold uppercase tracking-widest text-fg/40 group-hover:text-fg/60 transition-colors">
        {label}
      </span>
      <div className="flex items-center gap-1.5">
        {right}
        {open
          ? <ChevronDown className="size-3.5 text-fg/25" />
          : <ChevronRight className="size-3.5 text-fg/25" />}
      </div>
    </button>
  );
}

/* ------------------------------------------------------------------ */
/* Progress                                                            */
/* ------------------------------------------------------------------ */

/** Tasks still on the list — a deleted task was dropped, not finished. */
function liveTasks(tasks: TaskItem[]): TaskItem[] {
  return tasks.filter((t) => t.status !== 'deleted');
}

function ProgressSection({ tasks: all }: { tasks: TaskItem[] }) {
  const [open, setOpen] = useState(true);
  const tasks = liveTasks(all);
  const total = tasks.length;
  const done = tasks.filter((t) => t.status === 'completed').length;
  const active = tasks.find((t) => t.status === 'in_progress');

  return (
    <div>
      <SectionHeader
        label="Progress"
        right={<span className="text-[10.5px] text-fg/25">{done}/{total}</span>}
        open={open}
        onToggle={() => setOpen((v) => !v)}
      />
      <AnimatePresence initial={false}>
        {open && (
          <motion.div
            initial={{ height: 0, opacity: 0 }}
            animate={{ height: 'auto', opacity: 1, transition: { duration: 0.16 } }}
            exit={{ height: 0, opacity: 0, transition: { duration: 0.12 } }}
            // -mx-2 px-2: room for the rows' full-width, rounded hover.
            className="-mx-2 overflow-hidden px-2"
          >
            <div className="flex flex-col gap-0.5 pb-1">
              <AnimatePresence>
                {tasks.map((task, i) => {
                  const isDone = task.status === 'completed';
                  const isCurrent = task.status === 'in_progress';
                  return (
                    <motion.div
                      key={task.id}
                      custom={i}
                      variants={itemVariants}
                      initial="hidden"
                      animate="visible"
                      className={cn(
                        'flex items-start gap-2 text-[13px] leading-snug py-1',
                        isDone ? 'text-fg/25' : isCurrent ? 'text-fg/85' : 'text-fg/50',
                      )}
                    >
                      <span className="mt-[3px] shrink-0">
                        {isDone
                          ? <Check className="size-3 text-green-500/60" strokeWidth={2.5} />
                          : isCurrent
                          ? <ArrowRight className="size-3 text-blue-400" strokeWidth={2} />
                          : <Circle className="size-3 text-fg/20" />}
                      </span>
                      <span className={cn('min-w-0', isDone && 'line-through decoration-fg/15')}>
                        {task.subject}
                      </span>
                    </motion.div>
                  );
                })}
              </AnimatePresence>
              {active && (
                <div className="mt-1.5 h-px bg-fg/10 rounded-full overflow-hidden">
                  <motion.div
                    className="h-full bg-blue-400/40 rounded-full"
                    animate={{ x: ['-100%', '200%'] }}
                    transition={{ duration: 2, repeat: Infinity, ease: 'easeInOut' }}
                  />
                </div>
              )}
            </div>
          </motion.div>
        )}
      </AnimatePresence>
    </div>
  );
}

/* ------------------------------------------------------------------ */
/* Commit dialog                                                       */
/* ------------------------------------------------------------------ */

function CommitDialog({
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
            <div className={cn(
              'w-8 h-4 rounded-full transition-colors flex items-center',
              includeUnstaged ? 'bg-blue-500' : 'bg-fg/[0.12]',
            )}>
              <div className={cn(
                'size-3 rounded-full bg-white shadow transition-transform mx-0.5',
                includeUnstaged ? 'translate-x-4' : 'translate-x-0',
              )} />
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
          <span>⌘</span><CornerDownLeft className="size-3" />
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
        <span className={cn(
          'text-[13px] font-medium',
          hasAhead ? 'text-fg/75' : 'text-fg/30',
        )}>
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
function workspaceHasContent(
  gitStatus: GitStatusView | null,
  sessionDiff: SessionDiffView,
  branchPr: BranchPrView | null,
  sessionCommitted: boolean,
): boolean {
  if (!gitStatus?.in_repo) return false;
  return (
    sessionDiff.files.length > 0 ||
    (sessionCommitted && gitStatus.ahead > 0) ||
    !!branchPr
  );
}

function WorkspaceSection({
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
          <motion.div
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
                      <span className="text-green-500/90 dark:text-green-400/80">+{fmtNum(sessionDiff.added)}</span>
                      <span className="text-red-500/90 dark:text-red-400/80">−{fmtNum(sessionDiff.removed)}</span>
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
              {pending > 0 && <ChangedFiles files={sessionDiff.files} onOpen={(p) => onReview(p)} />}

              {pending === 0 && canPush && (
                <button
                  type="button"
                  disabled={pushing}
                  onClick={async () => {
                    setPushing(true);
                    try { await onPush(); } finally { setPushing(false); }
                  }}
                  className="flex items-center gap-2 rounded-lg bg-fg/[0.035] px-2.5 py-2 text-left transition-colors hover:bg-fg/[0.06] disabled:opacity-50"
                >
                  <ArrowUp className="size-3.5 shrink-0 text-fg/35" />
                  <span className="flex-1 text-[12.5px] font-medium text-fg/70">
                    {pushing ? 'Pushing…' : `Push ${gitStatus.ahead} commit${gitStatus.ahead === 1 ? '' : 's'}`}
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
          </motion.div>
        )}
      </AnimatePresence>
    </div>
  );
}

const FILES_SHOWN = 6;

const STATUS_MARK: Record<SessionFile['status'], { letter: string; className: string; title: string }> = {
  added: { letter: 'A', className: 'text-green-400/80', title: 'New file' },
  modified: { letter: 'M', className: 'text-amber-300/80', title: 'Modified' },
  deleted: { letter: 'D', className: 'text-red-400/80', title: 'Deleted' },
};

/** The chat's changed files; each opens the review drawer on its diff. */
function ChangedFiles({ files, onOpen }: { files: SessionFile[]; onOpen: (path: string) => void }) {
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
            <span className={cn('w-3 shrink-0 text-center font-mono text-[10.5px] font-semibold', mark.className)} title={mark.title}>
              {mark.letter}
            </span>
            {/* Name first and never cut; the folder gives way. */}
            <span className="flex min-w-0 flex-1 items-baseline gap-1.5 text-[12.5px]">
              <span className={cn('shrink-0 text-fg/75', f.status === 'deleted' && 'line-through decoration-fg/25')}>
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

/* ------------------------------------------------------------------ */
/* Subagents                                                           */
/* ------------------------------------------------------------------ */

/** This chat's subagents, in transcript order. State for a call that isn't
 *  in the transcript (another chat's, or one compacted away) is left out,
 *  so the panel never shows a section with nothing in it. */
function chatSubagents(
  subagentState: Map<string, SubagentStreamState>,
  entries: Entry[],
): { callId: string; state: SubagentStreamState }[] {
  const out: { callId: string; state: SubagentStreamState }[] = [];
  if (subagentState.size === 0) return out;
  for (const e of entries) {
    if (e.kind !== 'tool') continue;
    const s = subagentState.get(e.call.id);
    if (s) out.push({ callId: e.call.id, state: s });
  }
  return out;
}

function SubagentsSection({
  ordered,
  onOpenAgent,
}: {
  ordered: { callId: string; state: SubagentStreamState }[];
  onOpenAgent: (callId: string) => void;
}) {
  const [open, setOpen] = useState(true);
  const roster = useSubagents()?.subagents;

  return (
    <div>
      <SectionHeader label="Subagents" open={open} onToggle={() => setOpen((v) => !v)} />
      <AnimatePresence initial={false}>
        {open && (
          <motion.div
            initial={{ height: 0, opacity: 0 }}
            animate={{ height: 'auto', opacity: 1, transition: { duration: 0.16 } }}
            exit={{ height: 0, opacity: 0, transition: { duration: 0.12 } }}
            // -mx-2 px-2: room for the rows' full-width, rounded hover.
            className="-mx-2 overflow-hidden px-2"
          >
            <div className="flex flex-col gap-0.5 pb-1">
              <AnimatePresence>
                {ordered.map(({ callId, state }, i) => {
                  const type = state.agentName ?? null;
                  const persona = type ? roster?.find((r) => r.name === type) : undefined;
                  const { name, badge, description } = agentDisplayInfo(state, roster);
                  return (
                    <motion.button
                      key={callId}
                      custom={i}
                      variants={itemVariants}
                      initial="hidden"
                      animate="visible"
                      type="button"
                      onClick={() => onOpenAgent(callId)}
                      className="flex items-start gap-2.5 py-1.5 -mx-2 px-2 rounded text-left hover:bg-fg/[0.04] transition-colors w-full"
                    >
                      <SubagentFace
                        id={type ?? callId}
                        face={persona?.face}
                        size={20}
                        state={state.done ? 'done' : 'working'}
                        className="mt-px"
                      />
                      <span className="flex-1 min-w-0 flex flex-col gap-0.5">
                        <span className="flex items-baseline gap-1.5 min-w-0">
                          <span className="truncate text-[13px] font-semibold text-fg/75 leading-snug">{name}</span>
                          {badge && <span className="shrink-0 text-[11px] font-medium text-fg/30">({badge})</span>}
                        </span>
                        {description && (
                          <span className="truncate text-[12px] font-medium text-fg/40 leading-snug">{description}</span>
                        )}
                      </span>
                      {!state.done && <span className="text-[10px] text-fg/20 shrink-0 mt-0.5">running</span>}
                    </motion.button>
                  );
                })}
              </AnimatePresence>
            </div>
          </motion.div>
        )}
      </AnimatePresence>
    </div>
  );
}

/* ------------------------------------------------------------------ */
/* Sources                                                             */
/* ------------------------------------------------------------------ */

const SOURCES_SHOWN = 8;

const SOURCE_KIND_ICON = {
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
function describeSource(url: string): { title: string; detail: string; kind: keyof typeof SOURCE_KIND_ICON; origin: string | null } {
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
    const what: Record<string, string> = { branch: 'Branch', file: 'File', commit: 'Commit', list: '', repo: 'Repository', page: '' };
    const label = pretty.kind === 'commit' ? head.split('@')[1] ?? '' : detail;
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

function capitalize(s: string): string {
  return s ? s[0].toUpperCase() + s.slice(1) : s;
}

function SourceRow({ url }: { url: string }) {
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

function SourcesSection({ sources }: { sources: string[] }) {
  const [open, setOpen] = useState(true);
  const [all, setAll] = useState(false);
  const shown = all ? sources : sources.slice(0, SOURCES_SHOWN);

  return (
    <div>
      <SectionHeader label="Sources" open={open} onToggle={() => setOpen((v) => !v)} />
      <AnimatePresence initial={false}>
        {open && (
          <motion.div
            initial={{ height: 0, opacity: 0 }}
            animate={{ height: 'auto', opacity: 1, transition: { duration: 0.16 } }}
            exit={{ height: 0, opacity: 0, transition: { duration: 0.12 } }}
            // -mx-2 px-2: room for the rows' full-width, rounded hover.
            className="-mx-2 overflow-hidden px-2"
          >
            <div className="flex flex-col gap-0.5 pb-1">
              <AnimatePresence>
                {shown.map((url, i) => (
                  <motion.div key={url} custom={i} variants={itemVariants} initial="hidden" animate="visible">
                    <SourceRow url={url} />
                  </motion.div>
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
          </motion.div>
        )}
      </AnimatePresence>
    </div>
  );
}

/* ------------------------------------------------------------------ */
/* Main panel                                                          */
/* ------------------------------------------------------------------ */

const PANEL_WIDTH = 280;
const MIN_WIDTH_TO_SHOW = PANEL_WIDTH + 320;
/** Horizontal space the open panel covers, for the transcript / composer
 *  to keep clear. Nothing is reserved while it's collapsed to its pill. */
export const CONTEXT_PANEL_RESERVE = PANEL_WIDTH + 16;

/** Whether the window is wide enough to show the panel at all. */
export function useContextPanelFits(): boolean {
  const [w, setW] = useState(window.innerWidth);
  useEffect(() => {
    const onResize = () => setW(window.innerWidth);
    window.addEventListener('resize', onResize);
    return () => window.removeEventListener('resize', onResize);
  }, []);
  return w >= MIN_WIDTH_TO_SHOW;
}

/** True when the panel has anything to show for this session. */
export function contextPanelHasContent(p: {
  tasks: TaskItem[];
  gitStatus: GitStatusView | null;
  sessionDiff: SessionDiffView;
  branchPr: BranchPrView | null;
  sessionCommitted: boolean;
  subagentState: Map<string, SubagentStreamState>;
  entries: Entry[];
}): boolean {
  return (
    liveTasks(p.tasks).length > 0 ||
    workspaceHasContent(p.gitStatus, p.sessionDiff, p.branchPr, p.sessionCommitted) ||
    chatSubagents(p.subagentState, p.entries).length > 0 ||
    extractSources(p.entries).length > 0
  );
}

export type ContextPanelProps = {
  sessionTitle?: string;
  tasks: TaskItem[];
  subagentState: Map<string, SubagentStreamState>;
  entries: Entry[];
  gitStatus: GitStatusView | null;
  sessionDiff: SessionDiffView;
  branchPr: BranchPrView | null;
  /** This session committed via the panel — its unpushed commits are its own. */
  sessionCommitted: boolean;
  /** Expanded card vs. collapsed pill. Owned by the parent so it can keep
   *  the transcript clear of the card only while it's open. */
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onOpenAgent: (callId: string) => void;
  /** Open the "Review changes" drawer, on one file when given. */
  onReview: (path?: string) => void;
  onPush: () => Promise<void>;
  onCommit: (message: string, includeUnstaged: boolean, pushAfter: boolean) => Promise<void>;
};

const cardStyle: React.CSSProperties = {
  backgroundColor: 'var(--float-card)',
  backdropFilter: 'blur(14px)',
  WebkitBackdropFilter: 'blur(14px)',
  boxShadow: 'var(--float-shadow)',
};

export function ContextPanel(props: ContextPanelProps) {
  const {
    tasks,
    subagentState,
    entries,
    gitStatus,
    sessionDiff,
    branchPr,
    sessionCommitted,
    open,
    onOpenChange,
    onOpenAgent,
    onReview,
    onPush,
    onCommit,
  } = props;
  const fits = useContextPanelFits();
  // Derived once per render and shared by the pill and every section, so
  // they can't disagree about what there is to show.
  const sources = useMemo(() => extractSources(entries), [entries]);
  const subagents = useMemo(() => chatSubagents(subagentState, entries), [subagentState, entries]);
  const live = liveTasks(tasks);
  const showWorkspace = workspaceHasContent(gitStatus, sessionDiff, branchPr, sessionCommitted);

  if (!fits || (live.length === 0 && !showWorkspace && subagents.length === 0 && sources.length === 0)) {
    return null;
  }

  const done = live.filter((t) => t.status === 'completed').length;
  const pending = sessionDiff.files.length;
  const agentsRunning = subagents.some((s) => !s.state.done);

  return (
    <AnimatePresence mode="wait" initial={false}>
      {!open ? (
        /* Collapsed: a small pill that summarizes, out of the way. */
        <motion.button
          key="pill"
          type="button"
          onClick={() => onOpenChange(true)}
          title="Show context"
          initial={{ opacity: 0, scale: 0.94 }}
          animate={{ opacity: 1, scale: 1, transition: { duration: 0.16, ease: 'easeOut' } }}
          exit={{ opacity: 0, scale: 0.94, transition: { duration: 0.1 } }}
          className="absolute top-3 right-3 z-20 flex max-w-[340px] items-center gap-2.5 rounded-full border border-fg/[0.08] px-3 py-1.5 text-[12px] text-fg/55 transition-colors hover:border-fg/[0.16] hover:text-fg/80"
          style={cardStyle}
        >
          <PanelRightOpen className="size-3.5 shrink-0 text-fg/40" />
          <span className="font-medium text-fg/65">Context</span>
          {pending > 0 && (
            <span className="flex shrink-0 gap-1 font-mono text-[11px]">
              <span className="text-green-400/80">+{fmtNum(sessionDiff.added)}</span>
              <span className="text-red-400/80">−{fmtNum(sessionDiff.removed)}</span>
            </span>
          )}
          {pending > 0 && (
            <span className="shrink-0 text-fg/40">
              {pending} file{pending === 1 ? '' : 's'}
            </span>
          )}
          {live.length > 0 && (
            <span className="flex shrink-0 items-center gap-1">
              <Check className="size-3 text-green-500/60" strokeWidth={2.5} />
              {done}/{live.length}
            </span>
          )}
          {branchPr && (
            <span className="flex shrink-0 items-center gap-1">
              <GitPullRequest
                className="size-3"
                style={{ color: prStateColor(branchPr.state, branchPr.isDraft) }}
              />
              #{branchPr.number}
            </span>
          )}
          {agentsRunning && (
            <span className="size-1.5 shrink-0 rounded-full bg-mira-purple animate-pulse" title="subagents running" />
          )}
        </motion.button>
      ) : (
        <motion.aside
          key="card"
          initial={{ opacity: 0, scale: 0.97, y: -6 }}
          animate={{ opacity: 1, scale: 1, y: 0, transition: { duration: 0.18, ease: 'easeOut' } }}
          exit={{ opacity: 0, scale: 0.97, y: -4, transition: { duration: 0.12 } }}
          className="absolute top-3 right-3 z-20 flex flex-col overflow-hidden rounded-2xl border border-fg/[0.08]"
          style={{ ...cardStyle, width: PANEL_WIDTH, maxHeight: 'calc(100% - 24px)', transformOrigin: 'top right' }}
        >
          <div className="flex items-center justify-between px-4 pt-3 pb-1">
            <span className="text-[12.5px] font-semibold text-fg/75">Context</span>
            <button
              type="button"
              onClick={() => onOpenChange(false)}
              title="Collapse"
              className="-mr-1.5 rounded-md p-1 text-fg/35 transition-colors hover:bg-fg/[0.06] hover:text-fg/75"
            >
              <PanelRightClose className="size-3.5" />
            </button>
          </div>
          <div className="overflow-y-auto px-4 pb-1.5 flex flex-col divide-y divide-fg/[0.07]">
            {live.length > 0 && (
              <div className="py-2">
                <ProgressSection tasks={tasks} />
              </div>
            )}
            {gitStatus && showWorkspace && (
              <div className="py-2">
                <WorkspaceSection
                  gitStatus={gitStatus}
                  sessionDiff={sessionDiff}
                  branchPr={branchPr}
                  sessionCommitted={sessionCommitted}
                  onReview={onReview}
                  onPush={onPush}
                  onCommit={onCommit}
                />
              </div>
            )}
            {subagents.length > 0 && (
              <div className="py-2">
                <SubagentsSection ordered={subagents} onOpenAgent={onOpenAgent} />
              </div>
            )}
            {sources.length > 0 && (
              <div className="py-2">
                <SourcesSection sources={sources} />
              </div>
            )}
          </div>
        </motion.aside>
      )}
    </AnimatePresence>
  );
}
