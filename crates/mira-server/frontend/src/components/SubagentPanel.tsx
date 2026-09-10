import { useMemo, useState } from 'react';
import { CaretRight, CircleNotch, Info, WarningCircle, X } from '@phosphor-icons/react';
import type { Entry } from '../App';
import { groupAgentRuns } from '../App';
import type { ToolCall, ToolResult } from '../types';
import type { ToolStatus } from './ToolCard';
import { AssistantContent } from './AssistantContent';
import { Markdown } from './Markdown';
import { identityFor, extractPrompt, stripAgentIdMarker } from './AgentCard';
import { ToolCard } from './ToolCard';
import { ToolGroup } from './ToolGroup';
import { cn } from '@/lib/utils';

/** Right-side pane that shows one or more subagents in detail. Opens
 *  when the user clicks an AgentCard row in the transcript and adds a
 *  tab; further clicks add more tabs. Tabs are dismissible; when the
 *  last one closes the whole pane closes.
 *
 *  Visual model: reads like a mini chat, not a form. Background matches
 *  the main pane (bg-background), separated only by a subtle border to
 *  the left. The active tab is a grey rounded-full capsule; the "i"
 *  button in the header reveals the agent's prompt on demand. */
export type SubagentTab = {
  callId: string;
  call: ToolCall;
  status: ToolStatus;
  result: ToolResult | null;
  /** Live child transcript, built up from `subagent_*` WS frames. Same
   *  Entry shape as the parent transcript so the panel body can reuse
   *  parent-side rendering conventions. Empty until events start flowing. */
  streamEntries: Entry[];
  streamDone: boolean;
};

type Props = {
  tabs: SubagentTab[];
  activeCallId: string | null;
  onSelectTab: (callId: string) => void;
  onCloseTab: (callId: string) => void;
  onClose: () => void;
};

export function SubagentPanel({ tabs, activeCallId, onSelectTab, onCloseTab, onClose }: Props) {
  const active = tabs.find((t) => t.callId === activeCallId) ?? tabs[0];
  if (!active) return null;

  return (
    <aside className="flex h-full min-w-0 flex-col overflow-hidden border-l border-border bg-background">
      {/* tab strip — h-11 matches the main pane's header so the two
          dividers line up exactly across the vertical border. */}
      <div className="flex h-11 shrink-0 items-center gap-1 border-b border-border/60 pl-2 pr-1.5">
        <div className="flex min-w-0 flex-1 items-center gap-1 overflow-x-auto">
          {tabs.map((t) => (
            <TabCapsule
              key={t.callId}
              tab={t}
              active={t.callId === active.callId}
              onSelect={() => onSelectTab(t.callId)}
              onClose={() => onCloseTab(t.callId)}
            />
          ))}
        </div>
        <button
          type="button"
          onClick={onClose}
          title="Close panel"
          className="shrink-0 rounded-md p-1.5 text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
        >
          <X className="size-4" />
        </button>
      </div>

      {/* active tab body */}
      <div className="min-h-0 flex-1 overflow-y-auto">
        <TabBody tab={active} />
      </div>
    </aside>
  );
}

/** Rounded-full "capsule" tab. Active gets a grey `bg-secondary` fill;
 *  inactive is transparent with hover feedback. Icon + color come from
 *  the agent's identity so tabs are visually distinct at a glance. */
function TabCapsule({
  tab, active, onSelect, onClose,
}: {
  tab: SubagentTab;
  active: boolean;
  onSelect: () => void;
  onClose: () => void;
}) {
  const identity = identityFor(tab.callId);
  return (
    <div
      role="tab"
      aria-selected={active}
      onClick={onSelect}
      className={cn(
        'group inline-flex cursor-pointer items-center gap-1.5 rounded-full px-3 py-1 text-[12.5px] transition-colors',
        active
          ? 'bg-secondary text-foreground'
          : 'text-muted-foreground hover:bg-accent/50 hover:text-foreground',
      )}
    >
      <identity.Icon className={cn('size-3.5', identity.textClass)} weight="fill" />
      <span className={cn('font-medium', active && identity.textClass)}>
        {identity.name}
      </span>
      <button
        type="button"
        aria-label="Close tab"
        onClick={(e) => {
          e.stopPropagation();
          onClose();
        }}
        className={cn(
          'ml-0.5 rounded-full p-0.5 text-muted-foreground/60 transition-opacity hover:bg-background/60 hover:text-foreground',
          active ? 'opacity-100' : 'opacity-0 group-hover:opacity-100',
        )}
      >
        <X className="size-3" />
      </button>
    </div>
  );
}

/** Chat-like body: mirrors what the parent conversation looks like. No
 *  form scaffolding (no "PROMPT" / "SUMMARY" headers, no boxed pre).
 *  The prompt lives behind the "i" toggle in the header since Codex's
 *  agent windows don't show it and the user typically already knows
 *  what they asked for. */
function TabBody({ tab }: { tab: SubagentTab }) {
  const identity = identityFor(tab.callId);
  const prompt = extractPrompt(tab.call.function.arguments);
  // Strip the `[mira-agent-id:X]\n` marker before displaying — it's an
  // internal handle for reload hydration, not user-facing text.
  const summary = tab.result?.content
    ? stripAgentIdMarker(tab.result.content).trim()
    : '';
  const isError = tab.result?.is_error === true;
  const isRunning = tab.status === 'running' || tab.status === 'pending';
  const [infoOpen, setInfoOpen] = useState(false);

  return (
    <div className="mx-auto flex max-w-2xl flex-col gap-4 px-5 pb-8 pt-5">
      <header className="flex items-center gap-2.5">
        <identity.Icon className={cn('size-5', identity.textClass)} weight="fill" />
        <div className="flex min-w-0 flex-col leading-tight">
          <span className={cn('text-[15px] font-semibold', identity.textClass)}>
            {identity.name}
          </span>
          <span className="text-[11px] text-muted-foreground">Subagent</span>
        </div>
        <button
          type="button"
          onClick={() => setInfoOpen((v) => !v)}
          title={infoOpen ? 'Hide agent info' : 'About this agent'}
          className={cn(
            'ml-1 shrink-0 rounded-full p-1 transition-colors',
            infoOpen
              ? 'bg-secondary text-foreground'
              : 'text-muted-foreground hover:bg-accent hover:text-foreground',
          )}
        >
          <Info className="size-4" weight={infoOpen ? 'fill' : 'regular'} />
        </button>
        <div className="ml-auto">
          <StatusPill status={tab.status} isError={isError} />
        </div>
      </header>

      {infoOpen && (
        <div className="rounded-lg border border-border/60 bg-card/40 p-4">
          <div className="mb-2 flex items-center gap-2">
            <Info className={cn('size-4', identity.textClass)} weight="fill" />
            <span className="text-[13px] font-medium text-foreground">
              About {identity.name}
            </span>
          </div>
          <p className="mb-3 text-[12px] leading-relaxed text-muted-foreground">
            A bounded subagent spawned by the parent — cold context, its own
            tools, and one summary back to the parent when done.
          </p>
          <div className="mb-1.5 text-[10.5px] font-semibold uppercase tracking-wider text-muted-foreground/70">
            Task
          </div>
          <div className="max-h-[40vh] overflow-y-auto whitespace-pre-wrap text-[13px] leading-relaxed text-foreground/85">
            {prompt || 'No task was provided.'}
          </div>
        </div>
      )}

      {/* Live child transcript — renders like a mini chat as tokens
          arrive. Falls through to the final summary block only when
          there's no live stream (e.g. resumed sessions where the events
          weren't captured). Uses the same `groupAgentRuns` folding as
          the parent transcript so consecutive same-name tool calls
          collapse into ToolGroup chips here too. */}
      {tab.streamEntries.length > 0 ? (
        <StreamedEntries
          entries={tab.streamEntries}
          isRunning={isRunning}
          streamDone={tab.streamDone}
        />
      ) : (
        <>
          {isRunning && !summary && (
            <div className="flex items-center gap-2 text-[13px] text-muted-foreground">
              <CircleNotch className="size-3.5 animate-spin text-mira-blue" />
              <span>Working…</span>
            </div>
          )}

          {summary && !isError && (
            <div className="max-w-full">
              <SubagentResult text={summary} />
            </div>
          )}

          {summary && isError && (
            <div className="max-w-full whitespace-pre-wrap text-[13px] text-destructive">
              {summary}
            </div>
          )}
        </>
      )}
    </div>
  );
}

/** Folds and renders a subagent's live transcript. Applies the same
 *  `groupAgentRuns` grouping as the main pane so consecutive reads/greps
 *  collapse into a single `ToolGroup` chip. Non-groupable entries pass
 *  through to `SubagentEntryView`. */
function StreamedEntries({
  entries,
  isRunning,
  streamDone,
}: {
  entries: Entry[];
  isRunning: boolean;
  streamDone: boolean;
}) {
  const groups = useMemo(() => groupAgentRuns(entries), [entries]);
  return (
    <div className="flex flex-col gap-1">
      {groups.map((item, i) => {
        if (item.kind === 'tool-group') {
          return (
            <ToolGroup
              key={`g-${i}`}
              entries={item.entries.map((e) => ({
                call: e.call,
                preview: e.preview,
                status: e.status,
                result: e.result,
              }))}
            />
          );
        }
        if (item.kind === 'agent-group') {
          // Nested `agent` spawns inside a subagent are rare (depth cap = 2)
          // but possible. Render each as a plain tool row rather than
          // recursing into another AgentCard/AgentGroup here — the parent
          // panel already owns the tab lifecycle, and a nested spawn is
          // usually just visual noise inside a specialist child.
          return (
            <div key={`g-${i}`} className="flex flex-col gap-0.5">
              {item.entries.map((e, j) => (
                <SubagentEntryView key={`ag-${i}-${j}`} entry={e} />
              ))}
            </div>
          );
        }
        return <SubagentEntryView key={`e-${i}`} entry={item.entry} />;
      })}
      {isRunning && !streamDone && (
        <div className="mt-1 flex items-center gap-2 text-[12px] text-muted-foreground">
          <CircleNotch className="size-3 animate-spin text-mira-blue" />
          <span>Working…</span>
        </div>
      )}
    </div>
  );
}

/** Renders a single entry from a live subagent transcript. Assistant
 *  text goes through `AssistantContent` so it looks native; tool calls
 *  use the shared `ToolCard` compact row. Warning frames get a small
 *  amber chip so `hit max_rounds`, verify failures, etc. are visible. */
function SubagentEntryView({ entry }: { entry: Entry }) {
  switch (entry.kind) {
    case 'msg': {
      if (entry.msg.role !== 'assistant') return null;
      const text = (entry.msg.content ?? '').trim();
      if (!text) return null;
      return (
        <div className="max-w-full">
          <SubagentResult text={text} />
        </div>
      );
    }
    case 'tool':
      return (
        <ToolCard
          call={entry.call}
          preview={entry.preview}
          status={entry.status}
          result={entry.result}
          onDecide={() => { /* subagents auto-approve — no user gate here */ }}
        />
      );
    case 'warning':
      return (
        <div className="inline-flex w-fit items-start gap-1.5 rounded-md border border-amber-500/25 bg-amber-500/[0.06] px-2 py-1 text-[11.5px] text-amber-300">
          <span className="font-semibold">!</span>
          <span className="break-words">{entry.text}</span>
        </div>
      );
    case 'error':
      return (
        <div className="font-mono text-[11.5px] text-destructive">
          error: {entry.text}
        </div>
      );
  }
}

function StatusPill({ status, isError }: { status: ToolStatus; isError: boolean }) {
  if (isError) {
    return (
      <span className="inline-flex items-center gap-1 rounded-full border border-destructive/40 bg-destructive/10 px-2 py-0.5 text-[11px] text-destructive">
        <WarningCircle className="size-3" weight="fill" />
        error
      </span>
    );
  }
  switch (status) {
    case 'pending':
      return (
        <span className="rounded-full border border-amber-500/30 bg-amber-500/[0.08] px-2 py-0.5 text-[11px] text-amber-300">
          awaiting approval
        </span>
      );
    case 'running':
      return (
        <span className="inline-flex items-center gap-1 rounded-full border border-mira-blue/30 bg-mira-blue/[0.08] px-2 py-0.5 text-[11px] text-mira-blue">
          <CircleNotch className="size-3 animate-spin" />
          working
        </span>
      );
    case 'denied':
      return (
        <span className="rounded-full border border-border bg-secondary px-2 py-0.5 text-[11px] text-muted-foreground">
          denied
        </span>
      );
    case 'complete':
      return (
        <span className="rounded-full border border-emerald-500/30 bg-emerald-500/[0.08] px-2 py-0.5 text-[11px] text-emerald-400">
          done
        </span>
      );
  }
}

/* ---------- structured-result rendering ---------- */

/** Renders a subagent's final response. Agents like `explore`, `reviewer`,
 *  and `sentinel` return JSON that matches their `response_schema` — dumping
 *  the raw JSON blob into the markdown renderer just displays braces and
 *  escaped quotes, so try to parse first and lay out known fields nicely.
 *  Falls back to plain markdown when the text isn't structured (e.g. the
 *  `coder`/`documenter` types, or free-form models that ignored the schema). */
function SubagentResult({ text }: { text: string }) {
  const parsed = useMemo(() => tryParseStructured(text), [text]);
  if (!parsed) return <AssistantContent text={text} />;
  return <StructuredView data={parsed} />;
}

type StructuredResult = {
  /** Free-form prose field — `summary` (explore) or `notes` (sentinel). */
  narrative?: { label: string; text: string };
  /** Enum-like conclusion field — `verdict` from reviewer/sentinel. */
  verdict?: { label: string; value: string; tone: 'ok' | 'warn' | 'bad' };
  /** Boolean flags like `mission_creep` — rendered as a colored chip. */
  flags?: { label: string; value: boolean }[];
  /** Arrays of citation-shaped items: findings / issues / unrelated_changes. */
  items?: { label: string; entries: StructuredItem[] };
  /** Any additional top-level fields we don't have a rich renderer for.
   *  Rendered as a plain key/value list so nothing gets silently dropped. */
  extras?: { key: string; value: unknown }[];
};

type StructuredItem = {
  path?: string;
  line?: number;
  severity?: string;
  summary?: string;
  suggestion?: string;
  note?: string;
  reason?: string;
};

const NARRATIVE_KEYS = ['summary', 'notes'];
const ITEM_KEYS = ['findings', 'issues', 'unrelated_changes'];
const VERDICT_TONE: Record<string, 'ok' | 'warn' | 'bad'> = {
  ok: 'ok',
  on_brief: 'ok',
  concerning: 'warn',
  changes_requested: 'warn',
  off_brief: 'bad',
};

/** Try to interpret `text` as one of the known subagent response schemas.
 *  Returns null when the text isn't valid JSON or doesn't look structured
 *  — the caller renders it as plain markdown in that case. */
function tryParseStructured(text: string): StructuredResult | null {
  const trimmed = text.trim();
  if (!trimmed.startsWith('{') || !trimmed.endsWith('}')) return null;
  let obj: unknown;
  try {
    obj = JSON.parse(trimmed);
  } catch {
    return null;
  }
  if (!obj || typeof obj !== 'object' || Array.isArray(obj)) return null;
  const record = obj as Record<string, unknown>;

  const out: StructuredResult = {};
  const consumed = new Set<string>();

  for (const key of NARRATIVE_KEYS) {
    const v = record[key];
    if (typeof v === 'string' && v.trim()) {
      out.narrative = { label: key, text: v };
      consumed.add(key);
      break;
    }
  }

  if (typeof record.verdict === 'string' && record.verdict) {
    const value = record.verdict;
    out.verdict = { label: 'verdict', value, tone: VERDICT_TONE[value] ?? 'warn' };
    consumed.add('verdict');
  }

  const flags: { label: string; value: boolean }[] = [];
  for (const [k, v] of Object.entries(record)) {
    if (typeof v === 'boolean') {
      flags.push({ label: k, value: v });
      consumed.add(k);
    }
  }
  if (flags.length > 0) out.flags = flags;

  for (const key of ITEM_KEYS) {
    const v = record[key];
    if (Array.isArray(v) && v.length > 0) {
      out.items = { label: key, entries: v.map(coerceItem) };
      consumed.add(key);
      break;
    } else if (Array.isArray(v)) {
      // Empty array — mark consumed so it doesn't spill into `extras`, and
      // remember it so the view can show a "no findings" line.
      out.items = { label: key, entries: [] };
      consumed.add(key);
      break;
    }
  }

  const extras: { key: string; value: unknown }[] = [];
  for (const [k, v] of Object.entries(record)) {
    if (consumed.has(k)) continue;
    if (v === null || v === undefined) continue;
    extras.push({ key: k, value: v });
  }
  if (extras.length > 0) out.extras = extras;

  // If nothing was interpreted (unknown JSON shape), let the caller fall
  // back to markdown so we don't produce an empty box.
  if (!out.narrative && !out.verdict && !out.items && !out.flags && !out.extras) {
    return null;
  }
  return out;
}

function coerceItem(raw: unknown): StructuredItem {
  if (!raw || typeof raw !== 'object' || Array.isArray(raw)) {
    return { note: String(raw) };
  }
  const r = raw as Record<string, unknown>;
  const pick = (k: string): string | undefined =>
    typeof r[k] === 'string' ? (r[k] as string) : undefined;
  return {
    path: pick('path'),
    line: typeof r.line === 'number' ? (r.line as number) : undefined,
    severity: pick('severity'),
    summary: pick('summary'),
    suggestion: pick('suggestion'),
    note: pick('note'),
    reason: pick('reason'),
  };
}

function StructuredView({ data }: { data: StructuredResult }) {
  return (
    <div className="flex flex-col gap-3">
      {data.verdict && <VerdictPill verdict={data.verdict} />}
      {data.narrative && (
        <div className="md">
          <Markdown text={data.narrative.text} />
        </div>
      )}
      {data.flags && data.flags.length > 0 && (
        <div className="flex flex-wrap gap-1.5">
          {data.flags.map((f) => (
            <span
              key={f.label}
              className={cn(
                'rounded-full border px-2 py-0.5 text-[11px]',
                f.value
                  ? 'border-amber-500/30 bg-amber-500/[0.08] text-amber-300'
                  : 'border-border bg-secondary text-muted-foreground',
              )}
            >
              {humanize(f.label)}: {f.value ? 'yes' : 'no'}
            </span>
          ))}
        </div>
      )}
      {data.items && <ItemsSection label={data.items.label} entries={data.items.entries} />}
      {data.extras && data.extras.length > 0 && <ExtrasBlock extras={data.extras} />}
    </div>
  );
}

function VerdictPill({ verdict }: { verdict: NonNullable<StructuredResult['verdict']> }) {
  const cls =
    verdict.tone === 'ok'
      ? 'border-emerald-500/30 bg-emerald-500/[0.08] text-emerald-400'
      : verdict.tone === 'warn'
        ? 'border-amber-500/30 bg-amber-500/[0.08] text-amber-300'
        : 'border-destructive/40 bg-destructive/10 text-destructive';
  return (
    <div className="flex items-center gap-2 text-[12px] text-muted-foreground">
      <span className="text-[10.5px] font-semibold uppercase tracking-wider">
        {humanize(verdict.label)}
      </span>
      <span className={cn('rounded-full border px-2 py-0.5 text-[11.5px] font-medium', cls)}>
        {humanize(verdict.value)}
      </span>
    </div>
  );
}

function ItemsSection({ label, entries }: { label: string; entries: StructuredItem[] }) {
  return (
    <div className="flex flex-col gap-2">
      <div className="text-[10.5px] font-semibold uppercase tracking-wider text-muted-foreground">
        {humanize(label)} {entries.length > 0 && <span className="text-muted-foreground/60">({entries.length})</span>}
      </div>
      {entries.length === 0 ? (
        <div className="text-[12.5px] text-muted-foreground/70">None reported.</div>
      ) : (
        <div className="flex flex-col gap-1.5">
          {entries.map((it, i) => (
            <ItemRow key={i} item={it} />
          ))}
        </div>
      )}
    </div>
  );
}

function ItemRow({ item }: { item: StructuredItem }) {
  const where = item.path
    ? item.line != null
      ? `${item.path}:${item.line}`
      : item.path
    : null;
  const body = item.summary ?? item.note ?? item.reason ?? '';
  return (
    <div className="rounded-md border border-border/60 bg-secondary/30 p-2.5">
      <div className="flex items-center gap-2">
        {item.severity && (
          <span
            className={cn(
              'rounded px-1.5 py-0.5 text-[10.5px] font-semibold uppercase tracking-wider',
              severityClass(item.severity),
            )}
          >
            {item.severity}
          </span>
        )}
        {body && <span className="text-[13px] text-foreground/90">{body}</span>}
      </div>
      {where && (
        <div className="mt-1 flex items-center gap-1 font-mono text-[11.5px] text-muted-foreground/80">
          <CaretRight className="size-3" />
          <span className="truncate">{where}</span>
        </div>
      )}
      {item.suggestion && (
        <div className="mt-1.5 rounded-md border border-emerald-500/25 bg-emerald-500/[0.06] px-2 py-1 text-[12px] text-emerald-200/90">
          <span className="mr-1 font-semibold">suggestion:</span>
          {item.suggestion}
        </div>
      )}
    </div>
  );
}

function severityClass(sev: string): string {
  switch (sev.toLowerCase()) {
    case 'critical':
      return 'bg-destructive/20 text-destructive';
    case 'major':
    case 'high':
      return 'bg-amber-500/20 text-amber-300';
    case 'minor':
    case 'medium':
      return 'bg-mira-blue/20 text-mira-blue';
    default:
      return 'bg-secondary text-muted-foreground';
  }
}

/** Renders anything the schema-aware view didn't recognize as key/value
 *  rows. Prevents silent data loss when a subagent's schema drifts from
 *  the shapes we know about. */
function ExtrasBlock({ extras }: { extras: { key: string; value: unknown }[] }) {
  return (
    <div className="flex flex-col gap-1 border-t border-border/50 pt-2">
      {extras.map(({ key, value }) => (
        <div key={key} className="text-[12.5px]">
          <span className="text-muted-foreground">{humanize(key)}:</span>{' '}
          <span className="font-mono text-foreground/85 break-all">{formatExtra(value)}</span>
        </div>
      ))}
    </div>
  );
}

function formatExtra(v: unknown): string {
  if (typeof v === 'string') return v;
  try {
    return JSON.stringify(v);
  } catch {
    return String(v);
  }
}

function humanize(s: string): string {
  return s.replace(/_/g, ' ').replace(/\b\w/g, (c) => c.toUpperCase());
}
