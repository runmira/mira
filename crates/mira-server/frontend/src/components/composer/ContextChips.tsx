import { Popover, PopoverContent, PopoverTrigger } from '@/components/ui/popover';
import { cn } from '@/lib/utils';
import { m } from 'framer-motion';
import { Cloud, Copy, GitBranch, Lightbulb, Loader, Monitor, Target } from 'lucide-react';
import { useEffect, useRef, useState } from 'react';
import { createWorktree, getGitStatus, putCwd, type GitStatusView } from '../../api';
import type { EnvironmentInfo, EnvironmentStatus, Goal } from '../../types';
import { UsageRing, type UsageRingData } from '../UsageRing';
import { reportWorkspaceSetup, type WorkspaceSetup } from '../WorkspaceSetupCard';
// (shortToolLabel removed with the composer approval footer — the
// inline card renders its own label via ToolCard's `summarize`.)

/* ---------- plan chip (visible only while plan mode is on) ---------- */

/** Active-state indicator + one-click exit. Renders only when plan mode is
 *  on; clicking it drops the user back to their prior mode. Entering plan
 *  mode happens via `/plan` (autocompletes) or the mode picker. */
export function PlanChip({ onExit }: { onExit: () => void }) {
  return (
    <button
      type="button"
      onClick={onExit}
      title="Plan mode on — click to exit"
      className="inline-flex items-center gap-1.5 rounded-full bg-mira-blue/15 px-2.5 py-1.5 text-[12.5px] text-mira-blue transition-colors hover:bg-mira-blue/25"
    >
      <Lightbulb className="size-3 shrink-0" />
      <span>Plan</span>
      <span className="text-mira-blue/70">·</span>
    </button>
  );
}

/* ---------- goal compose chip (visible while typing a new goal condition) ---------- */

/** Sibling of [`PlanChip`]: renders while the user is composing the
 *  goal condition (after `/goal`). Uses a dashed border to signal
 *  "empty / awaiting input" so it visually distinguishes from an
 *  active goal. Click cancels the compose flow. */
export function GoalComposeChip({ onCancel }: { onCancel: () => void }) {
  return (
    <button
      type="button"
      onClick={onCancel}
      title="Composing a goal — press Enter to set, or click to cancel"
      className="inline-flex items-center gap-1.5 rounded-full border border-dashed border-mira-purple/50 bg-mira-purple/10 px-2.5 py-1.5 text-[12.5px] text-mira-purple transition-colors hover:bg-mira-purple/20"
    >
      <Target className="size-3 shrink-0" />
      <span className="font-medium">Goal</span>
      <span className="text-mira-purple/70">·</span>
      <span className="opacity-90">describe the condition ↵</span>
    </button>
  );
}

/* ---------- goal chip (visible while a `/goal` is set) ---------- */

/** Mirrors [`PlanChip`]'s pattern so autonomy shows up in the same spot,
 *  just tinted purple to visually distinguish "goal-directed" from
 *  "plan-only". Body shows iteration progress + a truncated condition
 *  so the user sees at a glance how many rounds the autonomous loop
 *  has consumed. Click clears the goal (same as `/goal clear`). */
export function GoalChip({ goal, onClear }: { goal: Goal; onClear: () => void }) {
  const running = goal.status === 'active';
  const shortCond =
    goal.condition.length > 48 ? `${goal.condition.slice(0, 48).trim()}…` : goal.condition;
  // Non-active statuses keep the chip visible with a tint so the user
  // can see the terminal state at a glance without having to scroll to
  // the GoalPanel; hover still says "click to clear".
  const tone = running
    ? 'bg-mira-purple/15 text-mira-purple hover:bg-mira-purple/25'
    : goal.status === 'met'
      ? 'bg-emerald-500/15 text-emerald-300 hover:bg-emerald-500/25'
      : goal.status === 'needs_user' || goal.status === 'exhausted'
        ? 'bg-amber-500/15 text-amber-300 hover:bg-amber-500/25'
        : goal.status === 'impossible'
          ? 'bg-red-500/15 text-red-300 hover:bg-red-500/25'
          : 'bg-secondary/60 text-muted-foreground hover:bg-secondary';
  const title = running
    ? `Goal running (${goal.iterations}/${goal.max_iterations}) — click to clear`
    : `Goal ${goal.status.replace('_', ' ')} — click to clear`;
  return (
    <button
      type="button"
      onClick={onClear}
      title={title}
      className={cn(
        'inline-flex items-center gap-1.5 rounded-full px-2.5 py-1.5 text-[12.5px] transition-colors',
        tone,
      )}
    >
      <Target className="size-3 shrink-0" />
      <span className="font-medium">Goal</span>
      <span className="opacity-70">·</span>
      <span className="tabular-nums opacity-90">
        {goal.iterations}/{goal.max_iterations}
      </span>
      <span className="opacity-70">·</span>
      <span className="max-w-[240px] truncate opacity-90">{shortCond}</span>
    </button>
  );
}

/* ---------- session usage readout (tokens + $ cost) ---------- */

/**
 * Footer readout for the running session. Cost is the headline — it's the
 * lever people actually reason about — with token counts as smaller
 * context on the left. Hover reveals a fuller breakdown so power users can
 * still see the raw numbers without them shouting in the chrome.
 *
 * Renders nothing at all while the session is empty, so a fresh chat
 * doesn't lie by showing "$0.00" before the first turn.
 */
/** The limit closest to running out, with the share left (0–1). */

export function ComposerContextDock({
  environment,
  environments,
  envSwitching,
  envDisabled,
  onSwitchEnvironment,
  usageRing,
  cwd,
  onCwdSwitched,
}: {
  environment: EnvironmentStatus | null;
  environments: EnvironmentInfo[];
  envSwitching: string | null;
  envDisabled: boolean;
  onSwitchEnvironment?: (target: string) => void;
  usageRing?: UsageRingData | null;
  cwd: string;
  onCwdSwitched?: (path: string, sessionId?: string) => void;
}) {
  return (
    <m.div
      layout
      // Environment, usage and worktree: desk-side controls, and on a phone
      // the height is better spent on the transcript.
      className="-mb-3 w-full max-w-3xl px-4 sm:px-5 max-md:hidden"
      initial={{ opacity: 0, y: 4 }}
      animate={{ opacity: 1, y: 0 }}
      transition={{ duration: 0.18, ease: [0.4, 0, 0.2, 1] }}
    >
      <div className="flex min-h-10 items-center gap-2 rounded-t-[18px] rounded-b-none border border-b-0 border-border/55 bg-white px-3 pb-4 pt-1.5 shadow-[0_8px_24px_-22px_rgba(15,23,42,0.45)] dark:border-fg/[0.055] dark:bg-secondary/85 dark:shadow-[0_10px_28px_-24px_rgba(0,0,0,0.75)] backdrop-blur-xl">
        <EnvironmentChip
          status={environment}
          environments={environments}
          switching={envSwitching}
          disabled={envDisabled}
          onSwitch={onSwitchEnvironment}
        />
        <span className="min-w-4 flex-1" />
        {usageRing && <UsageRing data={usageRing} />}
        <WorktreeChip cwd={cwd} onCwdSwitched={onCwdSwitched} />
      </div>
    </m.div>
  );
}

/* ---------- environment chip (local ↔ remote environments) ---------- */

/** This machine, a scratch copy on this machine, or a cloud sandbox. */
export function envIcon(backend: string) {
  if (backend === 'local') return Monitor;
  if (backend === 'scratch') return Copy;
  return Cloud;
}

/** Where the session's tools run. Switching to a remote environment
 *  uploads the current worktree (uncommitted edits too); switching back
 *  merges the changes into the same worktree. The worktree stays the
 *  source of truth, so the worktree chip beside this one is unaffected. */
export function EnvironmentChip({
  status,
  environments,
  switching,
  disabled,
  onSwitch,
}: {
  status: EnvironmentStatus | null;
  environments: EnvironmentInfo[];
  switching: string | null;
  disabled: boolean;
  onSwitch?: (target: string) => void;
}) {
  const [open, setOpen] = useState(false);
  if (!status || !onSwitch) return null;
  const remote = status.current !== 'local';
  const Icon = envIcon(status.backend);
  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>
        <button
          type="button"
          className={cn(
            'inline-flex items-center gap-1.5 rounded-full px-2.5 py-1.5 text-[12.5px] transition-colors max-w-[11rem] min-w-0',
            remote
              ? 'text-mira-blue hover:bg-mira-blue/10'
              : 'text-muted-foreground hover:bg-mira-elev2 hover:text-foreground',
          )}
          title={
            switching
              ? switching
              : remote
                ? `tools run in ${status.current} (${status.backend})${status.workspace ? ` at ${status.workspace}` : ''}`
                : 'tools run on this machine'
          }
        >
          {switching ? (
            <Loader className="size-3 shrink-0 animate-spin" />
          ) : (
            <Icon className="size-3 shrink-0" />
          )}
          <span className="truncate">
            {switching ? 'switching…' : status.current === 'e2b' ? 'Cloud Sandbox' : status.current}
          </span>
        </button>
      </PopoverTrigger>
      <PopoverContent className="w-80 p-1.5" align="start">
        <div className="px-2 pb-1.5 pt-1 text-[10.5px] font-semibold uppercase tracking-wider text-muted-foreground">
          Environment
        </div>
        {switching && (
          <div className="px-2.5 pb-2 text-[12px] text-muted-foreground/80 break-words">
            {switching}
          </div>
        )}
        <div className="flex flex-col">
          {environments.map((e) => {
            const current = e.name === status.current;
            const parked = status.parked.includes(e.name);
            const EIcon = envIcon(e.backend);
            return (
              <button
                key={e.name}
                type="button"
                disabled={disabled || !!switching || current}
                onClick={() => {
                  setOpen(false);
                  onSwitch(e.name);
                }}
                className={cn(
                  'flex items-start gap-2 rounded-md px-2.5 py-1.5 text-left text-[13px] transition-colors',
                  current
                    ? 'text-foreground'
                    : 'text-muted-foreground hover:bg-accent/40 hover:text-foreground',
                  (disabled || switching) && !current && 'opacity-50',
                )}
              >
                <EIcon className="size-3.5 shrink-0 mt-0.5" />
                <span className="min-w-0 flex-1">
                  <span className="flex items-center gap-1.5">
                    <span className="truncate">{e.name === 'e2b' ? 'Cloud Sandbox' : e.name}</span>
                    {e.backend !== 'local' && e.backend !== e.name && (
                      <span className="rounded-sm bg-secondary px-1 text-[9.5px] uppercase tracking-wider text-muted-foreground">
                        {e.backend === 'e2b' ? 'Cloud Sandbox' : e.backend}
                      </span>
                    )}
                    {parked && (
                      <span
                        className="rounded-sm bg-mira-blue/15 px-1 text-[9.5px] uppercase tracking-wider text-mira-blue"
                        title="paused — resumes quickly"
                      >
                        paused
                      </span>
                    )}
                  </span>
                  {e.description && (
                    <span className="block truncate text-[11px] text-muted-foreground/70">
                      {e.description.replace(/\bE2B\b/g, 'Cloud Sandbox')}
                    </span>
                  )}
                </span>
                {current && <span className="text-mira-blue text-xs">✓</span>}
              </button>
            );
          })}
        </div>
        <div className="px-2.5 pt-1.5 pb-1 text-[11px] text-muted-foreground/70 border-t border-border/50 mt-1">
          Remote runs use a copy of this worktree; switching back merges the changes in.
          {disabled && !switching && ' Wait for the current turn to finish to switch.'}
        </div>
      </PopoverContent>
    </Popover>
  );
}

/* ---------- worktree chip (branch + dirty + worktree switcher) ---------- */

export function WorktreeChip({
  cwd,
  onCwdSwitched,
}: {
  cwd: string;
  onCwdSwitched?: (path: string, sessionId?: string) => void;
}) {
  const [open, setOpen] = useState(false);
  const [status, setStatus] = useState<GitStatusView | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [newBranch, setNewBranch] = useState('');

  async function refresh() {
    try {
      const s = await getGitStatus();
      setStatus(s);
      setLoadError(null);
    } catch (e) {
      setLoadError(String((e as Error).message));
    }
  }

  useEffect(() => {
    if (!cwd) {
      setStatus(null);
      return;
    }
    refresh();
  }, [cwd]);

  async function switchTo(path: string) {
    setBusy(true);
    try {
      const { session_id } = await putCwd(path);
      // Hand the new slot id up to the parent so it can attach its WS.
      // Without this, the socket keeps forwarding the old slot's frames
      // and the transcript silently stays on the previous folder.
      onCwdSwitched?.(path, session_id);
      // Also refresh git state so the chip label reflects the new
      // worktree before the WS attach lands.
      await refresh();
      setOpen(false);
    } catch (e) {
      setLoadError(String((e as Error).message));
    } finally {
      setBusy(false);
    }
  }

  const setupSequence = useRef(0);
  async function prepareWorkspace(branch: string, existingPath?: string) {
    if (!branch.trim() || busy) return;
    const sequence = ++setupSequence.current;
    const setup: WorkspaceSetup = {
      id: `workspace-${Date.now()}-${sequence}`,
      branch,
      phase: 'creating',
      startedAt: Date.now(),
    };
    let cancelled = false;
    let createdPath = existingPath;
    let stage: 'creating' | 'opening' = existingPath ? 'opening' : 'creating';
    const report = (phase: WorkspaceSetup['phase'], error?: string) =>
      reportWorkspaceSetup({
        ...setup,
        phase,
        error,
        failedStage: phase === 'failed' ? stage : undefined,
        cancel:
          phase === 'creating'
            ? () => {
                cancelled = true;
                report('cancelled');
              }
            : undefined,
        workLocally:
          phase === 'creating'
            ? () => {
                cancelled = true;
                report('cancelled');
              }
            : phase === 'failed'
              ? () => {
                  void switchTo(cwd);
                  report('cancelled');
                }
              : undefined,
        retry: phase === 'failed' ? () => void prepareWorkspace(branch, createdPath) : undefined,
      });
    setBusy(true);
    setLoadError(null);
    setOpen(false);
    report(stage);
    try {
      createdPath ??= (await createWorktree(branch)).path;
      if (cancelled || sequence !== setupSequence.current) return;
      stage = 'opening';
      report('opening');
      const { session_id } = await putCwd(createdPath);
      onCwdSwitched?.(createdPath, session_id);
      setNewBranch('');
      report('done');
      await refresh();
    } catch (error) {
      if (!cancelled && sequence === setupSequence.current) {
        const message = error instanceof Error ? error.message : String(error);
        setLoadError(message);
        report('failed', message);
      }
    } finally {
      if (sequence === setupSequence.current) setBusy(false);
    }
  }
  async function createAndSwitch() {
    await prepareWorkspace(newBranch.trim());
  }
  async function createAndSwitchTo(branch: string) {
    await prepareWorkspace(branch);
  }

  const label = status?.in_repo ? (status.branch ?? 'detached') : 'no git';
  const primary = status?.worktrees.find((w) => !isLinkedWorktree(w.path, status.worktrees));

  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>
        <button
          type="button"
          className="inline-flex items-center gap-1.5 rounded-full px-2.5 py-1.5 text-[12.5px] text-muted-foreground hover:bg-mira-elev2 hover:text-foreground transition-colors max-w-[10rem] min-w-0"
          title={
            status?.in_repo ? `branch: ${label}${status.dirty ? ' (dirty)' : ''}` : 'not a git repo'
          }
        >
          <GitBranch className="size-3 shrink-0" />
          <span className="truncate">{label}</span>
          {status?.dirty && (
            <span
              className="size-1.5 rounded-full bg-amber-500 shrink-0"
              title="uncommitted changes"
            />
          )}
          {status?.is_worktree && (
            <span className="rounded-sm bg-mira-blue/15 px-1 text-[9.5px] uppercase tracking-wider text-mira-blue">
              wt
            </span>
          )}
        </button>
      </PopoverTrigger>
      <PopoverContent className="w-72 p-1.5" align="start">
        <div className="px-2 pb-1.5 pt-1 text-[10.5px] font-semibold uppercase tracking-wider text-muted-foreground">
          Working tree
        </div>
        {!status?.in_repo && (
          <div className="px-2.5 py-2 text-[12px] text-muted-foreground/70">Not a git repo.</div>
        )}
        {status?.in_repo && (
          <>
            <div className="flex flex-col">
              {status.worktrees.map((w) => (
                <button
                  key={w.path}
                  type="button"
                  disabled={busy || w.is_current}
                  onClick={() => switchTo(w.path)}
                  className={cn(
                    'flex items-center gap-2 rounded-md px-2.5 py-1.5 text-left text-[13px] transition-colors',
                    w.is_current
                      ? 'text-foreground'
                      : 'text-muted-foreground hover:bg-accent/40 hover:text-foreground',
                    busy && !w.is_current && 'opacity-50',
                  )}
                >
                  <GitBranch className="size-3.5 shrink-0" />
                  <span className="min-w-0 flex-1 truncate">{w.branch ?? 'detached'}</span>
                  {w.is_current && <span className="text-mira-blue text-xs">✓</span>}
                  {primary && w.path === primary.path && (
                    <span className="rounded-sm bg-secondary px-1 text-[9.5px] uppercase tracking-wider text-muted-foreground">
                      main
                    </span>
                  )}
                </button>
              ))}
            </div>
            {(() => {
              // Existing branches not already checked out in a worktree.
              // Clicking creates `.mira/worktrees/<branch>` (or a local
              // tracking branch on top of the remote) and switches to it
              // via the same `createWorktree` → `switchTo` path the
              // "New worktree" input takes.
              const available = (status.branches ?? []).filter((b) => !b.in_worktree);
              if (available.length === 0) return null;
              return (
                <div className="mt-1.5 border-t border-border/60 pt-1.5">
                  <div className="px-2.5 pb-1 text-[10.5px] font-semibold uppercase tracking-wider text-muted-foreground">
                    Switch to branch
                  </div>
                  <div className="max-h-56 overflow-y-auto flex flex-col">
                    {available.map((b) => (
                      <button
                        key={`${b.is_remote ? 'r' : 'l'}:${b.name}`}
                        type="button"
                        disabled={busy}
                        onClick={() => createAndSwitchTo(b.name)}
                        className={cn(
                          'flex items-center gap-2 rounded-md px-2.5 py-1.5 text-left text-[13px] transition-colors',
                          'text-muted-foreground hover:bg-accent/40 hover:text-foreground',
                          busy && 'opacity-50',
                        )}
                        title={b.upstream ? `tracks ${b.upstream}` : b.name}
                      >
                        <GitBranch className="size-3.5 shrink-0 opacity-60" />
                        <span className="min-w-0 flex-1 truncate">{b.name}</span>
                        {b.is_remote && (
                          <span className="rounded-sm bg-secondary/60 px-1 text-[9.5px] uppercase tracking-wider text-muted-foreground">
                            remote
                          </span>
                        )}
                      </button>
                    ))}
                  </div>
                </div>
              );
            })()}
            <div className="mt-1.5 border-t border-border/60 pt-1.5">
              <div className="px-2.5 pb-1 text-[10.5px] font-semibold uppercase tracking-wider text-muted-foreground">
                New worktree
              </div>
              <div className="flex items-center gap-1.5 px-1.5">
                <input
                  value={newBranch}
                  onChange={(e) => setNewBranch(e.target.value)}
                  onKeyDown={(e) => {
                    if (e.key === 'Enter') {
                      e.preventDefault();
                      createAndSwitch();
                    }
                  }}
                  placeholder="branch name"
                  disabled={busy}
                  className="flex-1 min-w-0 rounded-md border border-border bg-background/60 px-2 py-1 text-[12.5px] outline-none focus:border-mira-blue/60"
                />
                <button
                  type="button"
                  onClick={createAndSwitch}
                  disabled={busy || !newBranch.trim()}
                  className="rounded-md bg-foreground px-2.5 py-1 text-[12px] text-background transition-opacity hover:opacity-90 disabled:opacity-35"
                >
                  Add
                </button>
              </div>
            </div>
          </>
        )}
        {loadError && (
          <div className="mx-1.5 mt-1.5 rounded-md border border-destructive/40 bg-destructive/10 px-2 py-1 text-[11px] text-destructive">
            {loadError}
          </div>
        )}
      </PopoverContent>
    </Popover>
  );
}

/** A worktree is "linked" (not the primary) when it lives under any other
 *  worktree's `.git/worktrees/<name>`. We approximate cheaply: if the path
 *  is *inside* one of the other entries' paths, treat it as linked. Good
 *  enough for display sorting; the backend already tags `is_current`. */
export function isLinkedWorktree(path: string, all: { path: string }[]): boolean {
  return all.some((o) => o.path !== path && path.startsWith(o.path + '/'));
}
