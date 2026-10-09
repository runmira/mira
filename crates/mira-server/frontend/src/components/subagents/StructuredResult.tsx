import { cn } from '@/lib/utils';
import { ChevronRight, Info } from 'lucide-react';
import { useMemo, useState } from 'react';
import { AssistantContent } from '../AssistantContent';
import { Markdown } from '../Markdown';
/* ---------- structured-result rendering ---------- */

/** Renders a subagent's final response. Agents like `explore`, `reviewer`,
 *  and `sentinel` return JSON that matches their `response_schema` — dumping
 *  the raw JSON blob into the markdown renderer just displays braces and
 *  escaped quotes, so try to parse first and lay out known fields nicely.
 *  Falls back to plain markdown when the text isn't structured (e.g. the
 *  `coder`/`documenter` types, or free-form models that ignored the schema).
 *
 *  Models routinely wrap the required JSON in a ```json``` fence and
 *  prefix a sentence of prose ("Final answer:"). We split the two so
 *  the prose still reads as prose and the fenced JSON gets the
 *  structured view instead of being dumped as a code block. */
export function SubagentResult({ text }: { text: string }) {
  const split = useMemo(() => extractStructured(text), [text]);
  if (!split) return <AssistantContent text={text} />;
  return (
    <div className="flex flex-col gap-3">
      {split.prose && <AssistantContent text={split.prose} />}
      <StructuredView data={split.data} />
    </div>
  );
}

/** Try three shapes, in order, and return the first that parses to a
 *  recognisable structured result:
 *   1. The whole text is a bare `{ ... }` object.
 *   2. The text ends with a ```json ...``` fenced block; the prose
 *      before the fence is preserved and rendered above the parsed view.
 *   3. The text contains an inline `{ ... }` block we can extract.
 *  Returns `null` when nothing structured could be salvaged — the
 *  caller falls back to the plain markdown renderer. */
export function extractStructured(text: string): { data: StructuredResult; prose: string } | null {
  const raw = text.trim();

  // Case 1 — bare JSON object.
  if (raw.startsWith('{') && raw.endsWith('}')) {
    const parsed = tryParseStructured(raw);
    if (parsed) return { data: parsed, prose: '' };
  }

  // Case 2 — fenced ```json ... ``` (or plain ``` ... ``` where the
  // body happens to be JSON). Take the LAST fence in the text so a
  // model that quotes a JSON snippet mid-reasoning and then emits
  // the real final answer still wins.
  const fenceRe = /```(?:json)?\s*\n?([\s\S]*?)\n?```/gi;
  let lastMatch: RegExpExecArray | null = null;
  let m: RegExpExecArray | null;
  while ((m = fenceRe.exec(raw)) !== null) {
    lastMatch = m;
  }
  if (lastMatch) {
    const body = lastMatch[1].trim();
    if (body.startsWith('{') && body.endsWith('}')) {
      const parsed = tryParseStructured(body);
      if (parsed) {
        const before = raw.slice(0, lastMatch.index).trim();
        const after = raw.slice(lastMatch.index + lastMatch[0].length).trim();
        // Prose is anything outside the fence. Prefer whichever side
        // has content; if both do, keep both joined by a paragraph
        // break so the shape reads naturally.
        const prose = [before, after].filter(Boolean).join('\n\n');
        return { data: parsed, prose };
      }
    }
  }

  // Case 3 — an inline `{ ... }` block somewhere in the text (last
  // resort — handles models that emit prose + a bare object without
  // a fence).
  const openIdx = raw.indexOf('{');
  const closeIdx = raw.lastIndexOf('}');
  if (openIdx >= 0 && closeIdx > openIdx) {
    const body = raw.slice(openIdx, closeIdx + 1);
    const parsed = tryParseStructured(body);
    if (parsed) {
      const before = raw.slice(0, openIdx).trim();
      const after = raw.slice(closeIdx + 1).trim();
      const prose = [before, after].filter(Boolean).join('\n\n');
      return { data: parsed, prose };
    }
  }

  return null;
}

export type StructuredResult = {
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

export type StructuredItem = {
  path?: string;
  line?: number;
  severity?: string;
  summary?: string;
  suggestion?: string;
  note?: string;
  reason?: string;
};

export const NARRATIVE_KEYS = ['summary', 'notes'];

export const ITEM_KEYS = ['findings', 'issues', 'unrelated_changes'];

export const VERDICT_TONE: Record<string, 'ok' | 'warn' | 'bad'> = {
  ok: 'ok',
  on_brief: 'ok',
  concerning: 'warn',
  changes_requested: 'warn',
  off_brief: 'bad',
};

/** Try to interpret `text` as one of the known subagent response schemas.
 *  Returns null when the text isn't valid JSON or doesn't look structured
 *  — the caller renders it as plain markdown in that case. */
export function tryParseStructured(text: string): StructuredResult | null {
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

export function coerceItem(raw: unknown): StructuredItem {
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

export function StructuredView({ data }: { data: StructuredResult }) {
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
                'rounded-full bg-secondary/70 px-2 py-0.5 text-[11px]',
                f.value ? 'text-amber-300' : 'text-muted-foreground',
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

export function VerdictPill({ verdict }: { verdict: NonNullable<StructuredResult['verdict']> }) {
  const toneText =
    verdict.tone === 'ok'
      ? 'text-emerald-400'
      : verdict.tone === 'warn'
        ? 'text-amber-300'
        : 'text-destructive';
  return (
    <div className="flex items-center gap-2 text-[12px] text-muted-foreground">
      <span className="text-[10.5px] font-semibold uppercase tracking-wider">
        {humanize(verdict.label)}
      </span>
      <span
        className={cn(
          'rounded-full bg-secondary/70 px-2 py-0.5 text-[11.5px] font-medium',
          toneText,
        )}
      >
        {humanize(verdict.value)}
      </span>
    </div>
  );
}

export function ItemsSection({ label, entries }: { label: string; entries: StructuredItem[] }) {
  return (
    <div className="flex flex-col gap-2">
      <div className="text-[10.5px] font-semibold uppercase tracking-wider text-muted-foreground">
        {humanize(label)}{' '}
        {entries.length > 0 && <span className="text-muted-foreground/60">({entries.length})</span>}
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

export function ItemRow({ item }: { item: StructuredItem }) {
  const where = item.path ? (item.line != null ? `${item.path}:${item.line}` : item.path) : null;
  const body = item.summary ?? item.note ?? item.reason ?? '';
  return (
    <div className="rounded-xl bg-secondary/40 p-2.5">
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
          <ChevronRight className="size-3" />
          <span className="truncate">{where}</span>
        </div>
      )}
      {item.suggestion && (
        <div className="mt-1.5 rounded-md bg-background/60 px-2 py-1 text-[12px] text-foreground/80">
          <span className="mr-1 font-semibold text-emerald-400">suggestion:</span>
          {item.suggestion}
        </div>
      )}
    </div>
  );
}

export function severityClass(sev: string): string {
  // Fill-only variants — no borders. Tone is carried by the label
  // text; the background stays a subtle neutral tint of the same hue
  // so the pill still scans by color without shouting.
  switch (sev.toLowerCase()) {
    case 'critical':
      return 'bg-destructive/15 text-destructive';
    case 'major':
    case 'high':
      return 'bg-amber-500/15 text-amber-300';
    case 'minor':
    case 'medium':
      return 'bg-mira-blue/15 text-mira-blue';
    default:
      return 'bg-secondary/70 text-muted-foreground';
  }
}

/** Renders anything the schema-aware view didn't recognize as key/value
 *  rows. Prevents silent data loss when a subagent's schema drifts from
 *  the shapes we know about. */
export function ExtrasBlock({ extras }: { extras: { key: string; value: unknown }[] }) {
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

export function formatExtra(v: unknown): string {
  if (typeof v === 'string') return v;
  try {
    return JSON.stringify(v);
  } catch {
    return String(v);
  }
}

export function humanize(s: string): string {
  return s.replace(/_/g, ' ').replace(/\b\w/g, (c) => c.toUpperCase());
}

/** Approve/Deny card the SubagentPanel renders when a review-required
 *  child has produced its final summary and is blocked on human input.
 *  Approve returns the summary to the parent (optionally with a note
 *  prepended); Deny turns the tool result into an error whose body is
 *  the note. The card is dismissed as soon as the user picks either
 *  side — App.tsx clears `pendingReview` locally, and the backend's
 *  tool_end lands moments later with the resolved result. */
export function ReviewCard({
  review,
  identityTextClass,
  onApprove,
  onDeny,
}: {
  review: { promptId: string; summary: string };
  identityTextClass: string;
  onApprove: (note?: string) => void;
  onDeny: (note?: string) => void;
}) {
  const [note, setNote] = useState('');

  return (
    <div className="overflow-hidden rounded-2xl border border-border/40 bg-card/80 backdrop-blur">
      <div className="flex items-center gap-2 px-4 pt-3.5 pb-3">
        <Info className={cn('size-3.5', identityTextClass)} fill="currentColor" />
        <span className="text-[12.5px] font-semibold tracking-tight text-foreground">
          Review required
        </span>
        <span className="ml-auto rounded-full bg-secondary/70 px-2 py-0.5 text-[10px] font-semibold uppercase tracking-[0.11em] text-muted-foreground">
          awaiting decision
        </span>
      </div>
      <div className="border-t border-border/30 px-4 py-3">
        <p className="mb-2.5 text-[12px] leading-relaxed text-muted-foreground">
          The subagent finished and is waiting for you to approve its summary before the parent gets
          it. Deny to send back an error instead.
        </p>
        <div className="mb-3 max-h-[36vh] overflow-y-auto whitespace-pre-wrap rounded-xl bg-secondary/40 px-3 py-2 text-[13px] leading-relaxed text-foreground/85">
          {review.summary || '(empty summary)'}
        </div>
        <textarea
          value={note}
          onChange={(e) => setNote(e.target.value)}
          placeholder="Optional note (prepended on approve, sent as reason on deny)"
          rows={2}
          className="w-full resize-none rounded-md bg-secondary/50 px-2.5 py-1.5 text-[12px] leading-relaxed outline-none placeholder:text-muted-foreground/50 focus:bg-secondary/70"
        />
      </div>
      <div className="flex items-center justify-end gap-1.5 border-t border-border/30 bg-background/30 px-4 py-2.5">
        <button
          type="button"
          onClick={() => onDeny(note.trim() ? note.trim() : undefined)}
          className="rounded-md px-2.5 py-1.5 text-[11.5px] font-medium text-muted-foreground transition-colors hover:bg-secondary/60 hover:text-foreground"
        >
          Deny
        </button>
        <button
          type="button"
          onClick={() => onApprove(note.trim() ? note.trim() : undefined)}
          className="inline-flex items-center gap-1.5 rounded-full bg-foreground px-3.5 py-1.5 text-[11.5px] font-semibold text-background transition-all hover:brightness-95"
        >
          Approve
        </button>
      </div>
    </div>
  );
}
