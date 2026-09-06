import { useEffect, useMemo, useState } from 'react';
import {
  CaretRight,
  Check,
  CircleNotch,
  FilePlus,
  FileText,
  MagnifyingGlass,
  NotePencil,
  Play,
  Sparkle,
  Terminal,
  X,
} from '@phosphor-icons/react';
import type { DiffLine, DiffPreview, ToolCall, ToolResult } from '../types';
import { Button } from '@/components/ui/button';
import { cn } from '@/lib/utils';

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
  const summary = useMemo(() => summarize(call), [call]);
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
  const summary = useMemo(() => summarize(call), [call]);

  const borderTint =
    status === 'running' ? 'border-l-mira-blue/40' :
    status === 'denied'  ? 'border-l-destructive/50' :
    'border-l-transparent';

  return (
    <div className={cn('w-full max-w-[78%] border-l-2 pl-2 -ml-2 transition-colors', borderTint)}>
      <button
        onClick={() => setExpanded((v) => !v)}
        className="flex w-full items-center gap-2.5 rounded-md px-1 py-1.5 text-left text-[13px] text-muted-foreground transition-colors hover:bg-accent/40 hover:text-foreground min-w-0"
      >
        <CaretRight
          className={cn('size-3 shrink-0 text-muted-foreground/60 transition-transform', expanded && 'rotate-90 text-muted-foreground')}
        />
        <span className="text-muted-foreground">{summary.icon}</span>
        <span className="flex-1 min-w-0 truncate">
          <span className="font-medium text-foreground">{summary.verb}</span>
          {summary.target && (
            <span className="ml-1.5 font-mono text-[12px] text-muted-foreground">{summary.target}</span>
          )}
        </span>
        <StatusMark status={status} />
      </button>

      {expanded && (
        <div className="ml-6 mt-1 mb-2 flex animate-fade-in flex-col gap-1.5">
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

function summarize(call: ToolCall): { verb: string; target: string; icon: React.ReactNode } {
  const name = call.function.name;
  const args = safeParse(call.function.arguments);
  switch (name) {
    case 'read_file':  return { verb: 'Read',     target: shortPath(args?.path),   icon: <FileText  className="size-3.5" /> };
    case 'write_file': return { verb: 'Wrote',    target: shortPath(args?.path),   icon: <FilePlus  className="size-3.5" /> };
    case 'edit_file':  return { verb: 'Edited',   target: shortPath(args?.path),   icon: <NotePencil  className="size-3.5" /> };
    case 'bash':       return { verb: 'Ran',      target: shortCmd(args?.command), icon: <Terminal  className="size-3.5" /> };
    case 'grep':       return { verb: 'Searched', target: quote(args?.pattern),    icon: <MagnifyingGlass className="size-3.5" /> };
    case 'rustfmt':    return { verb: 'Formatted', target: shortPath(args?.path),  icon: <Sparkle  className="size-3.5" /> };
    default:           return { verb: name,       target: '',                       icon: <Play      className="size-3.5" /> };
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
