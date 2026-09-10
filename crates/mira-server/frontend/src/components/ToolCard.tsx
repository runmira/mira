import { useEffect, useMemo, useState } from 'react';
import {
  CaretDown,
  Check,
  CircleNotch,
  Play,
  X,
} from '@phosphor-icons/react';
import type { DiffLine, DiffPreview, ToolCall, ToolResult } from '../types';
import { Button } from '@/components/ui/button';
import { cn } from '@/lib/utils';
import { infoFor } from './ToolGroup';

export type ToolStatus = 'pending' | 'running' | 'denied' | 'complete';

type Props = {
  call: ToolCall;
  preview: DiffPreview | null;
  status: ToolStatus;
  result: ToolResult | null;
  onDecide: (allow: boolean) => void;
};

/** Two shapes:
 *  - `pending`   → loud approval card with Allow/Deny + preview
 *  - anything else → compact row, click to expand args + result */
export function ToolCard({ call, preview, status, result, onDecide }: Props) {
  useEffect(() => {
    if (status !== 'pending') return;
    function onKey(e: KeyboardEvent) {
      const t = e.target as HTMLElement | null;
      if (t && (t.tagName === 'TEXTAREA' || t.tagName === 'INPUT')) return;
      if (e.key === 'y' || e.key === 'Y') { e.preventDefault(); onDecide(true); }
      else if (e.key === 'n' || e.key === 'N' || e.key === 'Escape') { e.preventDefault(); onDecide(false); }
    }
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [status, onDecide]);

  if (status === 'pending') {
    return <PendingApprovalCard call={call} preview={preview} onDecide={onDecide} />;
  }
  return <CompactToolRow call={call} status={status} result={result} preview={preview} />;
}

/* ---------- pending approval ---------- */

function PendingApprovalCard({
  call, preview, onDecide,
}: { call: ToolCall; preview: DiffPreview | null; onDecide: (a: boolean) => void }) {
  // Pending = about to run → use the present-continuous verb ("Reading",
  // "Running", "Editing") so the header reads as a proposal, not a receipt.
  const summary = useMemo(() => summarize(call, 'pending'), [call]);
  const prettyArgs = useMemo(() => prettyPrint(call.function.arguments), [call.function.arguments]);
  const kindLabel = preview ? labelFor(preview.kind) : null;

  return (
    <div className="flex w-full max-w-[78%] flex-col gap-2 rounded-lg border border-amber-500/40 bg-card p-3">
      <div className="flex items-center gap-2 font-mono text-[12.5px]">
        <span className="text-mira-tool">{summary.icon}</span>
        <span className="font-medium text-foreground">
          {summary.verb} <span className="font-normal text-muted-foreground">{summary.target}</span>
        </span>
        {kindLabel && <Pill>{kindLabel}</Pill>}
        <span className="ml-auto rounded-full bg-amber-500/15 px-2 py-0.5 text-[10.5px] font-semibold uppercase tracking-wider text-amber-300">
          awaiting approval
        </span>
      </div>

      {preview ? (
        <div className="diff">
          {preview.lines.map((line, i) => <DiffRow key={i} line={line} />)}
        </div>
      ) : (
        <pre className="m-0 max-h-[22vh] overflow-auto whitespace-pre-wrap rounded-md border border-border/60 bg-background px-3 py-2 font-mono text-xs">
          {prettyArgs}
        </pre>
      )}

      <div className="flex justify-end gap-1.5 pt-0.5">
        <Button variant="outline" size="sm" onClick={() => onDecide(false)}>Deny (n)</Button>
        <Button variant="allow" size="sm" onClick={() => onDecide(true)}>Allow (y)</Button>
      </div>
    </div>
  );
}

/* ---------- compact row (running / complete / denied) ---------- */

function CompactToolRow({
  call, status, result, preview,
}: { call: ToolCall; status: ToolStatus; result: ToolResult | null; preview: DiffPreview | null }) {
  const [expanded, setExpanded] = useState(false);
  const summary = useMemo(() => summarize(call, status), [call, status]);

  return (
    <div className="w-full max-w-[78%]">
      <button
        onClick={() => setExpanded((v) => !v)}
        // Dense, inline, no card chrome — matches the AgentCard row so a
        // wall of tool calls reads as a compact list rather than a stack
        // of separate cards.
        className="flex w-full min-w-0 items-center gap-2 rounded-md px-1 py-0.5 text-left text-[13px] text-muted-foreground transition-colors hover:bg-accent/40 hover:text-foreground"
      >
        <CaretDown
          weight="bold"
          className={cn(
            'size-3 shrink-0 text-muted-foreground/60 transition-transform',
            !expanded && '-rotate-90',
            expanded && 'text-muted-foreground',
          )}
        />
        <span className="shrink-0 text-muted-foreground">{summary.icon}</span>
        <span className="min-w-0 flex-1 truncate">
          <span className="font-medium text-foreground">{summary.verb}</span>
          {summary.target && (
            <span className="ml-1.5 font-mono text-[12px] text-muted-foreground">{summary.target}</span>
          )}
        </span>
        <StatusMark status={status} />
      </button>

      {expanded && (
        <div className="ml-6 mb-1.5 mt-1 flex animate-fade-in flex-col gap-1.5">
          {preview && (
            <div className="diff compact">
              {preview.lines.slice(0, 20).map((l, i) => <DiffRow key={i} line={l} />)}
              {preview.lines.length > 20 && (
                <div className="diff-line hunk">… {preview.lines.length - 20} more lines</div>
              )}
            </div>
          )}
          {!preview && <ReconstructedPreview call={call} />}
          {result && <ResultBlock content={result.content} isError={result.is_error ?? false} />}
        </div>
      )}
    </div>
  );
}

/**
 * Fallback body shown when a live `DiffPreview` isn't available — most
 * commonly after a session reload, since the server currently doesn't
 * persist previews. Renders each known tool in a shape that reads well:
 *
 *   edit_file  → pseudo-diff (- old / + new)
 *   write_file → all-adds pseudo-diff
 *   bash       → shell-style block with a `$` prompt and real newlines
 *   grep       → `pattern` on one line + path hint
 *   read_file  → the file path
 *   rustfmt    → the file path
 *
 * Anything else falls through to the pretty-printed JSON — that's still
 * the least-bad rendering for unknown args shapes.
 */
function ReconstructedPreview({ call }: { call: ToolCall }) {
  const args = useMemo(() => safeParse(call.function.arguments), [call.function.arguments]);
  const tool = call.function.name;
  const pseudo = useMemo(() => reconstructDiff(tool, args), [tool, args]);

  if (pseudo) {
    const lines = pseudo.slice(0, 20);
    return (
      <div className="diff compact">
        {lines.map((l, i) => <DiffRow key={i} line={l} />)}
        {pseudo.length > 20 && (
          <div className="diff-line hunk">… {pseudo.length - 20} more lines</div>
        )}
      </div>
    );
  }

  // Bash: render the command as a proper shell block. JSON-escaped `\n`s
  // in the raw args become real line breaks; multi-line heredocs, chained
  // `&&`s, etc. all read like they would in a terminal.
  if (tool === 'bash' && typeof args?.command === 'string') {
    return <CommandBlock command={String(args.command)} />;
  }

  // Simple key/value tools — one-line summary with the interesting field.
  const simple = simpleArgSummary(tool, args);
  if (simple) {
    return (
      <div className="rounded-md border border-border/60 bg-background px-3 py-2 font-mono text-[12px]">
        {simple.map((row, i) => (
          <div key={i} className="flex gap-2">
            <span className="text-muted-foreground/70 shrink-0">{row.label}</span>
            <span className="min-w-0 flex-1 break-all text-foreground/90">{row.value}</span>
          </div>
        ))}
      </div>
    );
  }

  // Unknown shape — last-resort JSON.
  return (
    <pre className="m-0 max-h-[14vh] overflow-auto whitespace-pre-wrap rounded-md border border-border/60 bg-background px-3 py-2 font-mono text-[11.5px] text-muted-foreground">
      {prettyPrint(call.function.arguments)}
    </pre>
  );
}

function CommandBlock({ command }: { command: string }) {
  // Collapse any lone `\n` literals from over-eager JSON escaping into real
  // newlines. Real shell content is already fine; we're just guarding the
  // one case where JSON serialisation leaves `\\n` in the args string.
  const normalised = command.replace(/\\n/g, '\n');
  const lines = normalised.split('\n');
  return (
    <pre className="m-0 max-h-[14vh] overflow-auto rounded-md border border-border/60 bg-background px-3 py-2 font-mono text-[12px] leading-relaxed">
      {lines.map((line, i) => (
        <div key={i} className="flex gap-2 whitespace-pre-wrap break-all">
          <span className="text-muted-foreground/50 shrink-0 select-none">
            {i === 0 ? '$' : ' '}
          </span>
          <span className="text-foreground/90">{line || ' '}</span>
        </div>
      ))}
    </pre>
  );
}

function simpleArgSummary(tool: string, args: any): { label: string; value: string }[] | null {
  if (!args) return null;
  switch (tool) {
    case 'read_file':
      return typeof args.path === 'string'
        ? [{ label: 'path', value: String(args.path) }]
        : null;
    case 'rustfmt':
      return typeof args.path === 'string'
        ? [{ label: 'path', value: String(args.path) }]
        : null;
    case 'grep': {
      const rows: { label: string; value: string }[] = [];
      if (typeof args.pattern === 'string') rows.push({ label: 'pattern', value: String(args.pattern) });
      if (typeof args.path === 'string') rows.push({ label: 'path',    value: String(args.path) });
      if (typeof args.include === 'string') rows.push({ label: 'include', value: String(args.include) });
      return rows.length > 0 ? rows : null;
    }
    default:
      return null;
  }
}

/** Best-effort before/after reconstruction from raw tool args. Returns
 *  `null` for tools where a diff view doesn't apply. */
function reconstructDiff(tool: string, args: any): DiffLine[] | null {
  if (!args) return null;
  if (tool === 'edit_file' && typeof args.old_string === 'string' && typeof args.new_string === 'string') {
    const oldLines = String(args.old_string).split('\n');
    const newLines = String(args.new_string).split('\n');
    const lines: DiffLine[] = [];
    for (const t of oldLines) lines.push({ tag: 'del', text: t });
    for (const t of newLines) lines.push({ tag: 'add', text: t });
    return lines;
  }
  if (tool === 'write_file' && typeof args.content === 'string') {
    const contentLines = String(args.content).split('\n');
    // No old side (fresh write) — render as all-adds.
    return contentLines.map<DiffLine>((text) => ({ tag: 'add', text }));
  }
  return null;
}

function ResultBlock({ content, isError }: { content: string; isError: boolean }) {
  const [showAll, setShowAll] = useState(false);
  const lines = content.split('\n');
  const CAP = 6;
  const overflow = lines.length > CAP;
  const shown = showAll ? content : lines.slice(0, CAP).join('\n');

  return (
    <div className="overflow-hidden rounded-md border border-border/60 bg-background">
      <div className="flex items-center border-b border-border/60 px-2.5 py-1 text-[11px] uppercase tracking-wider text-muted-foreground">
        <span className={cn('flex-1', isError && 'text-destructive')}>
          {isError ? 'error output' : 'result'}
        </span>
        {overflow && (
          <button
            className="rounded px-1.5 py-0.5 text-[11px] normal-case tracking-normal text-mira-blue transition-colors hover:bg-mira-blue/10"
            onClick={() => setShowAll((v) => !v)}
          >
            {showAll ? 'show less' : `show all (${lines.length} lines)`}
          </button>
        )}
      </div>
      <pre className={cn(
        'm-0 max-h-[24vh] overflow-auto whitespace-pre-wrap break-words px-3 py-2 font-mono text-[11.5px] leading-relaxed',
        isError ? 'text-[#f7a5b0]' : 'text-muted-foreground',
      )}>
        {shown}{!showAll && overflow ? `\n… ${lines.length - CAP} more lines` : ''}
      </pre>
    </div>
  );
}

/* ---------- bits ---------- */

function StatusMark({ status }: { status: ToolStatus }) {
  switch (status) {
    case 'running':  return <CircleNotch className="size-3 shrink-0 animate-spin text-mira-blue" />;
    case 'denied':   return <X className="size-3.5 shrink-0 text-destructive" />;
    case 'complete': return <Check className="size-3.5 shrink-0 text-emerald-500" />;
    default:         return null;
  }
}

function Pill({ children }: { children: React.ReactNode }) {
  return (
    <span className="rounded-full border border-border bg-background px-1.5 py-0.5 text-[10.5px] uppercase tracking-wider text-muted-foreground">
      {children}
    </span>
  );
}

function DiffRow({ line }: { line: DiffLine }) {
  if (line.tag === 'hunkgap') return <div className="diff-line hunk">···</div>;
  const cls = line.tag === 'add' ? 'add' : line.tag === 'del' ? 'del' : 'ctx';
  const prefix = line.tag === 'add' ? '+' : line.tag === 'del' ? '-' : ' ';
  return (
    <div className={`diff-line ${cls}`}>
      <span className="prefix">{prefix}</span>
      <span className="text">{line.text}</span>
    </div>
  );
}

/** Verb + target + icon for a single tool row. Tense follows Codex:
 *  in-flight/pending calls read as present-continuous ("Reading foo.rs"),
 *  finished/denied calls read as past ("Read foo.rs"). Verb map is
 *  centralised in `ToolGroup.infoFor` so grouped and ungrouped rows stay
 *  in lockstep. Target extraction stays here because it depends on
 *  per-tool arg shape (`args.pattern` for grep, `args.command` for bash). */
function summarize(
  call: ToolCall,
  status: ToolStatus,
): { verb: string; target: string; icon: React.ReactNode } {
  const info = infoFor(call.function.name);
  const active = status === 'pending' || status === 'running';
  const verb = active ? info.verbCont : info.verbPast;
  const Icon = info.Icon;
  const icon =
    call.function.name in KNOWN
      ? <Icon className="size-3.5" />
      : <Play className="size-3.5" />;

  const args = safeParse(call.function.arguments);
  const target = pickTarget(call.function.name, args);
  return { verb, target, icon };
}

/** Tools with a specific `infoFor` entry — kept in sync with ToolGroup so
 *  the icon fallback triggers for genuinely unknown names only. */
const KNOWN: Record<string, true> = {
  read_file: true, write_file: true, edit_file: true, grep: true, glob: true,
  find_symbol: true, bash: true, rustfmt: true, web_fetch: true, web_search: true,
  git_diff: true, git_status: true, git_log: true, git_commit: true,
  memory_read: true, memory_search: true, memory_append: true, memory_edit: true,
  memory_remember: true,
};

/** Per-tool target extractor. Uses the argument key most useful to a
 *  human skimming the row — the file path, the command, the query — so
 *  the row reads as `Verb <what>` at a glance. */
function pickTarget(tool: string, args: any): string {
  switch (tool) {
    case 'read_file':
    case 'write_file':
    case 'edit_file':
    case 'rustfmt':
      return shortPath(args?.path);
    case 'bash':
      return shortCmd(args?.command);
    case 'grep':
      return quote(args?.pattern);
    case 'glob':
      return args?.pattern ? String(args.pattern) : '';
    case 'find_symbol':
      return args?.name ? String(args.name) : args?.query ? String(args.query) : '';
    case 'web_fetch':
      return args?.url ? String(args.url) : '';
    case 'web_search':
      return quote(args?.query);
    case 'git_diff':
    case 'git_log':
    case 'git_status':
      return '';
    case 'git_commit':
      return args?.message ? shortCmd(args.message) : '';
    case 'memory_read':
    case 'memory_search':
    case 'memory_append':
    case 'memory_edit':
    case 'memory_remember':
      return args?.path ? shortPath(args.path) : args?.query ? String(args.query) : '';
    default:
      return '';
  }
}

function safeParse(s: string): any {
  try { return JSON.parse(s); } catch { return null; }
}

function shortPath(p: string | undefined): string {
  if (!p) return '';
  const parts = String(p).split('/');
  if (parts.length <= 3) return String(p);
  return '…/' + parts.slice(-2).join('/');
}

function shortCmd(cmd: string | undefined): string {
  if (!cmd) return '';
  const one = String(cmd).replace(/\s+/g, ' ').trim();
  return one.length > 60 ? one.slice(0, 60) + '…' : one;
}

function quote(s: string | undefined): string {
  return s ? `"${s}"` : '';
}

function labelFor(k: DiffPreview['kind']): string {
  switch (k) {
    case 'edit':      return 'edit';
    case 'overwrite': return 'overwrite';
    case 'create':    return 'create';
  }
}

function prettyPrint(json: string): string {
  try { return JSON.stringify(JSON.parse(json), null, 2); }
  catch { return json; }
}
