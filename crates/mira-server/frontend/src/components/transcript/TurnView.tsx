/**
 * One user turn in the transcript: its entries plus the "worked for"
 * summary chip. Memoized so streaming into the last turn doesn't re-render
 * earlier ones. Split out of App.tsx.
 */
import { UsageRecoveryCard, type RecoveryAction } from '../UsageRecoveryCard';
import { Collapse } from '../ui/Collapse';
import { turnActivity, type Turn, type GroupItem } from '../../lib/turnActivity';
import { TurnChanges } from '../TurnChanges';
import { memo, useContext, useEffect, useMemo, useRef, useState } from 'react';
import {
  ChevronDown,
} from 'lucide-react';
import { cn } from '../../lib/utils';
import { getBoolPref, PREF_KEYS } from '../../lib/prefs';
import { type SkillView } from '../../api';
import { type AskUserDecision } from '../AskUserCard';
import { AgentGroup } from '../AgentCard';
import { countsByCategory, countsPhrase, ToolGroup } from '../ToolGroup';
import type {
  ApprovalScope,
  DiffPreview,
  Mode,
  PlanStep,
  UsageTotals,
} from '../../types';
import { sameTurn, type TurnTiming, type TurnStatsEntry, type Entry } from '../../transcript/entries';
import { MessageActionsContext, EntryView, TurnStatsContext, formatDuration } from './EntryView';

/** Re-render a turn only when something it shows changed. Handlers are
 *  stable (see `useStableCallback`), so plain identity works for them. */
export const TurnView = memo(TurnViewImpl, (prev, next) => {
  for (const k of Object.keys(next) as (keyof TurnViewProps)[]) {
    if (k === 'turn') {
      if (!sameTurn(prev.turn, next.turn)) return false;
    } else if (prev[k] !== next[k]) {
      return false;
    }
  }
  return true;
});

export type TurnViewProps = Parameters<typeof TurnViewImpl>[0];

export function TurnViewImpl({
  turn, diffSummary, recovery, recoveryDisabled, onRecoveryAction, timing: timingProp, usage: usageProp, model: modelProp, index, expanded, isActive, onToggle, onDecide, onPlanReply, onAskUserReply, onOpenAgent, onOpenFile, skills, mode, onSetMode, minimapId, approvalViaDialog, offscreenOk = false,
}: {
  turn: Turn;
  recovery?: import('../../types').QueuedInput;
  recoveryDisabled?: boolean;
  onRecoveryAction?: (id: string, action: RecoveryAction) => Promise<void>;
  diffSummary?: import('../../types').TurnDiffSummary;
  /** Position in the transcript; what `onToggle` is called with. */
  index: number;
  /** Not one of the latest turns: the browser may skip laying it out while
   *  it's off screen. */
  offscreenOk?: boolean;
  timing: TurnTiming | null;
  /** Tokens this turn used (live turns only; not persisted). */
  usage: UsageTotals | null;
  model: string;
  expanded: boolean;
  isActive: boolean;
  onToggle: (index: number) => void;
  onDecide: (callId: string, allow: boolean, scope?: ApprovalScope) => void;
  onPlanReply: (callId: string, approved: boolean, steps?: PlanStep[], note?: string) => void;
  /** Answer callback for the `ask_user` clarification tool. */
  onAskUserReply: (callId: string, decision: AskUserDecision) => void;
  /** Opens (or focuses) the right-side SubagentPanel tab for the given
   *  agent call. Wired from App.tsx via `openAgentTab`. */
  onOpenAgent: (callId: string) => void;
  /** Opens (or focuses) a file viewer tab in the right-side panel. */
  onOpenFile: (path: string, diff: DiffPreview | null) => void;
  /** Loaded skill roster — passed through so the user bubble can render
   *  `@skill:<name>` mentions as pretty chips (icon + display label +
   *  hash-derived color) rather than raw tokens. */
  skills: SkillView[];
  /** Session mode + setter — plumbed to pending tool-approval cards so
   *  the "Always allow" button can bump the mode to `edit` (auto
   *  everything unless a rule blocks) for the current session. */
  mode: Mode;
  onSetMode: (m: Mode) => void;
  /** Stable scroll-target id for the timeline minimap (`turn-${index}`). */
  minimapId?: string | null;
  /** Forwarded to entry tool cards: when an agent drives, decisions live in
   *  the unified dialog, not on the cards. */
  approvalViaDialog?: boolean;
}) {
  const messageActions = useContext(MessageActionsContext);
  // An agent turn carries its own stats; they win over the per-index
  // maps, which only line up with Mira's own turns.
  const own = turn.body.find((e): e is TurnStatsEntry => e.kind === 'turn_stats');
  const timing: TurnTiming | null =
    own?.startedAt != null ? { startedAt: own.startedAt, endedAt: own.endedAt } : timingProp;
  const usage = own?.usage ?? usageProp;
  const model = own?.model ?? modelProp;

  // Split the body into "intermediate work" and the final assistant text.
  // Rule: the LAST assistant text message with non-empty content is the
  // final answer; everything before it is intermediate. Tool cards + earlier
  // assistant text hide behind the "Worked for" chip when collapsed.
  // The stats entry is data, not content: it mustn't count as work done.
  const body = useMemo(() => turn.body.filter((e) => e.kind !== 'turn_stats'), [turn.body]);
  const nativeTurn = own != null || body.some((e) => (e.kind === 'msg' && e.nativeMessageId != null) || (e.kind === 'tool' && e.agentCall != null) || (e.kind === 'thought' && e.native));
  const { intermediateRaw, intermediate, finalEntry, trailing } = useMemo(
    () => turnActivity(body, nativeTurn && !isActive), [body, nativeTurn, isActive],
  );

  // While a turn is in flight, force the intermediate section open so
  // in-progress tool calls (esp. pending approval bubbles) stay visible.
  // The user can collapse it after `done` fires. A pending approval
  // requires a click to keep the model moving; hiding it would deadlock.
  const hasPendingApproval = intermediateRaw.some(
    (e) => e.kind === 'tool' && e.status === 'pending',
  );
  const hasPendingAskUser = intermediateRaw.some(
    (e) => e.kind === 'tool' && e.askUser != null && e.askUser.decision === null,
  );
  const hasPendingPlan = intermediateRaw.some(
    (e) => e.kind === 'tool' && e.plan != null && e.plan.decision === null,
  );
  // `waitingForUser` covers every state where mira has handed the turn
  // back to the human: approval prompts, plan review, ask_user cards.
  // The "Working…" timer pauses while this is true so the elapsed
  // display reflects work-done-by-mira, not wall-clock-minus-thinking.
  const waitingForUser = hasPendingApproval || hasPendingAskUser || hasPendingPlan;

  // Freeze the timer during wait periods. `waitStartedRef` marks the
  // wall-clock instant the current wait began; `waitAccumRef` keeps the
  // running total of prior wait segments in this same turn (so a
  // turn with multiple approval rounds still reads correctly). Both are
  // per-turn state — the component instance is stable across renders. */
  const waitStartedRef = useRef<number | null>(null);
  const waitAccumRef = useRef<number>(0);
  useEffect(() => {
    const now = Date.now();
    if (waitingForUser && waitStartedRef.current === null) {
      waitStartedRef.current = now;
    } else if (!waitingForUser && waitStartedRef.current !== null) {
      waitAccumRef.current += now - waitStartedRef.current;
      waitStartedRef.current = null;
    }
  }, [waitingForUser]);

  const activeWaitMs =
    waitStartedRef.current !== null ? Date.now() - waitStartedRef.current : 0;
  const totalWaitMs = waitAccumRef.current + activeWaitMs;
  const rawDurationMs = timing
    ? (timing.endedAt ?? Date.now()) - timing.startedAt
    : null;
  const durationMs =
    rawDurationMs !== null ? Math.max(0, rawDurationMs - totalWaitMs) : null;
  const showWorkedChip =
    getBoolPref(PREF_KEYS.transcriptTurnStats, true) &&
    (intermediate.length > 0 || isActive) && durationMs != null;

  const forceOpen = isActive || hasPendingApproval || hasPendingAskUser || hasPendingPlan;
  const effectivelyExpanded = expanded || forceOpen;

  // Codex-style categorised activity phrase ("3 reads, 4 searches, 1 write")
  // shown next to the "Worked for" duration so a collapsed turn still tells
  // the reader WHAT mira did, not just for how long. Counts every tool
  // entry across the whole turn body — some flows put the assistant text
  // BEFORE the tool run (e.g. "huh?" turns, or turns interrupted after the
  // model started answering), which would land tools in `trailing` rather
  // than `intermediateRaw` and drop them from the count.
  const activitySummary = useMemo(() => {
    const calls = turn.body
      .filter((e): e is Extract<Entry, { kind: 'tool' }> => e.kind === 'tool')
      .map((e) => e.call);
    if (calls.length === 0) return '';
    return countsPhrase(countsByCategory(calls));
  }, [turn.body]);

  const renderActivity = (item: GroupItem, i: number, section: string) => {
    if (item.kind === 'agent-group') {
      return <div key={`${section}-a-${item.entries[0].call.id}`} className="flex justify-start"><AgentGroup entries={item.entries} onOpen={onOpenAgent} /></div>;
    }
    if (item.kind === 'tool-group') {
      return <div key={`${section}-g-${item.entries[0].call.id}`} className="flex justify-start"><ToolGroup entries={item.entries} onOpenFile={onOpenFile} /></div>;
    }
    return <EntryView key={`${section}-i-${i}`} showAssistantActions={!nativeTurn} streaming={isActive} entry={item.entry} onDecide={onDecide} onPlanReply={onPlanReply} onAskUserReply={onAskUserReply} onOpenAgent={onOpenAgent} approvalViaDialog={approvalViaDialog} onOpenFile={onOpenFile} skills={skills} mode={mode} onSetMode={onSetMode} />;
  };

  return (
    <div
      data-minimap-id={minimapId ?? undefined}
      className="flex min-w-0 flex-col gap-2"
      // Long chats: skip layout and paint for older turns while they're off
      // screen. The browser remembers each one's last size, so scrolling and
      // the minimap's jumps stay accurate.
      style={offscreenOk ? { contentVisibility: 'auto', containIntrinsicSize: 'auto 480px' } : undefined}
    >
      {turn.user && <EntryView entry={turn.user} onDecide={onDecide} onPlanReply={onPlanReply} onAskUserReply={onAskUserReply} onOpenAgent={onOpenAgent} approvalViaDialog={approvalViaDialog} onOpenFile={onOpenFile} skills={skills} mode={mode} onSetMode={onSetMode} />}

      {showWorkedChip && (
        <div className="w-full border-b border-border/70 pb-2">
        <WorkedForChip
          durationMs={durationMs!}
          active={isActive && timing?.endedAt == null}
          waitingForUser={waitingForUser}
          expanded={effectivelyExpanded}
          locked={forceOpen}
          onToggle={() => onToggle(index)}
          activity={activitySummary}
        />
        </div>
      )}

      {intermediate.length > 0 && <Collapse open={effectivelyExpanded || !showWorkedChip}><div className="flex min-w-0 flex-col gap-2">{intermediate.map((item, i) => renderActivity(item, i, 'work'))}</div></Collapse>}

      {finalEntry && (
        <TurnStatsContext.Provider
          value={
            timing?.endedAt != null
              ? { durationMs: durationMs ?? timing.endedAt - timing.startedAt, usage, model, costUsd: own?.costUsd ?? null }
              : null
          }
        >
          <EntryView showAssistantActions={!nativeTurn || !isActive} streaming={isActive} entry={finalEntry} onDecide={onDecide} onPlanReply={onPlanReply} onAskUserReply={onAskUserReply} onOpenAgent={onOpenAgent} approvalViaDialog={approvalViaDialog} onOpenFile={onOpenFile} skills={skills} mode={mode} onSetMode={onSetMode} />
        </TurnStatsContext.Provider>
      )}
      {trailing.map((item, i) => renderActivity(item, i, 'trailing'))}
      {recovery && onRecoveryAction && <UsageRecoveryCard key={recovery.id} item={recovery} disabled={!!recoveryDisabled} onAction={onRecoveryAction} />}
      {diffSummary && <TurnChanges summary={diffSummary} onOpenFile={path => onOpenFile(path, null)} busy={messageActions?.busy} onUndo={turn.user && messageActions ? () => messageActions.restore(turn.user!) : undefined} />}
    </div>
  );
}

export function WorkedForChip({
  durationMs, active, waitingForUser, expanded, locked, onToggle, activity,
}: {
  durationMs: number;
  active: boolean;
  /** True while mira has handed the turn back to the user (approval,
   *  plan review, ask_user card). The chip flips to a "Waiting for you"
   *  label and drops the duration — the timer visibly pauses. */
  waitingForUser: boolean;
  expanded: boolean;
  /** Force-open due to in-flight work or a pending approval — the chip
   *  goes non-interactive so the user can't collapse away important state. */
  locked: boolean;
  onToggle: () => void;
  /** Categorised activity phrase ("3 reads, 4 searches, 1 write") for the
   *  turn's intermediate work — appended after the duration so a collapsed
   *  turn still surfaces WHAT mira did. Empty when nothing ran (e.g. a
   *  no-tool answer). */
  activity: string;
}) {
  // The turn around this chip is memoized and only re-renders when its
  // entries change, so a live count can't ride the parent's tick (it sat
  // at "0s" while the thinking line below counted up). Tick here instead,
  // from the moment the given duration was measured.
  const live = active && !waitingForUser;
  const measuredAt = useMemo(() => Date.now(), [durationMs]);
  const [, setTick] = useState(0);
  useEffect(() => {
    if (!live) return;
    const t = setInterval(() => setTick((n) => n + 1), 1000);
    return () => clearInterval(t);
  }, [live]);
  const shownMs = live ? durationMs + (Date.now() - measuredAt) : durationMs;
  const durationLabel = waitingForUser
    ? 'Waiting for you'
    : active
      ? `Working… ${formatDuration(shownMs)}`
      : `Worked for ${formatDuration(durationMs)}`;
  const dot = waitingForUser
    ? 'bg-amber-400'
    : active
      ? 'bg-mira-blue'
      : null;
  // "Active" here = the turn is still running (blue dot) or waiting
  // for the user (amber dot). In either case the label needs full
  // contrast — it's telling the user *something is happening*.
  // Idle "Worked for" recedes into muted grey so scrolling past
  // finished turns doesn't visually shout; hover brings it back.
  const isActive = active || waitingForUser;
  return (
    <button
      type="button"
      onClick={locked ? undefined : onToggle}
      disabled={locked}
      aria-expanded={expanded}
      title={locked ? 'Auto-expanded while in progress' : undefined}
      // `group` so the trailing caret can key off hover state via
      // `group-hover:*` — hidden until the row is hovered or already
      // expanded, so a collapsed transcript stays quiet.
      className={cn(
        'group flex w-fit items-center gap-1.5 rounded-md py-1 text-[14px] font-semibold leading-5 transition-colors',
        // Idle: muted grey. Hover/active/expanded: full contrast.
        isActive || expanded ? 'text-foreground/90' : 'text-muted-foreground/70',
        locked ? 'cursor-default opacity-80' : 'hover:bg-accent/40 hover:text-foreground',
      )}
    >
      <span>{durationLabel}</span>
      {activity && (
        // Middle-dot separator + un-bolded activity phrase so the
        // duration stays the primary read and the counts trail as a
        // subtitle. Hidden while waiting on the user — the amber
        // "Waiting for you" label is already carrying the message.
        !waitingForUser && (
          <>
            <span
              aria-hidden
              className={cn(
                'font-normal opacity-70',
                isActive || expanded ? 'text-foreground/60' : 'text-muted-foreground/50',
              )}
            >
              ·
            </span>
            <span
              className={cn(
                'font-normal',
                isActive || expanded ? 'text-foreground/70' : 'text-muted-foreground/70',
              )}
            >
              {activity}
            </span>
          </>
        )
      )}
      {dot && <span className={cn('size-1.5 animate-pulse rounded-full', dot)} />}
      <ChevronDown
        strokeWidth={2.5}
        className={cn(
          'size-3.5 transition-all',
          // Caret adopts the row's text colour so it fades with the
          // label instead of standing out against the muted grey.
          isActive || expanded ? 'text-foreground/70' : 'text-muted-foreground/70',
          !expanded && '-rotate-90',
          // Hide when collapsed AND not hovered; always show when
          // expanded (or hovered) so state is legible without
          // needing a second visual language for open/closed.
          !expanded && 'opacity-0 group-hover:opacity-100',
        )}
      />
    </button>
  );
}

