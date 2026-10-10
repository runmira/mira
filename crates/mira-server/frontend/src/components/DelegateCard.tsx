/**
 * `delegate_task` in the transcript — a hand-off, not a generic tool call.
 *
 * The tool hands one task to another engine (Mira's own model, or an
 * installed agent) and waits, read-only, up to ten minutes for the answer.
 * A generic "Delegated …" tool row said almost nothing about that: not who
 * took it, not that it was a hand-off at all, and the one thing that
 * matters while you wait — *what it is doing* — was an expanding spinner.
 *
 * This card is built around the same "agents as companions" language the
 * rest of Mira uses. The row carries the two ends of the gesture (Mira's
 * orb → the engine's mark), who received it, and the live stage; opening it
 * shows the brief that was handed over, what the helper can touch (read and
 * search only), the staged progress, and finally the answer as prose.
 *
 * It is deliberately *only* for `delegate_task`. An external agent's own
 * `Task`/`Agent` call is a different thing — a subagent inside that agent —
 * and folds into the ordinary tool rows (see `agentTools.ts`).
 */
import { cn } from '@/lib/utils';
import {
  ArrowRight,
  Check,
  ChevronDown,
  CircleAlert,
  Eye,
  FileText,
  Globe,
  LoaderCircle,
  NotebookPen,
  Search,
  Sparkle,
  Terminal,
} from 'lucide-react';
import { useEffect, useMemo, useState } from 'react';
import type { ToolCall, ToolResult } from '../types';
import { AgentIcon } from './AgentIcon';
import { Markdown } from './Markdown';
import type { ToolStatus } from './ToolCard';
import { Collapse } from './ui/Collapse';

/* ---------- engine presentation ---------- */

export type DelegateEngine = {
  id: string;
  name: string;
  kind: 'mira' | 'agent';
};

/** One visible step a delegated child took, streamed live via
 *  `delegate_progress`. `kind` is a coarse verb bucket; `text` is the
 *  human-readable step. */
export type DelegateStep = {
  kind: string;
  text: string;
  /** Epoch ms the step arrived, for relative ordering. */
  at?: number;
};

/** Display name per agent driver. The id stays the key (`claude-code`), the
 *  name is what the engine's own UI calls it. Unknown engines read as a
 *  title-cased id rather than a raw slug. */
const AGENT_ENGINES: Record<string, string> = {
  'claude-code': 'Claude Code',
  claude: 'Claude',
  codex: 'Codex',
  opencode: 'OpenCode',
  gemini: 'Gemini',
  grok: 'Grok',
  cursor: 'Cursor',
  antigravity: 'Antigravity',
};

/** The engine a delegate call named, with `mira` as the default the server
 *  applies when `engine` is absent. */
export function delegateEngine(id: string | null | undefined): DelegateEngine {
  const e = (id ?? '').trim() || 'mira';
  if (e === 'mira') return { id: 'mira', name: 'Mira', kind: 'mira' };
  const name =
    AGENT_ENGINES[e] ?? e.replace(/[-_]+/g, ' ').replace(/\b\w/g, (c) => c.toUpperCase());
  return { id: e, name, kind: 'agent' };
}

/** True for this tool specifically. Mira's own tool is `delegate_task`;
 *  an MCP host may namespace it (`mcp__mira__delegate_task`), so the match
 *  is on the final segment. An agent's own `Task`/`Agent` call is mapped to
 *  `delegate` upstream and deliberately does *not* match — that is a
 *  subagent inside the agent, not this hand-off. */
export { isDelegateTaskName } from '../lib/delegateTools';

/** The brief and the engine a `delegate_task` call asked for. The brief is a
 *  single prose field, so it is rendered as prose, never as JSON. */
export function parseDelegateArgs(argsJson: string): { prompt: string; engine: string } {
  let prompt = '';
  let engine = 'mira';
  try {
    const a = JSON.parse(argsJson) as { prompt?: unknown; engine?: unknown };
    if (typeof a?.prompt === 'string') prompt = a.prompt;
    if (typeof a?.engine === 'string' && a.engine.trim()) engine = a.engine;
  } catch {
    /* keep defaults — a bad args blob renders an empty brief, not a crash */
  }
  return { prompt, engine };
}

/** The answer text from the tool result, trimmed. Empty when the call is
 *  still running or failed without a message. */
function answerOf(result: ToolResult | null): string {
  return (result?.content ?? '').trim();
}

/* ---------- stage model ---------- */

type Stage = 'handoff' | 'working' | 'answered';

const STAGES: { key: Stage; label: string }[] = [
  { key: 'handoff', label: 'Handed off' },
  { key: 'working', label: 'Working' },
  { key: 'answered', label: 'Answered' },
];

/** Which stage the run is at. `pending` is the approval wait (the hand-off
 *  hasn't happened yet), `running` is the helper at work, and any terminal
 *  state lands on `answered` — the answer itself may still be an error. */
function stageOf(status: ToolStatus): Stage {
  if (status === 'pending') return 'handoff';
  if (status === 'complete' || status === 'denied') return 'answered';
  return 'working';
}

/* ---------- card ---------- */

export function DelegateCard({
  call,
  status,
  result,
  startedAt,
  steps,
}: {
  call: ToolCall;
  status: ToolStatus;
  result: ToolResult | null;
  /** Epoch ms the call started, when known — powers the elapsed clock. */
  startedAt?: number | null;
  /** Live activity streamed from the child (newest last). */
  steps?: DelegateStep[];
}) {
  const { prompt, engine: engineId } = useMemo(
    () => parseDelegateArgs(call.function.arguments),
    [call.function.arguments],
  );
  const engine = delegateEngine(engineId);
  const isError = result?.is_error === true;
  const stage = stageOf(status);
  const active = status === 'running' || status === 'pending';
  const [expanded, setExpanded] = useState(false);

  const answer = answerOf(result);
  const elapsed = useElapsed(startedAt, active || undefined);
  const live = steps ?? [];
  const last = live[live.length - 1];

  return (
    <div className="w-full max-w-[78%]">
      <div
        role="button"
        tabIndex={0}
        onClick={() => setExpanded((v) => !v)}
        onKeyDown={(e) => {
          if (e.key === 'Enter' || e.key === ' ') setExpanded((v) => !v);
        }}
        className="group flex w-full min-w-0 cursor-pointer items-center gap-2 rounded-md px-1 py-0.5 text-left text-[13px] transition-colors hover:bg-accent/40"
      >
        <HandoffMarks engine={engine} active={active} />
        <span className="flex min-w-0 flex-1 items-center gap-1.5">
          <span className="shrink-0 text-muted-foreground">Handed off to</span>
          <span
            className="truncate font-medium"
            style={engine.kind === 'agent' ? undefined : { color: 'rgb(var(--mira-purple))' }}
          >
            {engine.name}
          </span>
          {/* While working, the live step is more informative than the word
              "working"; fall back to the stage chip when there's nothing yet. */}
          {active && last ? (
            <span className="flex min-w-0 items-center gap-1.5 text-[11.5px] text-mira-blue">
              <LoaderCircle className="size-3 shrink-0 animate-spin" />
              <span className="truncate" title={last.text}>
                {last.text}
              </span>
            </span>
          ) : (
            <StageChip stage={stage} isError={isError} active={active} />
          )}
        </span>
        {elapsed && (
          <span className="ml-2 shrink-0 font-mono text-[11px] tabular-nums text-muted-foreground/55">
            {elapsed}
          </span>
        )}
        <ChevronDown
          strokeWidth={2.5}
          className={cn(
            'size-3 shrink-0 text-foreground/70 transition-all',
            !expanded && '-rotate-90',
            !expanded && 'opacity-0 group-hover:opacity-100',
          )}
        />
        <StatusMark status={status} isError={isError} />
      </div>

      <Collapse open={expanded}>
        <DelegatePanel
          engine={engine}
          prompt={prompt}
          answer={answer}
          isError={isError}
          stage={stage}
          steps={live}
        />
      </Collapse>
    </div>
  );
}

/* ---------- marks ---------- */

/** Mira's own mark: a soft orb in the app's purple→cyan accent, matched to
 *  the little helper faces' weight without borrowing one of their bodies. */
function MiraOrb({ size = 18 }: { size?: number }) {
  return (
    <span
      aria-hidden
      className="inline-grid shrink-0 place-items-center rounded-full ring-1 ring-black/10"
      style={{
        width: size,
        height: size,
        background:
          'radial-gradient(circle at 32% 28%, rgb(var(--mira-cyan)) 0%, rgb(var(--mira-blue)) 48%, rgb(var(--mira-purple)) 100%)',
      }}
    >
      <span className="size-[3px] rounded-full bg-white/85" />
    </span>
  );
}

/** The two ends of the hand-off: where it came from, and where it went.
 *  When the target is Mira itself the orb is shown once — handing a task to
 *  the same mark twice would be noise, not a gesture. */
function HandoffMarks({ engine, active }: { engine: DelegateEngine; active: boolean }) {
  return (
    <span className="flex shrink-0 items-center gap-1">
      <MiraOrb size={18} />
      {engine.kind === 'agent' && (
        <>
          <ArrowRight
            className={cn(
              'size-3 text-muted-foreground/50',
              active && 'animate-pulse text-mira-blue/70',
            )}
          />
          <AgentIcon kind={engine.id} name={engine.name} size="sm" tile />
        </>
      )}
    </span>
  );
}

/* ---------- stage chip / track ---------- */

function StageChip({
  stage,
  isError,
  active,
}: {
  stage: Stage;
  isError: boolean;
  active: boolean;
}) {
  if (isError) {
    return (
      <span className="inline-flex shrink-0 items-center gap-1 text-[11.5px] text-destructive">
        <CircleAlert className="size-3" fill="currentColor" />
        failed
      </span>
    );
  }
  const label =
    stage === 'handoff' ? 'waiting to start' : stage === 'working' ? 'working' : 'answered';
  return (
    <span
      className={cn(
        'inline-flex shrink-0 items-center gap-1 text-[11.5px]',
        active ? 'text-mira-blue' : 'text-muted-foreground/70',
      )}
    >
      {active && <LoaderCircle className="size-3 animate-spin" />}
      {label}
    </span>
  );
}

/** The three-stage track. Completed stages get a filled tick, the live one a
 *  spinner, the rest stay as quiet dots — a glance says how far the hand-off
 *  has got without a second line of text. */
function StageTrack({ stage, isError }: { stage: Stage; isError: boolean }) {
  const currentIdx = STAGES.findIndex((x) => x.key === stage);

  return (
    <div className="flex items-center gap-2">
      {STAGES.map((s, i) => {
        // A stage is "done" once we're past it, or once the whole hand-off
        // has answered. The current stage shows a spinner, or a failure mark
        // when the run ended in an error.
        const done = i < currentIdx || stage === 'answered';
        const live = i === currentIdx && stage !== 'answered' && !isError;
        const failed = isError && i === currentIdx;
        return (
          <div key={s.key} className="flex min-w-0 flex-1 items-center gap-2">
            <span className="flex shrink-0 items-center gap-1.5">
              <span className="grid size-3.5 shrink-0 place-items-center">
                {failed ? (
                  <CircleAlert className="size-3 text-destructive" fill="currentColor" />
                ) : done ? (
                  <Check className="size-3.5 text-emerald-500" strokeWidth={3} />
                ) : live ? (
                  <LoaderCircle className="size-3 animate-spin text-mira-blue" />
                ) : (
                  <span className="size-1.5 rounded-full bg-muted-foreground/30" />
                )}
              </span>
              <span
                className={cn(
                  'truncate text-[11.5px]',
                  done
                    ? 'text-foreground/70'
                    : live
                      ? 'text-mira-blue'
                      : failed
                        ? 'text-destructive'
                        : 'text-muted-foreground/50',
                )}
              >
                {s.label}
              </span>
            </span>
            {i < STAGES.length - 1 && (
              <span
                className={cn('h-px min-w-2 flex-1', done ? 'bg-emerald-500/40' : 'bg-border/60')}
              />
            )}
          </div>
        );
      })}
    </div>
  );
}

/* ---------- panel ---------- */

function DelegatePanel({
  engine,
  prompt,
  answer,
  isError,
  stage,
  steps,
}: {
  engine: DelegateEngine;
  prompt: string;
  answer: string;
  isError: boolean;
  stage: Stage;
  steps: DelegateStep[];
}) {
  return (
    <div className="ml-6 mb-1.5 mt-1 flex animate-fade-in flex-col gap-2">
      <div className="overflow-hidden rounded-xl border border-border/50 bg-mira-elev1/50">
        {/* Header: who is doing the work. */}
        <div className="flex items-center gap-2 border-b border-border/40 bg-mira-elev1/70 px-3 py-2">
          <HandoffMarks engine={engine} active={stage === 'working'} />
          <span className="min-w-0 truncate text-[12.5px] font-medium text-foreground">
            {engine.kind === 'mira' ? 'Mira’s own model' : engine.name}
          </span>
          <span className="rounded-full border border-border/60 bg-background px-1.5 py-0.5 font-mono text-[10px] uppercase tracking-wider text-muted-foreground">
            {engine.id}
          </span>
          <span className="ml-auto flex shrink-0 items-center gap-1 text-[11px] text-muted-foreground/70">
            <Eye /> read-only
          </span>
        </div>

        <div className="space-y-3 px-3 py-2.5">
          <StageTrack stage={stage} isError={isError} />

          {steps.length > 0 && (
            <section className="space-y-1">
              <SectionLabel>Activity</SectionLabel>
              <ActivityLog steps={steps} active={stage === 'working'} />
            </section>
          )}

          {prompt && (
            <section className="space-y-1">
              <SectionLabel>Brief</SectionLabel>
              <div className="whitespace-pre-wrap break-words rounded-lg border border-border/40 bg-background/40 px-2.5 py-2 text-[12.5px] leading-relaxed text-foreground/85">
                {prompt}
              </div>
            </section>
          )}

          <section className="space-y-1">
            <SectionLabel>What it can do</SectionLabel>
            <ul className="space-y-0.5 text-[12px] text-muted-foreground">
              <Permission icon={Search} text="Read and search this project" />
              <Permission icon={FileText} text="Answer with findings" />
              <Permission icon={CircleAlert} text="Edits and commands are refused" muted />
            </ul>
          </section>

          {answer && (
            <section className="space-y-1">
              <SectionLabel>{isError ? 'Couldn’t finish' : 'Answer'}</SectionLabel>
              <div
                className={cn(
                  'max-h-[46vh] overflow-auto rounded-lg border px-3 py-2',
                  isError
                    ? 'border-destructive/30 bg-destructive/5 text-[12.5px] leading-relaxed text-destructive/90'
                    : 'border-border/40 bg-background/40',
                )}
              >
                {isError ? (
                  <div className="whitespace-pre-wrap break-words">{answer}</div>
                ) : (
                  <Markdown text={answer} />
                )}
              </div>
            </section>
          )}
        </div>
      </div>
    </div>
  );
}

/** The child's live step log: one quiet row per action it took, oldest first,
 *  with the newest at the bottom and a trailing spinner while it runs. Reads
 *  like a condensed trace of the work being done on the user's behalf. */
function ActivityLog({ steps, active }: { steps: DelegateStep[]; active: boolean }) {
  // Keep the tail in view as steps arrive; the whole card is already inside
  // the panel's scroll, so no fixed height here — just the full list.
  const tail = steps.slice(-24);
  return (
    <ol className="space-y-0.5 rounded-lg border border-border/40 bg-background/40 px-2 py-1.5">
      {tail.map((s, i) => (
        <li key={i} className="flex items-start gap-2 text-[12px] leading-snug">
          <span className="grid h-[18px] w-3 shrink-0 place-items-center text-muted-foreground/50">
            <StepIcon kind={s.kind} />
          </span>
          <span className="min-w-0 flex-1 truncate text-foreground/80" title={s.text}>
            {s.text}
          </span>
        </li>
      ))}
      {active && (
        <li className="flex items-center gap-2 text-[12px] text-mira-blue">
          <LoaderCircle className="size-3 shrink-0 animate-spin" />
          <span>working…</span>
        </li>
      )}
    </ol>
  );
}

function StepIcon({ kind }: { kind: string }) {
  switch (kind) {
    case 'read':
      return <FileText className="size-3" />;
    case 'search':
      return <Search className="size-3" />;
    case 'run':
      return <Terminal className="size-3" />;
    case 'fetch':
      return <Globe className="size-3" />;
    case 'write':
    case 'edit':
      return <NotebookPen className="size-3" />;
    case 'note':
      return <CircleAlert className="size-3" />;
    default:
      return <Sparkle className="size-3" />;
  }
}

function SectionLabel({ children }: { children: React.ReactNode }) {
  return (
    <div className="text-[10.5px] font-semibold uppercase tracking-wider text-muted-foreground/70">
      {children}
    </div>
  );
}

function Permission({
  icon: Icon,
  text,
  muted,
}: {
  icon: React.ComponentType<{ className?: string }>;
  text: string;
  muted?: boolean;
}) {
  return (
    <li className="flex items-center gap-1.5">
      <Icon
        className={cn(
          'size-3 shrink-0',
          muted ? 'text-muted-foreground/40' : 'text-emerald-500/70',
        )}
      />
      <span className={cn(muted && 'text-muted-foreground/60')}>{text}</span>
    </li>
  );
}

/* ---------- status ---------- */

function StatusMark({ status, isError }: { status: ToolStatus; isError: boolean }) {
  if (isError)
    return <CircleAlert className="size-3.5 shrink-0 text-destructive" fill="currentColor" />;
  switch (status) {
    case 'running':
    case 'pending':
      return <LoaderCircle className="size-3 shrink-0 animate-spin text-mira-blue" />;
    case 'denied':
      return <CircleAlert className="size-3.5 shrink-0 text-muted-foreground" />;
    case 'complete':
      return <Check className="size-3.5 shrink-0 text-emerald-500" strokeWidth={3} />;
    default:
      return null;
  }
}

/* ---------- helpers ---------- */

/** A live "1m 12s" clock while a delegation runs. Ticks once a second only
 *  while active; when it settles, the final duration is frozen so the row
 *  stops re-rendering. */
function useElapsed(startedAt: number | null | undefined, active?: boolean): string | null {
  const [now, setNow] = useState(() => Date.now());
  const [settledAt, setSettledAt] = useState<number | null>(null);
  useEffect(() => {
    if (active) {
      setSettledAt(null);
      setNow(Date.now());
      const t = setInterval(() => setNow(Date.now()), 1000);
      return () => clearInterval(t);
    }
    setSettledAt(Date.now());
  }, [active]);
  if (startedAt == null) return null;
  const end = active ? now : (settledAt ?? Date.now());
  const ms = Math.max(0, end - startedAt);
  if (ms < 1000) return active ? '0s' : null;
  const s = Math.round(ms / 1000);
  if (s < 60) return `${s}s`;
  return `${Math.floor(s / 60)}m ${s % 60}s`;
}
