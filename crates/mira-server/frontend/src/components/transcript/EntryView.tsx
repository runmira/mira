/**
 * Rendering of a single transcript entry (messages, tools, goals, engine
 * switches, status lines) and the per-message action buttons. Split out of
 * App.tsx.
 */
import { ImageAttachmentDetails } from '../ImageAttachmentDetails';
import { createContext, useContext, useState } from 'react';
import {
  RotateCw,
  Redo2,
  Check,
  LoaderCircle,
  Copy,
  Info,
  Lightbulb,
  Pencil,
  ShieldAlert,
  Sparkle,
  Target,
  History,
} from 'lucide-react';
import { cn } from '../../lib/utils';
import { imageSrc } from '../../lib/images';
import { EngineMark } from '../EnginePicker';
import { AgentIcon, ProviderIcon } from '../AgentIcon';
import { HtmlRenderCard } from '../HtmlRenderCard';
import { CompactionCard } from '../CompactionCard';
import { prettyModel } from '../../lib/models';
import { isAgentRequest } from '../../lib/agentRequest';
import { getBoolPref, PREF_KEYS } from '../../lib/prefs';
import { costUsd, formatDollars, shortNum } from '../../lib/usage';
import { type SkillView } from '../../api';
import { Tip } from '../ui/Tip';
import {
  parseSentAttachments,
  SentAttachmentChip,
} from '../Composer';
import { UserRichText } from '../UserRichText';
import { AssistantContent } from '../AssistantContent';
import { ThoughtBlock } from '../ThoughtBlock';
import {
  GitFork,
} from 'lucide-react';
import { ToolCard } from '../ToolCard';
import { PlanCard } from '../PlanCard';
import { AskUserCard, type AskUserDecision } from '../AskUserCard';
import { AgentCard } from '../AgentCard';
import { DelegateCard, isDelegateTaskName } from '../DelegateCard';
import { ToolGroup } from '../ToolGroup';
import type {
  ApprovalScope,
  DiffPreview,
  Mode,
  PlanStep,
  UsageTotals,
  SessionEngine,
} from '../../types';
import { engineLabel, agentDisplayName, type Entry, type GoalEntry } from '../../transcript/entries';

/** An engine's favicon: the agent's mark, or the provider's. */
export function EngineEndMark({ engine }: { engine: SessionEngine }) {
  if (engine.kind === 'agent' && engine.driver) {
    return <AgentIcon kind={engine.driver} name={engine.display_name} size="xs" tile={false} />;
  }
  return engine.instance
    ? <ProviderIcon instance={engine.instance} name={engine.display_name} model={engine.model} size="xs" />
    : <EngineMark engine={engine} model={engine.model} />;
}

/** The divider an engine switch leaves in the transcript: a handoff from
 *  one engine to the other, with what carried over. */
export function EngineSwitchDivider({ engine, from }: { engine: SessionEngine; from?: SessionEngine | null }) {
  if (from) {
    return (
      <div className="my-1 flex items-center gap-3 text-[11.5px] text-muted-foreground/60" role="separator">
        <span className="h-px flex-1 bg-border/50" />
        <span className="inline-flex min-w-0 items-center gap-1.5" title="The conversation so far was handed to the new engine">
          <span className="text-muted-foreground/50">Context handoff</span>
          <EngineEndMark engine={from} />
          <span className="truncate">{engineLabel(from)}</span>
          <span aria-hidden>→</span>
          <EngineEndMark engine={engine} />
          <span className="truncate">{engineLabel(engine)}</span>
        </span>
        <span className="h-px flex-1 bg-border/50" />
      </div>
    );
  }
  const provider = engine.display_name && !['Mira', 'provider'].includes(engine.display_name)
    ? engine.display_name
    : null;
  const name =
    engine.kind === 'agent'
      ? agentDisplayName(engine.driver ?? '', engine.display_name)
      : [provider, engine.model ? prettyModel(engine.model) : null].filter(Boolean).join(' · ') ||
        'your provider';
  return (
    <div className="my-1 flex items-center gap-3 text-[11.5px] text-muted-foreground/60" role="separator">
      <span className="h-px flex-1 bg-border/50" />
      <span className="inline-flex items-center gap-1.5">
        <EngineMark engine={engine} model={engine.model} />
        {engine.kind === 'agent' ? `Switched to ${name}` : `Back on ${name}`}
      </span>
      <span className="h-px flex-1 bg-border/50" />
    </div>
  );
}

export function EntryView({
  entry,
  onDecide,
  onPlanReply,
  onOpenAgent,
  onOpenFile,
  skills,
  mode,
  onSetMode,
  onAskUserReply,
  approvalViaDialog,
  showAssistantActions = true, streaming = false,
}: {
  entry: Entry;
  showAssistantActions?: boolean;
  streaming?: boolean;
  onDecide: (callId: string, allow: boolean, scope?: ApprovalScope) => void;
  onPlanReply: (callId: string, approved: boolean, steps?: PlanStep[], note?: string) => void;
  onOpenAgent: (callId: string) => void;
  /** Opens a file viewer tab in the right-side panel. */
  onOpenFile?: (path: string, diff: DiffPreview | null) => void;
  /** Answer callback for the `ask_user` tool card. Fires when the user
   *  submits picks (or dismisses); flips the card into its resolved
   *  state locally and posts back to the server. */
  onAskUserReply?: (callId: string, decision: AskUserDecision) => void;
  approvalViaDialog?: boolean;
  /** Roster used by the user bubble to pretty-print `@skill:<name>`
   *  mentions. Defaults to empty when the parent doesn't pass one
   *  (e.g. tool/plan/agent entries never touch it). */
  skills?: SkillView[];
  /** Session mode + setter — forwarded to the pending tool-approval
   *  card so its "Always allow" button can bump the session out of a
   *  gating mode. Optional so tool/plan/agent-only callers don't have
   *  to pass them. */
  mode?: Mode;
  onSetMode?: (m: Mode) => void;
}) {
  const actions = useContext(MessageActionsContext);
  switch (entry.kind) {
    case 'msg': {
      const { role, content } = entry.msg;
      if (role === 'tool') return null;
      const body = (content ?? '').trim();
      if (!body && !entry.msg.images?.length) return null;
      if (role === 'user') {
        // Attachments live above the bubble as chips (Codex-style). The
        // model still sees the fenced content in the body — we just hide
        // that from the reader so the transcript stays scannable.
        const { attachments, text } = parseSentAttachments(content ?? '');
        const images = entry.msg.images ?? [];
        return (
          <div className="flex flex-col items-end gap-1.5">
            {entry.msg.input_intent === 'steer' && <span title="Additional input for the current turn" className="inline-flex items-center gap-1 text-xs text-muted-foreground"><Redo2 size={12} aria-hidden="true" /> Steer</span>}
            {attachments.length > 0 && (
              <div className="flex max-w-[78%] flex-wrap justify-end gap-1.5">
                {attachments.map((a, i) => (
                  <SentAttachmentChip key={`att-${i}-${a.filename}`} filename={a.filename} subtype={a.subtype} />
                ))}
              </div>
            )}
            {images.length > 0 && (
              <div className="flex max-w-[78%] flex-wrap justify-end gap-1.5">
                {images.map((img, i) => (
                  <div key={i} className="flex flex-col gap-1"><img
                    src={imageSrc(img)}
                    loading="lazy"
                    alt="attached image"
                    onClick={() => actions?.openImage(imageSrc(img))}
                    className="max-h-40 max-w-[240px] cursor-zoom-in rounded-xl border border-border object-cover"
                  /><ImageAttachmentDetails image={img} /></div>
                ))}
              </div>
            )}
            {text.trim() && (
              <UserMessage entry={entry} text={text} raw={content ?? ''}>
                <UserRichText text={text} roster={skills ?? []} />
              </UserMessage>
            )}
          </div>
        );
      }
      return (
        <div className="group/msg flex justify-start">
          <div className="max-w-[90%]">
            <div data-assistant-message><AssistantContent streaming={streaming && !entry.nativeCompleted} text={content ?? ''} onOpenFile={onOpenFile} /></div>
            {showAssistantActions ? <AssistantActions entry={entry} text={content ?? ''} /> : <div className="mt-1 flex opacity-0 transition-opacity group-hover/msg:opacity-100 focus-within:opacity-100"><MessageTime entry={entry} /></div>}
          </div>
        </div>
      );
    }
    case 'tool':
      // An agent's permission request is answered in the composer card; in
      // the transcript the agent's own tool card already shows the call.
      if (entry.status === 'pending' && isAgentRequest(entry.call)) return null;
      // The `plan` tool gets a dedicated inline card with an editable step
      // list; the `ask_user` tool gets a multi-choice question card; the
      // `agent` tool gets a compact per-agent card so parallel spawns
      // don't dominate the transcript. Everything else falls through to
      // the generic tool row.
      //
      // While a plan / ask_user is still pending (no decision yet), we show
      // a compact chip in the transcript — the interactive version lives in
      // the Composer so it stays anchored at the bottom even in long chats.
      if (entry.plan) {
        if (!entry.plan.decision) {
          return (
            <div className="flex justify-start">
              <div className="inline-flex items-center gap-2 rounded-xl border border-mira-blue/20 bg-mira-blue/[0.05] px-3 py-1.5 text-[12.5px]">
                <Lightbulb fill="currentColor" className="size-3.5 shrink-0 text-mira-blue/70" />
                <span className="font-medium text-mira-blue/80">Plan</span>
                <span className="text-muted-foreground/40">·</span>
                <span className="text-muted-foreground/80 truncate max-w-[40ch]">{entry.plan.proposal.title}</span>
                <span className="text-muted-foreground/40">·</span>
                <span className="text-[11px] text-muted-foreground/60">review below ↓</span>
              </div>
            </div>
          );
        }
        return (
          <div className="flex justify-start">
            <PlanCard
              proposal={entry.plan.proposal}
              decision={entry.plan.decision}
              onApprove={(steps) => onPlanReply(entry.call.id, true, steps)}
              onCancel={(note) => onPlanReply(entry.call.id, false, undefined, note || undefined)}
            />
          </div>
        );
      }
      if (entry.askUser) {
        if (!entry.askUser.decision) {
          return (
            <div className="flex justify-start">
              <div className="inline-flex items-center gap-2 rounded-xl border border-mira-blue/20 bg-mira-blue/[0.05] px-3 py-1.5 text-[12.5px]">
                <Sparkle fill="currentColor" className="size-3.5 shrink-0 text-mira-blue/70" />
                <span className="font-medium text-mira-blue/80">Question</span>
                <span className="text-muted-foreground/40">·</span>
                <span className="text-[11px] text-muted-foreground/60">answer below ↓</span>
              </div>
            </div>
          );
        }
        return (
          <div className="flex justify-start">
            <div>
            <AskUserCard
              proposal={entry.askUser.proposal}
              decision={entry.askUser.decision}
              onSubmit={(answers) =>
                onAskUserReply?.(entry.call.id, { cancelled: false, answers })
              }
              onCancel={() =>
                onAskUserReply?.(entry.call.id, { cancelled: true })
              }
            />
            {entry.runtimeDelivery && entry.runtimeDelivery !== 'cancelled' && (
              <p className="mt-1 text-xs text-muted-foreground" role="status">
                {entry.runtimeDelivery === 'queued' ? 'Answers saved; waiting to send.' : entry.runtimeDelivery === 'dispatching' ? 'Answers saved; delivery has not been confirmed.' : entry.runtimeDelivery === 'delivered' ? 'Answers sent.' : ''}
              </p>
            )}
            </div>
          </div>
        );
      }
      if (entry.call.function.name === 'agent') {
        return (
          <div className="flex justify-start">
            <AgentCard
              call={entry.call}
              status={entry.status}
              result={entry.result}
              onOpen={onOpenAgent}
            />
          </div>
        );
      }
      // A cross-engine hand-off gets its own card (who took it, what it is
      // doing) instead of a generic tool row. Agents' own `Task`/`Agent`
      // calls map to `delegate` and stay ordinary tool rows — see the card.
      if (isDelegateTaskName(entry.call.function.name)) {
        return (
          <div className="flex justify-start">
            <DelegateCard
              call={entry.call}
              status={entry.status}
              result={entry.result}
              startedAt={entry.startedAt}
              steps={entry.delegateSteps}
            />
          </div>
        );
      }
      // Pending approval: show a compact chip in transcript since the
      // interactive card is now anchored in the Composer.
      if (entry.status === 'pending') {
        return (
          <div className="flex justify-start">
            <div className="inline-flex items-center gap-2 rounded-xl border border-amber-500/20 bg-amber-500/[0.05] px-3 py-1.5 text-[12.5px]">
              <ShieldAlert fill="currentColor" className="size-3.5 shrink-0 text-amber-400/80" />
              <span className="font-medium text-amber-400/80">Approval</span>
              <span className="text-muted-foreground/40">·</span>
              <span className="text-muted-foreground/80 truncate max-w-[40ch] font-mono text-[11.5px]">
                {entry.call.function.name}
              </span>
              <span className="text-muted-foreground/40">·</span>
              <span className="text-[11px] text-muted-foreground/60">review below ↓</span>
            </div>
          </div>
        );
      }
      if (entry.agentCall) {
        return <div className="flex justify-start"><ToolGroup entries={[{
          call: entry.call, preview: entry.preview, status: entry.status,
          result: entry.result, progressLines: entry.progressLines,
        }]} onOpenFile={onOpenFile} /></div>;
      }
      return (
        <div className="flex justify-start">
          <ToolCard
            call={entry.call}
            preview={entry.preview}
            status={entry.status}
            result={entry.result}
            progressLines={entry.progressLines}
            onDecide={(allow, scope) => onDecide(entry.call.id, allow, scope)}
            mode={mode}
            onSetMode={onSetMode}
            onOpenFile={onOpenFile}
            decisionsViaDialog={approvalViaDialog}
          />
        </div>
      );
    case 'acp_plan':
      // The agent's plan, as a quiet checklist — the same visual weight as
      // the tool rows around it, not a separately labelled box.
      return (
        <div className="flex justify-start">
          <div className="w-full max-w-[90%] px-1 py-0.5 text-[13px]">
            <div className="mb-1.5 text-[12px] font-medium text-muted-foreground">Plan</div>
            <ul className="space-y-1">
              {entry.entries.map((item, i) => {
                const done = item.status === 'completed';
                const active = item.status === 'in_progress';
                return (
                  <li key={i} className="flex items-start gap-2">
                    <span
                      className={cn(
                        'mt-[5px] size-2.5 shrink-0 rounded-full border',
                        done
                          ? 'border-emerald-500/70 bg-emerald-500/70'
                          : active
                            ? 'border-mira-blue bg-mira-blue/30'
                            : 'border-muted-foreground/40',
                      )}
                    />
                    <span className={cn(done ? 'text-muted-foreground line-through decoration-muted-foreground/40' : 'text-foreground/90')}>
                      {item.content}
                    </span>
                  </li>
                );
              })}
            </ul>
          </div>
        </div>
      );
    case 'engine_switch':
      return <EngineSwitchDivider engine={entry.engine} from={entry.from} />;
    case 'html_render':
      return <HtmlRenderCard title={entry.title} html={entry.html} />;
    case 'turn_stats':
      return null;
    case 'activity':
      return <details className="rounded-md border border-border/50 px-3 py-2 text-xs text-muted-foreground"><summary>{entry.title}</summary>{entry.detail && <p className="mt-2 whitespace-pre-wrap">{entry.detail}</p>}</details>;
    case 'warning':
      return <StatusLine text={entry.text} />;
    case 'error':
      return <StatusLine text={entry.text} tone="error" />;
    case 'goal':
      return <GoalTranscriptChip entry={entry} />;
    case 'compact':
      return <CompactionCard entry={entry} />;
    case 'thought':
      return (
        <div className="flex justify-start">
          <div className="max-w-[90%]">
            <ThoughtBlock
              content={entry.text}
              live={entry.live}
              startedAt={entry.startedAt}
              endedAt={entry.endedAt}
            />
          </div>
        </div>
      );
    default: {
      // A transcript entry kind this build does not know how to render.
      //
      // This arm exists because the switch is *not* exhaustiveness-checked,
      // and that is exactly the trap: adding an `Entry` variant compiles
      // cleanly, `tsc` passes, and the row silently renders nothing — which
      // in a transcript is indistinguishable from a wedged agent. Rendering a
      // visible marker turns that class of mistake into something obvious.
      // `never` here would make the switch exhaustive and turn a *missing*
      // case into a compile error. We deliberately do not assert that: the
      // point of this arm is to survive new variants gracefully, and a
      // cast keeps the compiler from rejecting the very additions it should
      // tolerate.
      const unknown = entry as { kind: string };
      return (
        <StatusLine
          text={`unrendered transcript entry: ${unknown.kind}`}
          tone="error"
        />
      );
    }
  }
}

/** Goal lifecycle events in the transcript.
 *
 * Design principles:
 *  - No border, no left accent bar, no pill. The event is anchored by
 *    a colored Target icon + a status-tinted headline; that's enough
 *    to distinguish a goal beat from surrounding assistant text
 *    without a second visual layer of chrome.
 *  - The reason (evaluator note or original condition) breathes below
 *    the headline in muted body text, indented to align under the
 *    headline for a clean two-tier read.
 *  - Terminal statuses use short, positive English ("Goal met") not
 *    protocol-speak ("goal_done · met"). */
export function GoalTranscriptChip({ entry }: { entry: GoalEntry }) {
  const tint = goalChipTint(entry.status, entry.variant);
  const headline = goalChipHeadline(entry);
  const body = entry.reason ?? entry.condition ?? null;
  return (
    <div className="flex justify-start">
      <div className="w-full max-w-2xl py-1.5">
        <div className="flex items-center gap-2 leading-5">
          <Target
            fill="currentColor"
            className={cn(
              'size-3.5 shrink-0',
              tint.icon,
            )}
          />
          <span
            className={cn(
              'text-[13px] font-semibold tracking-tight',
              tint.head,
            )}
          >
            {headline}
          </span>
        </div>
        {body && (
          <div className="mt-1 pl-[22px] whitespace-pre-wrap break-words text-[13px] leading-relaxed text-muted-foreground">
            {body}
          </div>
        )}
      </div>
    </div>
  );
}

export function goalChipHeadline(entry: GoalEntry): string {
  const iter = entry.iteration && entry.maxIterations
    ? ` · iteration ${entry.iteration} of ${entry.maxIterations}`
    : '';
  switch (entry.variant) {
    case 'set':
      return 'Goal set';
    case 'cleared':
      return 'Goal cleared';
    case 'progress':
      return `Goal · still working${iter}`;
    case 'done': {
      const status = entry.status;
      if (status === 'met') return 'Goal met';
      if (status === 'impossible') return 'Goal is impossible';
      if (status === 'needs_user') return 'Goal needs your input';
      if (status === 'exhausted') return 'Goal hit its iteration cap';
      if (status === 'cleared') return 'Goal cleared';
      return 'Goal finished';
    }
  }
}

/** Icon + headline color per (variant, status). No bg / bar / border
 *  fields — those live in the render function above and are always
 *  transparent by design. */
export function goalChipTint(
  status: GoalEntry['status'],
  variant: GoalEntry['variant'],
) {
  if (variant === 'set' || variant === 'progress') {
    return { icon: 'text-mira-purple', head: 'text-mira-purple' };
  }
  if (variant === 'cleared' || status === 'cleared') {
    return { icon: 'text-muted-foreground', head: 'text-muted-foreground' };
  }
  switch (status) {
    case 'met':
      return { icon: 'text-emerald-400', head: 'text-emerald-300' };
    case 'impossible':
      return { icon: 'text-red-400', head: 'text-red-300' };
    case 'needs_user':
    case 'exhausted':
      return { icon: 'text-amber-400', head: 'text-amber-300' };
    default:
      return { icon: 'text-mira-purple', head: 'text-mira-purple' };
  }
}

/* ------------------------------------------------------------------ */
/* Message actions: copy, edit & resend, retry                          */
/* ------------------------------------------------------------------ */

export type MessageActions = {
  busy: boolean;
  /** Rewind to this user message and send `text` in its place. */
  edit: (entry: Entry, text: string) => void;
  /** Re-send the user message that led to this reply. */
  retry: (entry: Entry) => void;
  /** Offer to put the files back the way they were before this message. */
  restore: (entry: Entry) => void;
  /** New chat with the history through this message's turn. `null` when
   *  this chat can't be forked (an external agent drives it). */
  fork: ((entry: Entry) => void) | null;
  /** Show an image full-screen. */
  openImage: (src: string) => void;
};

export const MessageActionsContext = createContext<MessageActions | null>(null);

/** Stats for the turn a final reply closes — shown in its hover row. */
export type TurnStats = {
  durationMs: number;
  usage: UsageTotals | null;
  model: string;
  /** The agent's own cost estimate, when it reports one. */
  costUsd?: number | null;
};

export const TurnStatsContext = createContext<TurnStats | null>(null);

export function turnStatsLabel(t: TurnStats): string {
  const parts = [formatDuration(t.durationMs)];
  if (t.usage) {
    parts.push(`${shortNum(t.usage.prompt_tokens)} in · ${shortNum(t.usage.completion_tokens)} out`);
    const cost = t.costUsd ?? costUsd(t.model, t.usage);
    if (cost != null) parts.push(formatDollars(cost));
  }
  return parts.join(' · ');
}

export function ActionButton({
  title,
  hint,
  align = 'center',
  onClick,
  disabled,
  children,
}: {
  title: string;
  /** A second, quieter line under the label. */
  hint?: string;
  /** Where the tip sits against the button: `end` for rows at the right
   *  edge (your messages) so it never runs off the transcript. */
  align?: 'start' | 'center' | 'end';
  onClick: () => void;
  disabled?: boolean;
  children: React.ReactNode;
}) {
  return (
    <Tip label={title} hint={hint} align={align}>
      <button
        type="button"
        aria-label={title}
        onClick={onClick}
        disabled={disabled}
        className="rounded-md p-1 text-muted-foreground/60 transition-colors hover:bg-accent/50 hover:text-foreground disabled:pointer-events-none disabled:opacity-30"
      >
        {children}
      </button>
    </Tip>
  );
}

export function CopyButton({ text, align }: { text: string; align?: 'start' | 'center' | 'end' }) {
  const [copied, setCopied] = useState(false);
  return (
    <ActionButton
      title={copied ? 'Copied' : 'Copy'}
      align={align}
      onClick={() => {
        void navigator.clipboard.writeText(text).then(() => {
          setCopied(true);
          setTimeout(() => setCopied(false), 1200);
        });
      }}
    >
      {copied ? <Check className="size-3.5" /> : <Copy className="size-3.5" />}
    </ActionButton>
  );
}

export function MessageTime({ entry }: { entry: Entry }) {
  const timestamp = entry.kind === 'msg' ? entry.msg.created_at : undefined;
  if (timestamp == null || !Number.isFinite(timestamp)) return null;
  const date = new Date(timestamp);
  if (Number.isNaN(date.getTime())) return null;
  return <time dateTime={date.toISOString()} title={date.toLocaleString()} className="mx-1.5 text-[11px] tabular-nums text-muted-foreground">{date.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })}</time>;
}

export function AssistantActions({ entry, text }: { entry: Entry; text: string }) {
  const actions = useContext(MessageActionsContext);
  const stats = useContext(TurnStatsContext);
  const showStats = getBoolPref(PREF_KEYS.transcriptTurnStats, true);
  if (!text.trim()) return null;
  return (
    <div className="mt-1 flex items-center gap-0.5 opacity-0 transition-opacity group-hover/msg:opacity-100 focus-within:opacity-100">
      <MessageTime entry={entry} />
      <CopyButton text={text} align="start" />
      {actions && (
        <ActionButton
          title="Retry"
          hint="Send the message before this reply again"
          align="start"
          disabled={actions.busy}
          onClick={() => actions.retry(entry)}
        >
          <RotateCw className="size-3.5" />
        </ActionButton>
      )}
      {showStats && stats && (
        <span className="ml-1.5 text-[11.5px] tabular-nums text-muted-foreground/60">{turnStatsLabel(stats)}</span>
      )}
    </div>
  );
}

/** User bubble with hover actions; Edit swaps it for a textarea that
 *  re-sends from this point (Enter to send, Esc to cancel). */
export function UserMessage({
  entry,
  text,
  raw,
  children,
}: {
  entry: Entry;
  /** Display text (attachments stripped) — what Copy copies. */
  text: string;
  /** Exact sent text — what Edit starts from. */
  raw: string;
  children: React.ReactNode;
}) {
  const actions = useContext(MessageActionsContext);
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(raw);
  const submit = () => {
    const t = draft.trim();
    if (!t || !actions) return;
    setEditing(false);
    actions.edit(entry, t);
  };

  if (editing) {
    return (
      <div className="flex w-full max-w-[78%] flex-col gap-2 rounded-2xl border border-border bg-secondary/60 p-2.5">
        <textarea
          autoFocus
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === 'Enter' && !e.shiftKey) {
              e.preventDefault();
              submit();
            } else if (e.key === 'Escape') {
              setEditing(false);
            }
          }}
          rows={Math.min(10, Math.max(2, draft.split('\n').length))}
          className="w-full resize-none bg-transparent px-1.5 text-[14.5px] outline-none"
        />
        <div className="flex items-center justify-end gap-1.5">
          <span className="mr-auto px-1 text-[11px] text-muted-foreground/60">
            Later messages are replaced; file edits stay (restore them with ↺).
          </span>
          <button
            type="button"
            onClick={() => setEditing(false)}
            className="rounded-md px-2.5 py-1 text-[12px] text-muted-foreground hover:text-foreground"
          >
            Cancel
          </button>
          <button
            type="button"
            onClick={submit}
            disabled={!draft.trim() || actions?.busy}
            className="rounded-md bg-foreground px-2.5 py-1 text-[12px] font-medium text-background disabled:opacity-40"
          >
            Send
          </button>
        </div>
      </div>
    );
  }

  return (
    <div className="group/msg flex max-w-[78%] flex-col items-end">
      {/* The sent message gets the same card treatment as the composer:
          a white, softly-shadowed card in light mode (and the filled
          bubble it always was in dark). One card language for "your
          words" wherever they sit in the UI. */}
      <div className="whitespace-pre-wrap break-words rounded-2xl rounded-br-md border border-border/60 elev-card dark:bg-secondary px-4 py-2.5 text-[14.5px]">
        {children}
      </div>
      <div className="mt-1 flex items-center gap-0.5 opacity-0 transition-opacity group-hover/msg:opacity-100 focus-within:opacity-100">
        <MessageTime entry={entry} />
        <CopyButton text={text} align="end" />
        {actions && (
          <ActionButton
            title="Edit"
            hint="Change this message and send it again"
            align="end"
            disabled={actions.busy}
            onClick={() => {
              setDraft(raw);
              setEditing(true);
            }}
          >
            <Pencil className="size-3.5" />
          </ActionButton>
        )}
        {actions && (
          <ActionButton
            title="Restore files"
            hint="Put files back as they were before this message"
            align="end"
            disabled={actions.busy}
            onClick={() => actions.restore(entry)}
          >
            <History className="size-3.5" />
          </ActionButton>
        )}
        {actions?.fork && (
          <ActionButton
            title="Fork from here"
            hint="New chat with everything through this reply"
            align="end"
            onClick={() => actions.fork?.(entry)}
          >
            <GitFork className="size-3.5" />
          </ActionButton>
        )}
      </div>
    </div>
  );
}

/* ------------------------------------------------------------------ */
/* Status lines: harness notes riding the warning stream                */
/* ------------------------------------------------------------------ */

export type StatusTone = 'ok' | 'warn' | 'error' | 'busy' | 'info' | 'memory';

/** Classify a `[channel] body` note into an icon tone and display text. */
export function parseStatus(raw: string, forced?: StatusTone): { tone: StatusTone; text: string; detail?: string } {
  const m = raw.match(/^\[([a-z-]+)\]\s*([\s\S]*)$/);
  const channel = m?.[1] ?? '';
  let text = m ? m[2] : raw;
  let detail: string | undefined;
  if (channel === 'environment') {
    const [head, ...rest] = text.split('\n');
    text = head;
    detail = rest.join('\n') || undefined;
  }
  if (forced) return { tone: forced, text, detail };
  switch (channel) {
    case 'verify':
      if (/running/i.test(text)) return { tone: 'busy', text: text.replace(/^running\s*/i, 'Running '), detail };
      if (/passed/i.test(text)) return { tone: 'ok', text, detail };
      return { tone: 'warn', text, detail };
    case 'undo':
      return { tone: 'ok', text, detail };
    case 'progress':
      return { tone: 'busy', text, detail };
    case 'memory':
      return { tone: 'memory', text, detail };
    case 'context':
    case 'environment':
      return { tone: 'info', text, detail };
    default:
      return { tone: 'warn', text, detail };
  }
}

/** Render `code` spans in a status message. */
export function withCode(text: string): React.ReactNode[] {
  return text.split(/(`[^`]+`)/g).map((part, i) =>
    part.startsWith('`') && part.endsWith('`') && part.length > 2 ? (
      <code key={i} className="rounded bg-fg/[0.06] px-1 font-mono text-[11.5px] text-foreground/75">
        {part.slice(1, -1)}
      </code>
    ) : (
      part
    ),
  );
}

/** One quiet line: a small tone icon and muted text. No box, no border —
 *  the icon carries the state so notes don't compete with the reply. */
export function StatusLine({ text, tone }: { text: string; tone?: StatusTone }) {
  const s = parseStatus(text, tone);
  const icon = {
    ok: <Check strokeWidth={2.5} className="size-3 text-emerald-400/80" />,
    warn: <ShieldAlert className="size-3.5 text-amber-400/80" />,
    error: <ShieldAlert className="size-3.5 text-destructive" />,
    busy: <LoaderCircle strokeWidth={2.5} className="size-3 animate-spin text-muted-foreground" />,
    info: <Info className="size-3.5 text-muted-foreground/70" />,
    memory: <Sparkle className="size-3.5 text-violet-300/70" />,
  }[s.tone];
  return (
    <div className="flex items-start gap-2 py-0.5 text-[12.5px] leading-5 text-muted-foreground">
      <span className="flex h-5 shrink-0 items-center">{icon}</span>
      <span className="min-w-0 break-words">
        {withCode(s.text)}
        {s.detail && (
          <pre className="mt-0.5 whitespace-pre-wrap break-words font-mono text-[11.5px] text-muted-foreground/70">
            {s.detail}
          </pre>
        )}
      </span>
    </div>
  );
}

export function formatDuration(ms: number): string {
  const s = Math.max(0, Math.round(ms / 1000));
  if (s < 60) return `${s}s`;
  const m = Math.floor(s / 60);
  const rem = s % 60;
  return rem === 0 ? `${m}m` : `${m}m ${rem}s`;
}
