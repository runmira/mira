import { AnimatePresence, m } from 'framer-motion';
import {
  Check,
  GitPullRequest,
  PanelRightClose,
  PanelRightOpen,
  SquareTerminal,
} from 'lucide-react';
import { useEffect, useMemo, useState } from 'react';
import type { BranchPrView, GitStatusView, SessionDiffView } from '../api';
import type { BackgroundProcess } from '../lib/backgroundProcesses';
import type { Entry, SubagentStreamState } from '../transcript/entries';
import type { TaskItem } from '../types';
import {
  BackgroundProcessesSection,
  chatSubagents,
  fmtNum,
  liveTasks,
  ProgressSection,
  SubagentsSection,
} from './context/ProgressSection';
import { extractSources, SourcesSection } from './context/SourcesSection';
import { prStateColor, workspaceHasContent, WorkspaceSection } from './context/WorkspaceSection';
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
  processes?: BackgroundProcess[];
  tasks: TaskItem[];
  gitStatus: GitStatusView | null;
  sessionDiff: SessionDiffView;
  branchPr: BranchPrView | null;
  sessionCommitted: boolean;
  subagentState: Map<string, SubagentStreamState>;
  entries: Entry[];
}): boolean {
  return (
    (p.processes?.some((process) => process.running) ?? false) ||
    liveTasks(p.tasks).length > 0 ||
    workspaceHasContent(p.gitStatus, p.sessionDiff, p.branchPr, p.sessionCommitted) ||
    chatSubagents(p.subagentState, p.entries).length > 0 ||
    extractSources(p.entries).length > 0
  );
}

export type ContextPanelProps = {
  processes?: BackgroundProcess[];
  stoppingProcesses?: ReadonlySet<number>;
  onOpenProcess?: (id: number) => void;
  onStopProcess?: (id: number) => Promise<void>;
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
  const runningProcesses = (props.processes ?? []).filter((process) => process.running);
  const fits = useContextPanelFits();
  // Derived once per render and shared by the pill and every section, so
  // they can't disagree about what there is to show.
  const sources = useMemo(() => extractSources(entries), [entries]);
  const subagents = useMemo(() => chatSubagents(subagentState, entries), [subagentState, entries]);
  const live = liveTasks(tasks);
  const showWorkspace = workspaceHasContent(gitStatus, sessionDiff, branchPr, sessionCommitted);

  if (
    !fits ||
    (live.length === 0 &&
      !showWorkspace &&
      subagents.length === 0 &&
      sources.length === 0 &&
      runningProcesses.length === 0)
  ) {
    return null;
  }

  const done = live.filter((t) => t.status === 'completed').length;
  const pending = sessionDiff.files.length;
  const agentsRunning = subagents.some((s) => !s.state.done);

  return (
    <AnimatePresence mode="wait" initial={false}>
      {!open ? (
        /* Collapsed: a small pill that summarizes, out of the way. */
        <m.button
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
          {runningProcesses.length > 0 && (
            <span
              className="inline-flex shrink-0 items-center gap-1"
              title="Background processes running"
            >
              <SquareTerminal aria-hidden="true" className="size-3" />
              {runningProcesses.length}
            </span>
          )}
          {agentsRunning && (
            <span
              className="size-1.5 shrink-0 rounded-full bg-mira-purple animate-pulse"
              title="subagents running"
            />
          )}
        </m.button>
      ) : (
        <m.aside
          key="card"
          initial={{ opacity: 0, scale: 0.97, y: -6 }}
          animate={{ opacity: 1, scale: 1, y: 0, transition: { duration: 0.18, ease: 'easeOut' } }}
          exit={{ opacity: 0, scale: 0.97, y: -4, transition: { duration: 0.12 } }}
          className="absolute top-3 right-3 z-20 flex flex-col overflow-hidden rounded-2xl border border-fg/[0.08]"
          style={{
            ...cardStyle,
            width: PANEL_WIDTH,
            maxHeight: 'calc(100% - 24px)',
            transformOrigin: 'top right',
          }}
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
            {runningProcesses.length > 0 && props.onOpenProcess && props.onStopProcess && (
              <div className="py-2">
                <BackgroundProcessesSection
                  processes={runningProcesses}
                  onOpen={props.onOpenProcess}
                  onStop={props.onStopProcess}
                  stopping={props.stoppingProcesses ?? new Set()}
                />
              </div>
            )}
            {sources.length > 0 && (
              <div className="py-2">
                <SourcesSection sources={sources} />
              </div>
            )}
          </div>
        </m.aside>
      )}
    </AnimatePresence>
  );
}
