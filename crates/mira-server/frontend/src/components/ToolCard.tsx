import { useEffect, useMemo, useState } from 'react';
import type { DiffLine, DiffPreview, ToolCall, ToolResult } from '../types';

export type ToolStatus = 'pending' | 'running' | 'denied' | 'complete';

type Props = {
  call: ToolCall;
  preview: DiffPreview | null;
  status: ToolStatus;
  result: ToolResult | null;
  onDecide: (allow: boolean) => void;
};

/**
 * Renders one tool call. Two shapes:
 *
 * - **pending approval** → full card with the diff/args and Allow/Deny.
 *   Needs to be visually loud because the user has to decide.
 *
 * - **running / complete / denied** → compact one-line row (`Read README.md`,
 *   `Ran ls -la`) with a status indicator. Click to expand and see the raw
 *   args + a truncated result. Codex/Claude-Code style — keeps the
 *   transcript readable when the model does five tool calls in a row.
 */
export function ToolCard({ call, preview, status, result, onDecide }: Props) {
  useEffect(() => {
    if (status !== 'pending') return;
    function onKey(e: KeyboardEvent) {
      const target = e.target as HTMLElement | null;
      if (target && (target.tagName === 'TEXTAREA' || target.tagName === 'INPUT')) return;
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
    <div className="tool-card status-pending">
      <div className="tool-card-head">
        <span className="tool-icon">{summary.icon}</span>
        <span className="tool-card-name">{summary.verb} <span className="tool-target">{summary.target}</span></span>
        {kindLabel && <span className="pill">{kindLabel}</span>}
        <span className="badge badge-pending">awaiting approval</span>
      </div>

      {preview ? (
        <div className="diff">
          {preview.lines.map((line, i) => <DiffRow key={i} line={line} />)}
        </div>
      ) : (
        <pre className="args">{prettyArgs}</pre>
      )}

      <div className="tool-card-actions">
        <button className="deny" onClick={() => onDecide(false)}>Deny (n)</button>
        <button className="allow" onClick={() => onDecide(true)}>Allow (y)</button>
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

  return (
    <div className={`tool-row status-${status} ${expanded ? 'expanded' : ''}`}>
      <button className="tool-row-head" onClick={() => setExpanded((v) => !v)}>
        <span className={`tool-row-chevron ${expanded ? 'open' : ''}`}>▸</span>
        <span className="tool-icon">{summary.icon}</span>
        <span className="tool-row-label">
          <span className="verb">{summary.verb}</span>{' '}
          <span className="target">{summary.target}</span>
        </span>
        <span className="tool-row-status"><StatusMark status={status} /></span>
      </button>

      {expanded && (
        <div className="tool-row-body">
          {preview && (
            <div className="diff compact">
              {preview.lines.slice(0, 20).map((l, i) => <DiffRow key={i} line={l} />)}
              {preview.lines.length > 20 && (
                <div className="diff-line hunk">… {preview.lines.length - 20} more lines</div>
              )}
            </div>
          )}
          {!preview && (
            <pre className="args mini">{prettyPrint(call.function.arguments)}</pre>
          )}
          {result && (
            <ResultBlock content={result.content} isError={result.is_error ?? false} />
          )}
        </div>
      )}
    </div>
  );
}

function ResultBlock({ content, isError }: { content: string; isError: boolean }) {
  const [showAll, setShowAll] = useState(false);
  const lines = content.split('\n');
  const CAP = 6;
  const overflow = lines.length > CAP;
  const shown = showAll ? content : lines.slice(0, CAP).join('\n');

  return (
    <div className={`tool-result ${isError ? 'error' : ''}`}>
      <div className="tool-result-head">
        <span>{isError ? 'error output' : 'result'}</span>
        {overflow && (
          <button className="tool-result-toggle" onClick={() => setShowAll((v) => !v)}>
            {showAll ? 'show less' : `show all (${lines.length} lines)`}
          </button>
        )}
      </div>
      <pre>{shown}{!showAll && overflow ? `\n… ${lines.length - CAP} more lines` : ''}</pre>
    </div>
  );
}

/* ---------- bits ---------- */

function StatusMark({ status }: { status: ToolStatus }) {
  switch (status) {
    case 'running':  return <span className="spinner" aria-label="running" />;
    case 'denied':   return <span className="mark denied">✕</span>;
    case 'complete': return <span className="mark done">✓</span>;
    default:         return null;
  }
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

/** Turn a raw ToolCall into a human "verb + target" for the compact row. */
function summarize(call: ToolCall): { verb: string; target: string; icon: string } {
  const name = call.function.name;
  const args = safeParse(call.function.arguments);
  switch (name) {
    case 'read_file':   return { verb: 'Read',    target: shortPath(args?.path), icon: '📄' };
    case 'write_file':  return { verb: 'Wrote',   target: shortPath(args?.path), icon: '📝' };
    case 'edit_file':   return { verb: 'Edited',  target: shortPath(args?.path), icon: '✏️' };
    case 'bash':        return { verb: 'Ran',     target: shortCmd(args?.command), icon: '⚡' };
    case 'grep':        return { verb: 'Searched', target: quote(args?.pattern), icon: '🔍' };
    case 'rustfmt':     return { verb: 'Formatted', target: shortPath(args?.path), icon: '🎨' };
    default:            return { verb: name, target: '', icon: '⚙' };
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
  return s ? `“${s}”` : '';
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
