import { cn } from '@/lib/utils';
import { Copy } from 'lucide-react';
import { useState } from 'react';
import { ansiToSegments, foldOutputLines } from '../../lib/toolOutput.mjs';
import { useTranscriptDisclosure } from '../TranscriptDisclosure';
/** For tools whose meaningful arg is a single prose field, pull it
 *  out so we can render it as text instead of JSON. Order below
 *  matters — earlier branches win when multiple fields exist. */
export function pickProseText(tool: string, args: any): string | null {
  if (!args) return null;
  switch (tool) {
    case 'memory_remember':
    case 'memory_append':
      return typeof args.text === 'string' ? String(args.text) : null;
    case 'memory_edit':
      if (typeof args.new_text === 'string') return String(args.new_text);
      if (typeof args.text === 'string') return String(args.text);
      return null;
    case 'agent':
      return typeof args.prompt === 'string' ? String(args.prompt) : null;
    case 'ask_user':
      return typeof args.question === 'string' ? String(args.question) : null;
    case 'task_create':
      return typeof args.subject === 'string' ? String(args.subject) : null;
    case 'git_commit':
      return typeof args.message === 'string' ? String(args.message) : null;
    case 'skill':
      return typeof args.args === 'string' ? String(args.args) : null;
    default:
      return null;
  }
}

export function CommandBlock({ command }: { command: string }) {
  // Collapse any lone `\n` literals from over-eager JSON escaping into real
  // newlines. Real shell content is already fine; we're just guarding the
  // one case where JSON serialisation leaves `\\n` in the args string.
  const normalised = command.replace(/\\n/g, '\n');
  const lines = normalised.split('\n');
  return (
    <pre className="m-0 max-h-[14vh] overflow-auto rounded-md border border-border/40 bg-mira-elev1/50 px-3 py-2 font-mono text-[12px] leading-relaxed">
      {lines.map((line, i) => (
        <div key={i} className="flex gap-2 whitespace-pre-wrap break-all">
          <span className="text-muted-foreground/60 shrink-0 select-none">
            {i === 0 ? '$' : ' '}
          </span>
          <span className="text-foreground/90">{line || ' '}</span>
        </div>
      ))}
    </pre>
  );
}

/**
 * Result of a tool call, rendered as a quiet elevated block that
 * matches the diff/prose blocks used elsewhere in the transcript.
 * No "RESULT" chrome bar — just the content with a subtle background
 * so it clearly belongs to the row above.
 *
 * Errors get a rose tint (not a full red card) so the transcript
 * still reads calmly during a run where a few tool calls fail.
 * Long outputs keep their first and last lines visible until expanded.
 */
export function ResultBlock({
  content,
  isError,
  stateKey,
  exitCode,
  timedOut = false,
  command,
}: {
  content: string;
  isError: boolean;
  stateKey: string;
  exitCode?: number | null;
  timedOut?: boolean;
  command?: string;
}) {
  const [showAll, setShowAll] = useTranscriptDisclosure(`output:${stateKey}`);
  const [copyState, setCopyState] = useState<string | null>(null);
  const folded = foldOutputLines(content);
  const overflow = folded.hidden > 0;
  const shown =
    showAll || !overflow
      ? content
      : [
          ...folded.lines.slice(0, 10),
          `… ${folded.hidden} more lines`,
          ...folded.lines.slice(10),
        ].join('\n');
  const failed = isError || (exitCode != null && exitCode !== 0);

  const copy = async (value: string, label: string) => {
    try {
      await navigator.clipboard.writeText(value);
      setCopyState(`${label} copied`);
    } catch {
      setCopyState('Copy failed');
    }
  };

  return (
    <div
      className={cn(
        'overflow-hidden rounded-md border bg-mira-elev1/50',
        failed ? 'border-rose-500/30' : 'border-border/40',
      )}
    >
      <div className="flex items-center gap-2 border-b border-border/30 bg-mira-elev1/70 px-2.5 py-1.5 text-[11px]">
        {(command !== undefined || exitCode !== undefined || timedOut) && (
          <span className={cn('font-medium', failed ? 'text-rose-300' : 'text-muted-foreground')}>
            {exitCode == null ? 'exit code unknown' : `exit code ${exitCode}`}
          </span>
        )}
        {timedOut && <span className="text-rose-300">timed out</span>}
        <div className="ml-auto flex items-center gap-1">
          {command !== undefined && (
            <button
              type="button"
              className="inline-flex items-center gap-1 rounded px-1.5 py-0.5 text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
              onClick={() => {
                void copy(command, 'Command');
              }}
            >
              <Copy className="size-3" />
              {copyState === 'Command copied' ? 'Copied' : 'Copy command'}
            </button>
          )}
          <button
            type="button"
            className="inline-flex items-center gap-1 rounded px-1.5 py-0.5 text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
            onClick={() => {
              void copy(content, 'Output');
            }}
          >
            <Copy className="size-3" />
            {copyState === 'Output copied' ? 'Copied' : 'Copy output'}
          </button>
          {copyState === 'Copy failed' && (
            <span role="status" className="text-rose-300">
              Copy failed
            </span>
          )}
        </div>
      </div>
      <pre
        className={cn(
          'm-0 max-h-[24vh] overflow-auto whitespace-pre-wrap break-words px-3 py-2 font-mono text-[11.5px] leading-relaxed',
          failed ? 'text-rose-200/90' : 'text-foreground/80',
        )}
      >
        <AnsiText text={shown} />
      </pre>
      {overflow && (
        <div className="flex items-center justify-end border-t border-border/30 bg-mira-elev1/70 px-2 py-1">
          <button
            type="button"
            className="rounded px-1.5 py-0.5 text-[11px] text-mira-blue transition-colors hover:bg-mira-blue/10"
            onClick={() => setShowAll((v) => !v)}
          >
            {showAll ? 'Show less' : `Show ${folded.hidden} more lines`}
          </button>
        </div>
      )}
    </div>
  );
}

export function AnsiText({ text }: { text: string }) {
  return (
    <>
      {ansiToSegments(text).map((segment, index) => (
        <span key={index} style={segment.style}>
          {segment.text}
        </span>
      ))}
    </>
  );
}

/** Scrollable terminal-style panel for live process output.
 *
 * Compact mode (row collapsed): shows the last 5 lines with a subtle
 * dark terminal background so a running process doesn't look hung.
 * Expanded mode (row open): full buffer in a taller scrollable block.
 *
 * A pulsing green dot indicates the process is still running; a static
 * grey dot means it has exited. */
export function LiveOutputPanel({
  lines,
  running,
  expanded,
}: {
  lines: string[];
  running: boolean;
  expanded: boolean;
}) {
  const visibleLines = expanded ? lines : lines.slice(-5);
  const totalLines = lines.length;

  return (
    <div
      className={cn(
        'ml-6 mt-0.5 rounded-md border font-mono text-[11.5px] leading-[1.55] transition-[max-height] duration-200',
        'border-border/30 bg-[#f6f7f9] dark:bg-[#1a1b1e]',
        expanded ? 'max-h-[40vh]' : 'max-h-[8rem]',
        'overflow-auto',
      )}
    >
      {/* Header strip */}
      <div className="sticky top-0 flex items-center gap-2 border-b border-border/20 bg-[#eef0f3] dark:bg-[#141416] px-3 py-1">
        <span
          className={cn(
            'size-1.5 rounded-full',
            running ? 'animate-pulse bg-emerald-400' : 'bg-muted-foreground/40',
          )}
        />
        <span className="text-[10.5px] text-muted-foreground/60">
          {running ? 'running' : 'exited'}
        </span>
        {!expanded && totalLines > 5 && (
          <span className="ml-auto text-[10px] text-muted-foreground/40">
            {totalLines} lines · showing last 5
          </span>
        )}
        {expanded && (
          <span className="ml-auto text-[10px] text-muted-foreground/40">{totalLines} lines</span>
        )}
      </div>
      {/* Output lines */}
      <div className="px-3 py-1.5">
        {visibleLines.map((line, i) => (
          <div key={i} className="whitespace-pre-wrap break-all text-[#24292f] dark:text-[#abb2bf]">
            {line ? <AnsiText text={line} /> : <span className="text-transparent">{'.'}</span>}
          </div>
        ))}
      </div>
    </div>
  );
}
