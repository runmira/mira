import { useState, useEffect, useRef } from 'react';
import { motion, AnimatePresence } from 'framer-motion';
import {
  ArrowUp,
  GitBranch,
  Link,
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
} from 'lucide-react';
import { cn } from '../lib/utils';
import type { GitStatusView, SessionDiffView, BranchPrView } from '../api';
import type { TaskItem } from '../types';
import type { SubagentStreamState, Entry } from '../App';
import { Popover, PopoverContent, PopoverTrigger } from './ui/popover';
import { identityFor } from './AgentCard';

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
      const url = m[0].replace(/[.,;:!?*`%#]+$/, '');
      if (!seen.has(url)) { seen.add(url); urls.push(url); }
    }
  }
  return urls;
}

function urlLabel(url: string): string {
  try {
    const u = new URL(url);
    const path = u.pathname.length > 1 ? u.pathname : '';
    return u.hostname + path;
  } catch {
    return url;
  }
}

function agentColor(s: string): string {
  const hash = [...s].reduce((h, c) => (h * 31 + c.charCodeAt(0)) | 0, 0);
  const colors = ['#e06c75', '#e5c07b', '#98c379', '#56b6c2', '#61afef', '#c678dd'];
  return colors[Math.abs(hash) % colors.length];
}

function formatAgentName(raw: string): string {
  return raw
    .split(/[-_\s]+/)
    .map((w) => w.charAt(0).toUpperCase() + w.slice(1))
    .join(' ');
}

function agentDisplayInfo(
  callId: string,
  state: SubagentStreamState,
): { name: string; badge: string | null; description: string } {
  const rawCat = state.agentCategory ?? null;
  const promptStr = (state.prompt ?? '').trim();

  // Use the same codename as the subagent panel tab so the entry is
  // immediately recognisable across both surfaces.
  const name = identityFor(callId).name;
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
      <span className="text-[11px] font-bold uppercase tracking-widest text-white/40 group-hover:text-white/60 transition-colors">
        {label}
      </span>
      <div className="flex items-center gap-1.5">
        {right}
        {open
          ? <ChevronDown className="size-3.5 text-white/25" />
          : <ChevronRight className="size-3.5 text-white/25" />}
      </div>
    </button>
  );
}

/* ------------------------------------------------------------------ */
/* Progress                                                            */
/* ------------------------------------------------------------------ */

function ProgressSection({ tasks }: { tasks: TaskItem[] }) {
  const [open, setOpen] = useState(true);
  const total = tasks.length;
  const done = tasks.filter((t) => t.status === 'completed' || t.status === 'deleted').length;
  const active = tasks.find((t) => t.status === 'in_progress');

  return (
    <div>
      <SectionHeader
        label="Progress"
        right={<span className="text-[10.5px] text-white/25">{done}/{total}</span>}
        open={open}
        onToggle={() => setOpen((v) => !v)}
      />
      <AnimatePresence initial={false}>
        {open && (
          <motion.div
            initial={{ height: 0, opacity: 0 }}
            animate={{ height: 'auto', opacity: 1, transition: { duration: 0.16 } }}
            exit={{ height: 0, opacity: 0, transition: { duration: 0.12 } }}
            className="overflow-hidden"
          >
            <div className="flex flex-col gap-0.5 pb-1">
              <AnimatePresence>
                {tasks.map((task, i) => {
                  const isDone = task.status === 'completed' || task.status === 'deleted';
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
                        isDone ? 'text-white/25' : isCurrent ? 'text-white/85' : 'text-white/50',
                      )}
                    >
                      <span className="mt-[3px] shrink-0">
                        {isDone
                          ? <Check className="size-3 text-green-500/60" strokeWidth={2.5} />
                          : isCurrent
                          ? <ArrowRight className="size-3 text-blue-400" strokeWidth={2} />
                          : <Circle className="size-3 text-white/20" />}
                      </span>
                      <span className={cn('min-w-0', isDone && 'line-through decoration-white/15')}>
                        {task.subject}
                      </span>
                    </motion.div>
                  );
                })}
              </AnimatePresence>
              {active && (
                <div className="mt-1.5 h-px bg-white/10 rounded-full overflow-hidden">
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
    <div className="flex flex-col" style={{ backgroundColor: '#1c1c1e' }}>
      {/* Branch row */}
      <div className="flex items-center gap-2 px-3 py-2.5 border-b border-white/[0.07]">
        <GitBranch className="size-3.5 shrink-0 text-white/40" />
        <span className="text-[13px] font-medium text-white/70">{branch}</span>
      </div>

      {/* Commit message */}
      <div className="px-3 pt-2.5 pb-2 border-b border-white/[0.07]">
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
          className="w-full resize-none rounded-md bg-white/[0.06] px-2.5 py-2 text-[12.5px] text-white/80 placeholder-white/25 outline-none focus:ring-1 focus:ring-white/20 transition-shadow"
        />
      </div>

      {/* Include untracked (new) files toggle */}
      {newFiles > 0 && (
        <button
          type="button"
          onClick={() => setIncludeUnstaged((v) => !v)}
          className="flex items-center justify-between gap-2 px-3 py-2.5 border-b border-white/[0.07] hover:bg-white/[0.04] transition-colors text-left"
        >
          <span className="text-[12.5px] text-white/60">
            Include new files <span className="text-white/30">({newFiles})</span>
          </span>
          <div className="flex items-center gap-2">
            <div className={cn(
              'w-8 h-4 rounded-full transition-colors flex items-center',
              includeUnstaged ? 'bg-blue-500' : 'bg-white/[0.12]',
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
        <div className="px-3 py-2 text-[11.5px] text-red-400/80 border-b border-white/[0.07]">
          {error}
        </div>
      )}

      {/* Action rows */}
      <button
        type="button"
        disabled={!!loading}
        onClick={() => void handleCommit(false)}
        className="flex items-center justify-between px-3 py-2.5 border-b border-white/[0.07] hover:bg-white/[0.04] disabled:opacity-50 transition-colors"
      >
        <span className="text-[13px] font-medium text-white/75">
          {loading === 'commit' ? 'Committing…' : 'Commit'}
        </span>
        <span className="flex items-center gap-0.5 text-[11px] text-white/25">
          <span>⌘</span><CornerDownLeft className="size-3" />
        </span>
      </button>

      <button
        type="button"
        disabled={!!loading}
        onClick={() => void handleCommit(true)}
        className="flex items-center justify-between px-3 py-2.5 border-b border-white/[0.07] hover:bg-white/[0.04] disabled:opacity-50 transition-colors"
      >
        <span className="text-[13px] font-medium text-white/75">
          {loading === 'commit-push' ? 'Committing & pushing…' : 'Commit and push'}
        </span>
      </button>

      <button
        type="button"
        disabled={!!loading || !hasAhead}
        onClick={() => void handlePush()}
        className="flex items-center justify-between px-3 py-2.5 hover:bg-white/[0.04] disabled:opacity-40 transition-colors"
      >
        <span className={cn(
          'text-[13px] font-medium',
          hasAhead ? 'text-white/75' : 'text-white/30',
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
    (sessionDiff.uncommitted ?? 0) > 0 ||
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
}: {
  gitStatus: GitStatusView;
  sessionDiff: SessionDiffView;
  branchPr: BranchPrView | null;
  sessionCommitted: boolean;
  onPush: () => Promise<void>;
  onCommit: (message: string, includeUnstaged: boolean, pushAfter: boolean) => Promise<void>;
}) {
  const [open, setOpen] = useState(true);
  const [commitOpen, setCommitOpen] = useState(false);
  const [pushing, setPushing] = useState(false);

  // Only what this session did: its own uncommitted files, and pushes for
  // commits it made — not whatever else happens to be dirty in the repo.
  const pending = sessionDiff.uncommitted ?? 0;
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
            className="overflow-hidden"
          >
            <div className="flex flex-col gap-1.5 pb-1.5">
              {pending > 0 && gitStatus.branch && (
                <div className="flex items-center gap-2 rounded-lg bg-white/[0.035] px-2.5 py-2">
                  <GitMerge className="size-3.5 shrink-0 text-white/35" />
                  <span className="min-w-0 flex-1">
                    <span className="block text-[12.5px] font-medium text-white/70">
                      {pending} file{pending === 1 ? '' : 's'} changed
                    </span>
                    <span className="flex gap-1.5 font-mono text-[11px]">
                      <span className="text-green-400/80">+{fmtNum(sessionDiff.added)}</span>
                      <span className="text-red-400/80">−{fmtNum(sessionDiff.removed)}</span>
                    </span>
                  </span>
                  <Popover open={commitOpen} onOpenChange={setCommitOpen}>
                    <PopoverTrigger asChild>
                      <button
                        type="button"
                        className="shrink-0 rounded-md bg-white/90 px-2.5 py-1 text-[12px] font-medium text-black transition-opacity hover:opacity-90"
                      >
                        Commit
                      </button>
                    </PopoverTrigger>
                    <PopoverContent
                      className="w-72 p-0 overflow-hidden border-white/[0.10]"
                      align="end"
                      side="bottom"
                      style={{ backgroundColor: '#1c1c1e' }}
                    >
                      <CommitDialog
                        branch={gitStatus.branch}
                        gitStatus={gitStatus}
                        sessionDiff={sessionDiff}
                        onCommit={onCommit}
                        onPush={onPush}
                        onClose={() => setCommitOpen(false)}
                      />
                    </PopoverContent>
                  </Popover>
                </div>
              )}

              {pending === 0 && canPush && (
                <button
                  type="button"
                  disabled={pushing}
                  onClick={async () => {
                    setPushing(true);
                    try { await onPush(); } finally { setPushing(false); }
                  }}
                  className="flex items-center gap-2 rounded-lg bg-white/[0.035] px-2.5 py-2 text-left transition-colors hover:bg-white/[0.06] disabled:opacity-50"
                >
                  <ArrowUp className="size-3.5 shrink-0 text-white/35" />
                  <span className="flex-1 text-[12.5px] font-medium text-white/70">
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
                  className="-mx-2 flex items-center gap-2 rounded px-2 py-1 text-white/60 transition-colors hover:bg-white/[0.04] hover:text-white/85"
                >
                  <GitPullRequest
                    className="size-3.5 shrink-0"
                    style={{ color: prStateColor(branchPr.state, branchPr.isDraft) }}
                  />
                  <span className="shrink-0 text-[12.5px] text-white/45">#{branchPr.number}</span>
                  <span className="min-w-0 flex-1 truncate text-[12.5px]">{branchPr.title}</span>
                  <ArrowUpRight className="size-3 shrink-0 text-white/30" />
                </a>
              )}
            </div>
          </motion.div>
        )}
      </AnimatePresence>
    </div>
  );
}

/* ------------------------------------------------------------------ */
/* Subagents                                                           */
/* ------------------------------------------------------------------ */

function SubagentsSection({
  subagentState,
  entries,
  onOpenAgent,
}: {
  subagentState: Map<string, SubagentStreamState>;
  entries: Entry[];
  onOpenAgent: (callId: string) => void;
}) {
  const [open, setOpen] = useState(true);

  const ordered: { callId: string; state: SubagentStreamState }[] = [];
  for (const e of entries) {
    if (e.kind === 'tool') {
      const s = subagentState.get(e.call.id);
      if (s) ordered.push({ callId: e.call.id, state: s });
    }
  }
  if (ordered.length === 0) return null;

  return (
    <div>
      <SectionHeader label="Subagents" open={open} onToggle={() => setOpen((v) => !v)} />
      <AnimatePresence initial={false}>
        {open && (
          <motion.div
            initial={{ height: 0, opacity: 0 }}
            animate={{ height: 'auto', opacity: 1, transition: { duration: 0.16 } }}
            exit={{ height: 0, opacity: 0, transition: { duration: 0.12 } }}
            className="overflow-hidden"
          >
            <div className="flex flex-col gap-0.5 pb-1">
              <AnimatePresence>
                {ordered.map(({ callId, state }, i) => {
                  const color = agentColor(callId);
                  const { name, badge, description } = agentDisplayInfo(callId, state);
                  return (
                    <motion.button
                      key={callId}
                      custom={i}
                      variants={itemVariants}
                      initial="hidden"
                      animate="visible"
                      type="button"
                      onClick={() => onOpenAgent(callId)}
                      className="flex items-start gap-2.5 py-1.5 -mx-2 px-2 rounded text-left hover:bg-white/[0.04] transition-colors w-full"
                    >
                      <span
                        className={cn('size-2 rounded-full shrink-0 mt-[5px]', !state.done && 'animate-pulse')}
                        style={{ backgroundColor: color }}
                      />
                      <span className="flex-1 min-w-0 flex flex-col gap-0.5">
                        <span className="flex items-baseline gap-1.5 min-w-0">
                          <span className="truncate text-[13px] font-semibold text-white/75 leading-snug">{name}</span>
                          {badge && <span className="shrink-0 text-[11px] font-medium text-white/30">({badge})</span>}
                        </span>
                        {description && (
                          <span className="truncate text-[12px] font-medium text-white/40 leading-snug">{description}</span>
                        )}
                      </span>
                      {!state.done && <span className="text-[10px] text-white/20 shrink-0 mt-0.5">running</span>}
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

function SourcesSection({ entries }: { entries: Entry[] }) {
  const [open, setOpen] = useState(true);
  const sources = extractSources(entries);
  if (sources.length === 0) return null;

  return (
    <div>
      <SectionHeader label="Sources" open={open} onToggle={() => setOpen((v) => !v)} />
      <AnimatePresence initial={false}>
        {open && (
          <motion.div
            initial={{ height: 0, opacity: 0 }}
            animate={{ height: 'auto', opacity: 1, transition: { duration: 0.16 } }}
            exit={{ height: 0, opacity: 0, transition: { duration: 0.12 } }}
            className="overflow-hidden"
          >
            <div className="flex flex-col gap-0.5 pb-1">
              <AnimatePresence>
                {sources.slice(0, 8).map((url, i) => (
                  <motion.a
                    key={url}
                    custom={i}
                    variants={itemVariants}
                    initial="hidden"
                    animate="visible"
                    href={url}
                    target="_blank"
                    rel="noopener noreferrer"
                    title={url}
                    className="flex items-center gap-2 py-[3px] -mx-2 px-2 rounded hover:bg-white/[0.06] transition-colors group"
                  >
                    <Link className="size-3 shrink-0 text-blue-400/50 group-hover:text-blue-400 transition-colors" />
                    <span className="min-w-0 truncate text-[12.5px] text-blue-400/70 group-hover:text-blue-400 transition-colors underline underline-offset-2 decoration-blue-400/30">
                      {urlLabel(url)}
                    </span>
                  </motion.a>
                ))}
              </AnimatePresence>
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
    p.tasks.length > 0 ||
    workspaceHasContent(p.gitStatus, p.sessionDiff, p.branchPr, p.sessionCommitted) ||
    p.subagentState.size > 0 ||
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
  onPush: () => Promise<void>;
  onCommit: (message: string, includeUnstaged: boolean, pushAfter: boolean) => Promise<void>;
};

const cardStyle: React.CSSProperties = {
  backgroundColor: 'rgba(28, 28, 30, 0.92)',
  backdropFilter: 'blur(14px)',
  WebkitBackdropFilter: 'blur(14px)',
  boxShadow: '0 12px 40px rgba(0,0,0,0.45), 0 2px 8px rgba(0,0,0,0.3)',
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
    onPush,
    onCommit,
  } = props;
  const fits = useContextPanelFits();
  const sources = extractSources(entries);

  if (!contextPanelHasContent(props) || !fits) return null;

  const done = tasks.filter((t) => t.status === 'completed' || t.status === 'deleted').length;
  const pending = sessionDiff.uncommitted ?? 0;
  const agentsRunning = [...subagentState.values()].some((s) => !s.done);

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
          className="absolute top-3 right-3 z-20 flex max-w-[340px] items-center gap-2.5 rounded-full border border-white/[0.08] px-3 py-1.5 text-[12px] text-white/55 transition-colors hover:border-white/[0.16] hover:text-white/80"
          style={cardStyle}
        >
          <PanelRightOpen className="size-3.5 shrink-0 text-white/40" />
          <span className="font-medium text-white/65">Context</span>
          {pending > 0 && (
            <span className="flex shrink-0 gap-1 font-mono text-[11px]">
              <span className="text-green-400/80">+{fmtNum(sessionDiff.added)}</span>
              <span className="text-red-400/80">−{fmtNum(sessionDiff.removed)}</span>
            </span>
          )}
          {tasks.length > 0 && (
            <span className="flex shrink-0 items-center gap-1">
              <Check className="size-3 text-green-500/60" strokeWidth={2.5} />
              {done}/{tasks.length}
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
          className="absolute top-3 right-3 z-20 flex flex-col overflow-hidden rounded-2xl border border-white/[0.08]"
          style={{ ...cardStyle, width: PANEL_WIDTH, maxHeight: 'calc(100% - 24px)', transformOrigin: 'top right' }}
        >
          <div className="flex items-center justify-between px-4 pt-3 pb-1">
            <span className="text-[12.5px] font-semibold text-white/75">Context</span>
            <button
              type="button"
              onClick={() => onOpenChange(false)}
              title="Collapse"
              className="-mr-1.5 rounded-md p-1 text-white/35 transition-colors hover:bg-white/[0.06] hover:text-white/75"
            >
              <PanelRightClose className="size-3.5" />
            </button>
          </div>
          <div className="overflow-y-auto px-4 pb-1.5 flex flex-col divide-y divide-white/[0.07]">
            {tasks.length > 0 && (
              <div className="py-2">
                <ProgressSection tasks={tasks} />
              </div>
            )}
            {gitStatus && workspaceHasContent(gitStatus, sessionDiff, branchPr, sessionCommitted) && (
              <div className="py-2">
                <WorkspaceSection
                  gitStatus={gitStatus}
                  sessionDiff={sessionDiff}
                  branchPr={branchPr}
                  sessionCommitted={sessionCommitted}
                  onPush={onPush}
                  onCommit={onCommit}
                />
              </div>
            )}
            {subagentState.size > 0 && (
              <div className="py-2">
                <SubagentsSection
                  subagentState={subagentState}
                  entries={entries}
                  onOpenAgent={onOpenAgent}
                />
              </div>
            )}
            {sources.length > 0 && (
              <div className="py-2">
                <SourcesSection entries={entries} />
              </div>
            )}
          </div>
        </motion.aside>
      )}
    </AnimatePresence>
  );
}
