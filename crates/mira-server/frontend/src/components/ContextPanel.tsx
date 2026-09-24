import { useState, useEffect, useRef } from 'react';
import { motion, AnimatePresence } from 'framer-motion';
import {
  ArrowUp,
  GitBranch,
  GitCommit,
  Monitor,
  Link,
  Check,
  ArrowRight,
  Circle,
  ChevronDown,
  ChevronRight,
  GitMerge,
  GitPullRequest,
  CornerDownLeft,
  Cloud,
  Copy,
} from 'lucide-react';
import { cn } from '../lib/utils';
import type { GitStatusView, SessionDiffView, BranchPrView } from '../api';
import { putCwd, createWorktree } from '../api';
import type { TaskItem, EnvironmentStatus, EnvironmentInfo } from '../types';
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

function envDisplayName(name: string): string {
  return name === 'e2b' ? 'Code Sandbox' : name;
}

function backendLabel(b: string): string {
  return b === 'e2b' ? 'Code Sandbox' : b;
}

function envIcon(backend: string) {
  if (backend === 'local') return Monitor;
  if (backend === 'scratch') return Copy;
  return Cloud;
}

function isLinkedWorktree(path: string, all: { path: string }[]): boolean {
  return all.some((o) => o.path !== path && path.startsWith(o.path + '/'));
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
  const [includeUnstaged, setIncludeUnstaged] = useState(false);
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
  const hasDirty = gitStatus.dirty;

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

      {/* Include unstaged toggle */}
      {hasDirty && (
        <button
          type="button"
          onClick={() => setIncludeUnstaged((v) => !v)}
          className="flex items-center justify-between gap-2 px-3 py-2.5 border-b border-white/[0.07] hover:bg-white/[0.04] transition-colors text-left"
        >
          <span className="text-[12.5px] text-white/60">Include unstaged changes</span>
          <div className="flex items-center gap-2">
            {(sessionDiff.added > 0 || sessionDiff.removed > 0) && (
              <span className="flex gap-1 font-mono text-[11px]">
                <span className="text-green-400/70">+{fmtNum(sessionDiff.added)}</span>
                <span className="text-red-400/70">-{fmtNum(sessionDiff.removed)}</span>
              </span>
            )}
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

function Row({
  icon,
  label,
  right,
  onClick,
  muted,
}: {
  icon: React.ReactNode;
  label: React.ReactNode;
  right?: React.ReactNode;
  onClick?: () => void;
  muted?: boolean;
}) {
  return (
    <div
      onClick={onClick}
      className={cn(
        'flex items-center gap-2.5 py-1',
        onClick && 'cursor-pointer hover:bg-white/[0.04] -mx-2 px-2 rounded transition-colors',
        muted ? 'text-white/30' : 'text-white/60',
      )}
    >
      <span className="shrink-0 text-white/30">{icon}</span>
      <span className="min-w-0 flex-1 truncate text-[13px] font-medium">{label}</span>
      {right && <span className="shrink-0 text-[12px]">{right}</span>}
    </div>
  );
}

function EnvironmentSection({
  gitStatus,
  sessionDiff,
  branchPr,
  environmentName,
  environment,
  environments,
  envSwitching,
  onSwitchEnvironment,
  onSwitchWorktree,
  onPush,
  onCommit,
}: {
  gitStatus: GitStatusView;
  sessionDiff: SessionDiffView;
  branchPr: BranchPrView | null;
  environmentName: string;
  environment: EnvironmentStatus | null;
  environments: EnvironmentInfo[];
  envSwitching: string | null;
  onSwitchEnvironment?: (target: string) => void;
  onSwitchWorktree?: (path: string, sessionId?: string) => void;
  onPush: () => Promise<void>;
  onCommit: (message: string, includeUnstaged: boolean, pushAfter: boolean) => Promise<void>;
}) {
  const [open, setOpen] = useState(true);
  const [commitOpen, setCommitOpen] = useState(false);
  const [envOpen, setEnvOpen] = useState(false);
  const [branchOpen, setBranchOpen] = useState(false);
  const [branchBusy, setBranchBusy] = useState(false);
  const [branchError, setBranchError] = useState<string | null>(null);
  const [newBranch, setNewBranch] = useState('');

  const hasSessionDiff = sessionDiff.added > 0 || sessionDiff.removed > 0;
  const showCommitRow = gitStatus.dirty || gitStatus.ahead > 0;

  const EnvIcon = environment ? envIcon(environment.backend) : Monitor;
  const hasMultipleEnvs = environments.length > 1;

  async function switchToWorktree(path: string) {
    setBranchBusy(true);
    setBranchError(null);
    try {
      const { session_id } = await putCwd(path);
      onSwitchWorktree?.(path, session_id ?? undefined);
      setBranchOpen(false);
    } catch (e) {
      setBranchError((e as Error).message);
    } finally {
      setBranchBusy(false);
    }
  }

  async function switchToBranch(branch: string) {
    setBranchBusy(true);
    setBranchError(null);
    try {
      const wt = await createWorktree(branch);
      const { session_id } = await putCwd(wt.path);
      onSwitchWorktree?.(wt.path, session_id ?? undefined);
      setBranchOpen(false);
    } catch (e) {
      setBranchError((e as Error).message);
    } finally {
      setBranchBusy(false);
    }
  }

  async function createAndSwitch() {
    if (!newBranch.trim()) return;
    await switchToBranch(newBranch.trim());
    setNewBranch('');
  }

  const primaryWorktree = gitStatus.worktrees.find((w) => !isLinkedWorktree(w.path, gitStatus.worktrees));
  const availableBranches = (gitStatus.branches ?? []).filter((b) => !b.in_worktree);

  return (
    <div>
      <SectionHeader label="Environment" open={open} onToggle={() => setOpen((v) => !v)} />
      <AnimatePresence initial={false}>
        {open && (
          <motion.div
            initial={{ height: 0, opacity: 0 }}
            animate={{ height: 'auto', opacity: 1, transition: { duration: 0.16 } }}
            exit={{ height: 0, opacity: 0, transition: { duration: 0.12 } }}
            className="overflow-hidden"
          >
            <div className="flex flex-col pb-1">
              {hasSessionDiff && (
                <Row
                  icon={<GitMerge className="size-3" />}
                  label="Changes"
                  right={
                    <span className="flex gap-1.5 font-mono">
                      <span className="text-green-400/80">+{fmtNum(sessionDiff.added)}</span>
                      <span className="text-red-400/80">-{fmtNum(sessionDiff.removed)}</span>
                    </span>
                  }
                />
              )}

              {/* Environment row with picker */}
              {hasMultipleEnvs && onSwitchEnvironment ? (
                <Popover open={envOpen} onOpenChange={setEnvOpen}>
                  <PopoverTrigger asChild>
                    <div className="flex items-center gap-2.5 py-1 cursor-pointer hover:bg-white/[0.04] -mx-2 px-2 rounded transition-colors text-white/60">
                      <span className="shrink-0 text-white/30"><EnvIcon className="size-3" /></span>
                      <span className="min-w-0 flex-1 truncate text-[13px] font-medium">
                        {envSwitching ? 'switching…' : envDisplayName(environmentName)}
                      </span>
                      <ChevronDown className="size-3 text-white/25 shrink-0" />
                    </div>
                  </PopoverTrigger>
                  <PopoverContent
                    className="w-72 p-1.5 overflow-hidden border-white/[0.10]"
                    align="end"
                    side="bottom"
                    style={{ backgroundColor: '#1c1c1e' }}
                  >
                    <div className="px-2 pb-1.5 pt-1 text-[10.5px] font-semibold uppercase tracking-widest text-white/30">
                      Environment
                    </div>
                    {envSwitching && (
                      <div className="px-2.5 pb-2 text-[11.5px] text-white/50 break-words">{envSwitching}</div>
                    )}
                    <div className="flex flex-col">
                      {environments.map((e) => {
                        const current = e.name === environment?.current;
                        const parked = environment?.parked.includes(e.name) ?? false;
                        const EIcon = envIcon(e.backend);
                        return (
                          <button
                            key={e.name}
                            type="button"
                            disabled={!!envSwitching || current}
                            onClick={() => { setEnvOpen(false); onSwitchEnvironment(e.name); }}
                            className={cn(
                              'flex items-start gap-2 rounded-md px-2.5 py-1.5 text-left text-[13px] transition-colors',
                              current ? 'text-white/80' : 'text-white/50 hover:bg-white/[0.06] hover:text-white/80',
                              envSwitching && !current && 'opacity-50',
                            )}
                          >
                            <EIcon className="size-3.5 shrink-0 mt-0.5 text-white/40" />
                            <span className="min-w-0 flex-1">
                              <span className="flex items-center gap-1.5">
                                <span className="truncate">{envDisplayName(e.name)}</span>
                                {e.backend !== 'local' && e.backend !== e.name && (
                                  <span className="rounded-sm bg-white/[0.08] px-1 text-[9.5px] uppercase tracking-wider text-white/30">
                                    {backendLabel(e.backend)}
                                  </span>
                                )}
                                {parked && (
                                  <span className="rounded-sm bg-blue-500/15 px-1 text-[9.5px] uppercase tracking-wider text-blue-400">paused</span>
                                )}
                              </span>
                              {e.description && (
                                <span className="block truncate text-[11px] text-white/30">
                                  {e.description.replace(/\bE2B\b/g, 'Code Sandbox')}
                                </span>
                              )}
                            </span>
                            {current && <span className="text-blue-400 text-xs shrink-0">✓</span>}
                          </button>
                        );
                      })}
                    </div>
                    <div className="px-2.5 pt-1.5 pb-1 text-[11px] text-white/25 border-t border-white/[0.07] mt-1">
                      Remote runs use a copy of this worktree; switching back merges the changes in.
                    </div>
                  </PopoverContent>
                </Popover>
              ) : (
                <Row icon={<EnvIcon className="size-3" />} label={envDisplayName(environmentName)} />
              )}

              {/* Branch row with picker */}
              {gitStatus.branch && (
                <Popover open={branchOpen} onOpenChange={setBranchOpen}>
                  <PopoverTrigger asChild>
                    <div className="flex items-center gap-2.5 py-1 cursor-pointer hover:bg-white/[0.04] -mx-2 px-2 rounded transition-colors text-white/60">
                      <span className="shrink-0 text-white/30"><GitBranch className="size-3" /></span>
                      <span className="min-w-0 flex-1 truncate text-[13px] font-medium">{gitStatus.branch}</span>
                      <ChevronDown className="size-3 text-white/25 shrink-0" />
                    </div>
                  </PopoverTrigger>
                  <PopoverContent
                    className="w-72 p-1.5 overflow-hidden border-white/[0.10]"
                    align="end"
                    side="bottom"
                    style={{ backgroundColor: '#1c1c1e' }}
                  >
                    <div className="px-2 pb-1.5 pt-1 text-[10.5px] font-semibold uppercase tracking-widest text-white/30">
                      Working tree
                    </div>
                    {branchError && (
                      <div className="px-2.5 pb-2 text-[11.5px] text-red-400/80 break-words">{branchError}</div>
                    )}
                    <div className="flex flex-col">
                      {gitStatus.worktrees.map((w) => (
                        <button
                          key={w.path}
                          type="button"
                          disabled={branchBusy || w.is_current}
                          onClick={() => switchToWorktree(w.path)}
                          className={cn(
                            'flex items-center gap-2 rounded-md px-2.5 py-1.5 text-left text-[13px] transition-colors',
                            w.is_current ? 'text-white/80' : 'text-white/50 hover:bg-white/[0.06] hover:text-white/80',
                            branchBusy && !w.is_current && 'opacity-50',
                          )}
                        >
                          <GitBranch className="size-3.5 shrink-0 text-white/40" />
                          <span className="min-w-0 flex-1 truncate">{w.branch ?? 'detached'}</span>
                          {w.is_current && <span className="text-blue-400 text-xs shrink-0">✓</span>}
                          {primaryWorktree && w.path === primaryWorktree.path && (
                            <span className="rounded-sm bg-white/[0.08] px-1 text-[9.5px] uppercase tracking-wider text-white/30">main</span>
                          )}
                        </button>
                      ))}
                    </div>
                    {availableBranches.length > 0 && (
                      <div className="mt-1.5 border-t border-white/[0.07] pt-1.5">
                        <div className="px-2.5 pb-1 text-[10.5px] font-semibold uppercase tracking-widest text-white/30">
                          Switch to branch
                        </div>
                        <div className="max-h-48 overflow-y-auto flex flex-col">
                          {availableBranches.map((b) => (
                            <button
                              key={`${b.is_remote ? 'r' : 'l'}:${b.name}`}
                              type="button"
                              disabled={branchBusy}
                              onClick={() => switchToBranch(b.name)}
                              className={cn(
                                'flex items-center gap-2 rounded-md px-2.5 py-1.5 text-left text-[13px] transition-colors',
                                'text-white/50 hover:bg-white/[0.06] hover:text-white/80',
                                branchBusy && 'opacity-50',
                              )}
                              title={b.upstream ? `tracks ${b.upstream}` : b.name}
                            >
                              <GitBranch className="size-3.5 shrink-0 text-white/30" />
                              <span className="min-w-0 flex-1 truncate">{b.name}</span>
                              {b.is_remote && (
                                <span className="rounded-sm bg-white/[0.08] px-1 text-[9.5px] uppercase tracking-wider text-white/30">remote</span>
                              )}
                            </button>
                          ))}
                        </div>
                      </div>
                    )}
                    <div className="mt-1.5 border-t border-white/[0.07] pt-1.5">
                      <div className="px-2.5 pb-1 text-[10.5px] font-semibold uppercase tracking-widest text-white/30">
                        New worktree
                      </div>
                      <div className="flex items-center gap-1.5 px-1.5">
                        <input
                          value={newBranch}
                          onChange={(e) => setNewBranch(e.target.value)}
                          onKeyDown={(e) => { if (e.key === 'Enter') { e.preventDefault(); void createAndSwitch(); } }}
                          placeholder="branch name"
                          disabled={branchBusy}
                          className="flex-1 min-w-0 rounded-md bg-white/[0.06] border border-white/[0.10] px-2 py-1 text-[12.5px] text-white/80 placeholder-white/25 outline-none focus:border-white/30"
                        />
                        <button
                          type="button"
                          onClick={() => void createAndSwitch()}
                          disabled={branchBusy || !newBranch.trim()}
                          className="rounded-md bg-white/90 px-2.5 py-1 text-[12px] text-black transition-opacity hover:opacity-90 disabled:opacity-30"
                        >
                          Add
                        </button>
                      </div>
                    </div>
                  </PopoverContent>
                </Popover>
              )}

              {gitStatus.last_commit && (
                <Row
                  icon={<GitCommit className="size-3" />}
                  label={gitStatus.last_commit.length > 28 ? gitStatus.last_commit.slice(0, 28) + '…' : gitStatus.last_commit}
                  muted
                />
              )}
              {branchPr && (
                <Row
                  icon={
                    <GitPullRequest
                      className="size-3"
                      style={{ color: prStateColor(branchPr.state, branchPr.isDraft) }}
                    />
                  }
                  label={
                    <span className="flex items-center gap-1.5">
                      <span className="text-white/60">#{branchPr.number}</span>
                      <span className="truncate text-white/50">{branchPr.title.length > 20 ? branchPr.title.slice(0, 20) + '…' : branchPr.title}</span>
                    </span>
                  }
                  onClick={() => window.open(branchPr.url, '_blank', 'noopener,noreferrer')}
                />
              )}
              {showCommitRow && gitStatus.branch && (
                <Popover open={commitOpen} onOpenChange={setCommitOpen}>
                  <PopoverTrigger asChild>
                    <div
                      className="flex items-center gap-2.5 py-1 cursor-pointer hover:bg-white/[0.04] -mx-2 px-2 rounded transition-colors text-white/60"
                    >
                      <span className="shrink-0 text-white/30"><ArrowUp className="size-3" /></span>
                      <span className="min-w-0 flex-1 truncate text-[13px] font-medium">
                        {gitStatus.ahead > 0 ? `Commit or push ↑${gitStatus.ahead}` : 'Commit changes'}
                      </span>
                    </div>
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

export type ContextPanelProps = {
  sessionTitle?: string;
  tasks: TaskItem[];
  subagentState: Map<string, SubagentStreamState>;
  entries: Entry[];
  gitStatus: GitStatusView | null;
  sessionDiff: SessionDiffView;
  branchPr: BranchPrView | null;
  environmentName: string;
  environment: EnvironmentStatus | null;
  environments: EnvironmentInfo[];
  envSwitching: string | null;
  onSwitchEnvironment?: (target: string) => void;
  onSwitchWorktree?: (path: string, sessionId?: string) => void;
  onOpenAgent: (callId: string) => void;
  onPush: () => Promise<void>;
  onCommit: (message: string, includeUnstaged: boolean, pushAfter: boolean) => Promise<void>;
};

export function ContextPanel({
  tasks,
  subagentState,
  entries,
  gitStatus,
  sessionDiff,
  branchPr,
  environmentName,
  environment,
  environments,
  envSwitching,
  onSwitchEnvironment,
  onSwitchWorktree,
  onOpenAgent,
  onPush,
  onCommit,
}: ContextPanelProps) {
  const [containerWidth, setContainerWidth] = useState(window.innerWidth);
  useEffect(() => {
    const onResize = () => setContainerWidth(window.innerWidth);
    window.addEventListener('resize', onResize);
    return () => window.removeEventListener('resize', onResize);
  }, []);

  const sources = extractSources(entries);
  const hasAnything =
    tasks.length > 0 ||
    gitStatus?.in_repo ||
    subagentState.size > 0 ||
    sources.length > 0;

  if (!hasAnything || containerWidth < MIN_WIDTH_TO_SHOW) return null;

  return (
    <motion.aside
      initial={{ opacity: 0, scale: 0.96, y: -6 }}
      animate={{ opacity: 1, scale: 1, y: 0, transition: { duration: 0.18, ease: 'easeOut' } }}
      exit={{ opacity: 0, scale: 0.96, y: -4, transition: { duration: 0.13 } }}
      className="absolute top-3 right-3 z-20 flex flex-col overflow-hidden rounded-xl"
      style={{
        width: PANEL_WIDTH,
        maxHeight: 'calc(100% - 24px)',
        backgroundColor: '#1c1c1e',
        boxShadow: '0 8px 32px rgba(0,0,0,0.45), 0 2px 8px rgba(0,0,0,0.3)',
      }}
    >
      <div className="overflow-y-auto px-4 py-0.5 flex flex-col divide-y divide-white/[0.07]">
        <AnimatePresence mode="popLayout">
          {[
            tasks.length > 0 && (
              <motion.div
                key="progress"
                initial={{ opacity: 0, y: -4 }}
                animate={{ opacity: 1, y: 0, transition: { duration: 0.16 } }}
                exit={{ opacity: 0, y: -4, transition: { duration: 0.12 } }}
                className="py-2"
              >
                <ProgressSection tasks={tasks} />
              </motion.div>
            ),
            gitStatus?.in_repo && (
              <motion.div
                key="env"
                initial={{ opacity: 0, y: -4 }}
                animate={{ opacity: 1, y: 0, transition: { duration: 0.16 } }}
                exit={{ opacity: 0, y: -4, transition: { duration: 0.12 } }}
                className="py-2"
              >
                <EnvironmentSection
                  gitStatus={gitStatus}
                  sessionDiff={sessionDiff}
                  branchPr={branchPr}
                  environmentName={environmentName}
                  environment={environment}
                  environments={environments}
                  envSwitching={envSwitching}
                  onSwitchEnvironment={onSwitchEnvironment}
                  onSwitchWorktree={onSwitchWorktree}
                  onPush={onPush}
                  onCommit={onCommit}
                />
              </motion.div>
            ),
            subagentState.size > 0 && (
              <motion.div
                key="agents"
                initial={{ opacity: 0, y: -4 }}
                animate={{ opacity: 1, y: 0, transition: { duration: 0.16 } }}
                exit={{ opacity: 0, y: -4, transition: { duration: 0.12 } }}
                className="py-2"
              >
                <SubagentsSection
                  subagentState={subagentState}
                  entries={entries}
                  onOpenAgent={onOpenAgent}
                />
              </motion.div>
            ),
            sources.length > 0 && (
              <motion.div
                key="sources"
                initial={{ opacity: 0, y: -4 }}
                animate={{ opacity: 1, y: 0, transition: { duration: 0.16 } }}
                exit={{ opacity: 0, y: -4, transition: { duration: 0.12 } }}
                className="py-2"
              >
                <SourcesSection entries={entries} />
              </motion.div>
            ),
          ].filter(Boolean)}
        </AnimatePresence>
      </div>
    </motion.aside>
  );
}
