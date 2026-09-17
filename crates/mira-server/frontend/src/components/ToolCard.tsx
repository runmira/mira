import { useMemo, useState } from 'react';
import {
  CaretDown,
  Check,
  CircleNotch,
  FileText,
  Play,
  X,
} from '@phosphor-icons/react';
import type { DiffLine, DiffPreview, Mode, ToolCall, ToolResult } from '../types';
import { cn } from '@/lib/utils';
import { infoFor } from './ToolGroup';

/** Tools whose *result* is redundant with what we're already showing.
 *
 *   - `write_file` / `edit_file` — the diff card is the point;
 *     "wrote N bytes" adds nothing.
 *   - `memory_remember` / `memory_append` / `memory_edit` — the
 *     content IS the args block; the result is just an ack like
 *     "remembered → path".
 *   - `task_create` / `task_update` — same shape; result is a
 *     one-line ack for something the row header already summarizes.
 *
 * Everything else (`bash`, `grep`, `read_file`, `find_*`,
 * `web_fetch`, `web_search`, `memory_read`, `memory_search`,
 * `task_list`, `task_get`, `git_*`, `glob`) still surfaces its
 * result — the output IS the point.
 */
const RESULT_SUPPRESSED: Record<string, true> = {
  write_file: true,
  edit_file: true,
  memory_remember: true,
  memory_append: true,
  memory_edit: true,
  task_create: true,
  task_update: true,
};

/** Tools where the row header already carries the interesting arg
 *  (glob pattern, grep pattern, read path, etc.), so an expanded
 *  arg preview would duplicate what's above. When one of these is
 *  expanded we skip straight to the result and don't render an
 *  arg-echo block. Bash is deliberately NOT here — the row header
 *  truncates at 60 chars and users often need the full command. */
const ARG_PREVIEW_REDUNDANT: Record<string, true> = {
  glob: true,
  grep: true,
  read_file: true,
  rustfmt: true,
  find_symbol: true,
  find_references: true,
  find_callers: true,
  web_fetch: true,
  web_search: true,
  memory_read: true,
  memory_search: true,
  task_get: true,
  task_list: true,
  git_status: true,
  git_diff: true,
  git_log: true,
};

/** Tools that get the Codex-style diff card treatment (header row
 *  outside with `+N -N` summary, elevated card with its own header +
 *  line numbers). Any tool that produces a `DiffPreview` today —
 *  write, edit, create — falls in this bucket. */
const DIFF_CARD_TOOLS: Record<string, true> = {
  write_file: true,
  edit_file: true,
};

export type ToolStatus = 'pending' | 'running' | 'denied' | 'complete';

type Props = {
  call: ToolCall;
  preview: DiffPreview | null;
  status: ToolStatus;
  result: ToolResult | null;
  onDecide: (allow: boolean) => void;
  /** Current session mode. `manual` and `auto` are gating modes where
   *  the Approve card can offer an "Always allow" affordance that
   *  bumps to `edit` (auto-approve everything unless a rule blocks).
   *  Omitted for callers that don't want the mode-swap UI (e.g. the
   *  subagent panel replay). */
  mode?: Mode;
  onSetMode?: (m: Mode) => void;
};

/** Two shapes:
 *  - `pending`   → preview card with inline Allow / Deny (+ Always allow
 *                  when the current mode still gates). Y/N keyboard
 *                  shortcut is bound at the App level so it fires no
 *                  matter which card is on screen.
 *  - anything else → compact row, click to expand args + result. */
export function ToolCard({ call, preview, status, result, onDecide, mode, onSetMode }: Props) {
  if (status === 'pending') {
    return (
      <PendingApprovalCard
        call={call}
        preview={preview}
        onDecide={onDecide}
        mode={mode}
        onSetMode={onSetMode}
      />
    );
  }
  return <CompactToolRow call={call} status={status} result={result} preview={preview} />;
}

/* ---------- pending approval ---------- */

function PendingApprovalCard({
  call, preview, onDecide, mode, onSetMode,
}: {
  call: ToolCall;
  preview: DiffPreview | null;
  onDecide: (allow: boolean) => void;
  mode?: Mode;
  onSetMode?: (m: Mode) => void;
}) {
  // Pending = about to run → use the present-continuous verb ("Reading",
  // "Running", "Editing") so the header reads as a proposal, not a receipt.
  const summary = useMemo(() => summarize(call, 'pending'), [call]);
  const prettyArgs = useMemo(() => prettyPrint(call.function.arguments), [call.function.arguments]);
  const kindLabel = preview ? labelFor(preview.kind) : null;
  // "Always allow" bumps the session out of a gating mode into `edit`
  // (auto-approve everything unless a rule blocks). Only offered when
  // the current mode actually gates — hidden in `edit`/`yolo` where
  // the option is a no-op and would just clutter the card.
  const showAlways = onSetMode != null && (mode === 'manual' || mode === 'auto' || mode === 'plan');

  return (
    <div className="flex w-full max-w-[78%] flex-col gap-2 overflow-hidden rounded-2xl border border-border/40 bg-card/80 p-3.5 backdrop-blur">
      <div className="flex items-center gap-2 font-mono text-[12.5px]">
        <span className="text-mira-tool">{summary.icon}</span>
        <span className="font-medium text-foreground">
          {summary.verb} <span className="font-normal text-muted-foreground">{summary.target}</span>
        </span>
        {kindLabel && <Pill>{kindLabel}</Pill>}
        <span className="ml-auto rounded-full bg-secondary px-2 py-0.5 text-[10.5px] font-semibold uppercase tracking-wider text-muted-foreground">
          awaiting approval
        </span>
      </div>

      {preview ? (
        <div className="diff">
          {preview.lines.map((line, i) => <DiffRow key={i} line={line} />)}
        </div>
      ) : (
        <pre className="m-0 max-h-[22vh] overflow-auto whitespace-pre-wrap rounded-md bg-background/60 px-3 py-2 font-mono text-xs text-muted-foreground">
          {prettyArgs}
        </pre>
      )}

      {/* Buttons live on the card itself so approvals stay next to the
       *  diff/args they act on. Y/N keyboard shortcut is bound at the
       *  App level and applies to the first pending card. Same
       *  monochrome CTA style as AskUserCard: filled tiles, no color
       *  strokes; the primary action is a solid foreground pill. */}
      <div className="flex items-center gap-1.5 pt-1">
        {showAlways && (
          <button
            type="button"
            onClick={() => { onSetMode?.('edit'); onDecide(true); }}
            className="mr-auto inline-flex items-center rounded-md px-2.5 py-1.5 text-[11.5px] font-medium text-muted-foreground transition-colors hover:bg-secondary/60 hover:text-foreground"
            title="Allow this and switch the session to 'Auto everything' — future writes and commands run without asking (unless a rule blocks)"
          >
            Always allow
          </button>
        )}
        <button
          type="button"
          onClick={() => onDecide(false)}
          className={cn(
            'rounded-md px-2.5 py-1.5 text-[11.5px] font-medium text-muted-foreground transition-colors hover:bg-secondary/60 hover:text-foreground',
            !showAlways && 'ml-auto',
          )}
          title="Deny (n)"
        >
          Deny
        </button>
        <button
          type="button"
          onClick={() => onDecide(true)}
          className="inline-flex items-center gap-1 rounded-full bg-foreground px-3.5 py-1.5 text-[11.5px] font-semibold text-background transition-all hover:brightness-95"
          title="Allow (y)"
        >
          Allow
        </button>
      </div>
      <div className="text-right text-[10.5px] text-muted-foreground/60">
        <kbd className="rounded bg-secondary/70 px-1 py-0.5 font-mono text-[10px]">y</kbd> allow · <kbd className="rounded bg-secondary/70 px-1 py-0.5 font-mono text-[10px]">n</kbd> deny
      </div>
    </div>
  );
}

/* ---------- compact row (running / complete / denied) ---------- */

function CompactToolRow({
  call, status, result, preview,
}: {
  call: ToolCall;
  status: ToolStatus;
  result: ToolResult | null;
  preview: DiffPreview | null;
}) {
  const [expanded, setExpanded] = useState(false);
  const summary = useMemo(() => summarize(call, status), [call, status]);

  const isDiffCardTool = call.function.name in DIFF_CARD_TOOLS;
  // Fallback: reconstruct a pseudo-diff from raw args when the server
  // didn't stream a live preview (session reloads, tests). Same
  // treatment either way — we just need the numbers for +N -N.
  const effectiveDiff: DiffLine[] | null = useMemo(() => {
    if (preview) return preview.lines;
    if (isDiffCardTool) {
      const args = safeParse(call.function.arguments);
      return reconstructDiff(call.function.name, args);
    }
    return null;
  }, [preview, isDiffCardTool, call.function.name, call.function.arguments]);
  const stats = useMemo(
    () => (effectiveDiff ? countDiff(effectiveDiff) : null),
    [effectiveDiff],
  );
  const filePath = useMemo(() => {
    if (!isDiffCardTool) return '';
    const args = safeParse(call.function.arguments);
    return typeof args?.path === 'string' ? String(args.path) : '';
  }, [isDiffCardTool, call.function.arguments]);

  // Filename-ish targets get a subtle inline-code pill so the "verb
  // target" line reads as "prefix + identifier" the way Codex renders
  // ("Read find_symbol.rs"). Non-path targets (bash commands, grep
  // patterns) stay as plain mono text — a pill around a shell
  // command would look strange.
  const targetIsPath = isPathishTool(call.function.name);

  return (
    <div className="w-full max-w-[78%]">
      <button
        onClick={() => setExpanded((v) => !v)}
        // Codex-look: no leading caret, no leading icon. Verb + target
        // as flowing text, trailing chevron that only appears on hover
        // or when expanded (same rule as the "Worked for" header). The
        // row remains fully clickable — the chevron is a visual hint,
        // not the click target.
        className="group flex w-full min-w-0 items-center gap-1.5 rounded-md px-1 py-0.5 text-left text-[13px] transition-colors hover:bg-accent/40"
      >
        <span className="min-w-0 flex-1 truncate">
          <span className="text-muted-foreground">{summary.verb}</span>
          {summary.target && (
            targetIsPath ? (
              <span className="ml-1.5 rounded bg-mira-elev1/70 px-1.5 py-0.5 font-mono text-[12px] text-foreground">
                {summary.target}
              </span>
            ) : (
              <span className="ml-1.5 font-mono text-[12px] text-foreground/85">
                {summary.target}
              </span>
            )
          )}
          {stats && (
            // Always show both counts side by side ("+291 -0" reads
            // consistently with "+3 -2") — a fresh write with zero
            // deletions still gets a rose "-0" so the format doesn't
            // shift shape between rows.
            <span className="ml-2 font-mono text-[12px]">
              <span className="text-emerald-400">+{stats.adds}</span>
              {' '}
              <span className="text-rose-400">-{stats.dels}</span>
            </span>
          )}
        </span>
        <CaretDown
          weight="bold"
          className={cn(
            'size-3 shrink-0 text-foreground/70 transition-all',
            !expanded && '-rotate-90',
            // Hide when collapsed AND not hovered — mirrors the
            // "Worked for" header so the disclosure language is
            // consistent across the transcript.
            !expanded && 'opacity-0 group-hover:opacity-100',
          )}
        />
        <StatusMark status={status} />
      </button>

      {expanded && (
        <div className="ml-6 mb-1.5 mt-1 flex animate-fade-in flex-col gap-1.5">
          {isDiffCardTool && effectiveDiff && (
            <DiffCard
              filePath={filePath}
              tool={call.function.name}
              lines={effectiveDiff}
              stats={stats}
            />
          )}
          {!isDiffCardTool && preview && (
            <div className="diff compact">
              {preview.lines.slice(0, 20).map((l, i) => <DiffRow key={i} line={l} />)}
              {preview.lines.length > 20 && (
                <div className="diff-line hunk">… {preview.lines.length - 20} more lines</div>
              )}
            </div>
          )}
          {!isDiffCardTool && !preview && <ReconstructedPreview call={call} />}
          {/* Result block skipped for write/edit — the diff already
              shows what changed; "wrote N bytes" adds nothing. Any
              other tool (bash / grep / read_file / …) still surfaces
              its output because the output IS the point there. */}
          {result &&
            !(call.function.name in RESULT_SUPPRESSED) && (
              <ResultBlock content={result.content} isError={result.is_error ?? false} />
            )}
        </div>
      )}
    </div>
  );
}

/**
 * Codex-style diff card. Elevated background, its own header row
 * showing the file icon + short filename + full path (muted) + the
 * `+N -N` summary, and line numbers on the left of every diff row.
 *
 * Line numbers are sequential (1..N) rather than source-file
 * positions — the raw `DiffPreview` doesn't carry hunk metadata, and
 * a running counter matches the Codex look while giving the reader
 * a visual guide. When a full source line-number scheme lands
 * upstream (hunk headers or LSP positions), this component swaps in
 * the real numbers.
 */
function DiffCard({
  filePath, tool, lines, stats,
}: {
  filePath: string;
  tool: string;
  lines: DiffLine[];
  stats: { adds: number; dels: number } | null;
}) {
  const shortName = filePath ? filePath.split('/').pop() ?? filePath : tool;
  const dir = filePath && filePath.includes('/')
    ? filePath.slice(0, filePath.lastIndexOf('/'))
    : '';

  return (
    <div className="overflow-hidden rounded-lg border border-border/50 bg-mira-elev1/60">
      <div className="flex items-center gap-2 border-b border-border/40 bg-mira-elev1/80 px-3 py-2 text-[12px]">
        <FileText className="size-3.5 shrink-0 text-muted-foreground" weight="regular" />
        <span className="shrink-0 font-medium text-foreground">{shortName}</span>
        {dir && (
          <span className="min-w-0 flex-1 truncate text-muted-foreground/70">
            {dir}
          </span>
        )}
        {stats && (
          <span className="ml-auto shrink-0 font-mono text-[11.5px]">
            <span className="text-emerald-400">+{stats.adds}</span>
            {' '}
            <span className="text-rose-400">-{stats.dels}</span>
          </span>
        )}
      </div>
      <div className="max-h-[50vh] overflow-auto font-mono text-[12px] leading-relaxed">
        {(() => {
          const CAP = 200;
          const shown = lines.slice(0, CAP);
          let ln = 0;
          return (
            <>
              {shown.map((line, i) => {
                if (line.tag === 'hunkgap') {
                  return (
                    <div key={i} className="px-3 py-0.5 text-center text-muted-foreground/60">
                      ···
                    </div>
                  );
                }
                // Only count real lines toward the line-number
                // counter; deletions don't advance the "new file"
                // number in a real diff, but for the sequential
                // approximation we count adds + context only.
                if (line.tag !== 'del') ln += 1;
                const shownNum = line.tag === 'del' ? '' : String(ln);
                const bg =
                  line.tag === 'add'
                    ? 'bg-emerald-500/[0.08]'
                    : line.tag === 'del'
                      ? 'bg-rose-500/[0.08]'
                      : '';
                const text =
                  line.tag === 'add'
                    ? 'text-emerald-200/90'
                    : line.tag === 'del'
                      ? 'text-rose-200/85'
                      : 'text-foreground/85';
                return (
                  <div
                    key={i}
                    className={cn(
                      'flex items-start gap-2 whitespace-pre-wrap break-words',
                      bg,
                    )}
                  >
                    <span className="w-10 shrink-0 select-none border-r border-border/30 bg-mira-elev1/40 px-2 py-0.5 text-right text-muted-foreground/60">
                      {shownNum}
                    </span>
                    <span className={cn('flex-1 py-0.5 pr-2', text)}>
                      {line.text || ' '}
                    </span>
                  </div>
                );
              })}
              {lines.length > CAP && (
                <div className="border-t border-border/30 px-3 py-1.5 text-center text-[11px] text-muted-foreground/70">
                  … {lines.length - CAP} more lines
                </div>
              )}
            </>
          );
        })()}
      </div>
    </div>
  );
}

/** Count adds vs deletions in a DiffLine list — powers the `+N -N`
 *  chip in both the compact summary row and the diff card header. */
function countDiff(lines: DiffLine[]): { adds: number; dels: number } {
  let adds = 0;
  let dels = 0;
  for (const l of lines) {
    if (l.tag === 'add') adds += 1;
    else if (l.tag === 'del') dels += 1;
  }
  return { adds, dels };
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

  // Skip arg echo entirely when the row header already carries the
  // meaningful arg (glob pattern, grep pattern, read path, etc.).
  // The row above and the result below say everything we need; a
  // duplicated arg block just adds visual weight.
  if (tool in ARG_PREVIEW_REDUNDANT) {
    return null;
  }

  // Text-heavy single-field tools — memory writes, subagent prompts,
  // ask-user proposals, task subjects — get rendered as prose in a
  // quiet elevated block so a paragraph reads as a paragraph, not
  // as a JSON payload.
  const proseText = pickProseText(tool, args);
  if (proseText) {
    return (
      <div className="rounded-md border border-border/40 bg-mira-elev1/50 px-3 py-2 text-[12.5px] leading-relaxed text-foreground/85">
        <div className="whitespace-pre-wrap break-words">{proseText}</div>
      </div>
    );
  }

  // Nothing worth showing — args are either empty or too weird to
  // render nicely. Drop the block entirely rather than fall back to
  // raw JSON (which was the old failure mode).
  return null;
}

/** For tools whose meaningful arg is a single prose field, pull it
 *  out so we can render it as text instead of JSON. Order below
 *  matters — earlier branches win when multiple fields exist. */
function pickProseText(tool: string, args: any): string | null {
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

function CommandBlock({ command }: { command: string }) {
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

/**
 * Result of a tool call, rendered as a quiet elevated block that
 * matches the diff/prose blocks used elsewhere in the transcript.
 * No "RESULT" chrome bar — just the content with a subtle background
 * so it clearly belongs to the row above.
 *
 * Errors get a rose tint (not a full red card) so the transcript
 * still reads calmly during a run where a few tool calls fail;
 * "show all" toggles the tail past the first N lines.
 */
function ResultBlock({ content, isError }: { content: string; isError: boolean }) {
  const [showAll, setShowAll] = useState(false);
  const lines = content.split('\n');
  const CAP = 6;
  const overflow = lines.length > CAP;
  const shown = showAll ? content : lines.slice(0, CAP).join('\n');

  return (
    <div
      className={cn(
        'overflow-hidden rounded-md border bg-mira-elev1/50',
        isError ? 'border-rose-500/30' : 'border-border/40',
      )}
    >
      <pre
        className={cn(
          'm-0 max-h-[24vh] overflow-auto whitespace-pre-wrap break-words px-3 py-2 font-mono text-[11.5px] leading-relaxed',
          isError ? 'text-rose-200/90' : 'text-foreground/80',
        )}
      >
        {shown}
        {!showAll && overflow ? `\n… ${lines.length - CAP} more lines` : ''}
      </pre>
      {overflow && (
        <div className="flex items-center justify-end border-t border-border/30 bg-mira-elev1/70 px-2 py-1">
          <button
            className="rounded px-1.5 py-0.5 text-[11px] text-mira-blue transition-colors hover:bg-mira-blue/10"
            onClick={() => setShowAll((v) => !v)}
          >
            {showAll ? 'show less' : `show all (${lines.length} lines)`}
          </button>
        </div>
      )}
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
  find_symbol: true, find_references: true, find_callers: true, bash: true, rustfmt: true, web_fetch: true, web_search: true,
  task_create: true, task_update: true, task_list: true, task_get: true,
  git_diff: true, git_status: true, git_log: true, git_commit: true,
  memory_read: true, memory_search: true, memory_append: true, memory_edit: true,
  memory_remember: true,
  skill: true, ask_user: true, plan: true, agent: true,
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
    case 'find_references':
    case 'find_callers':
      return args?.name ? String(args.name) : args?.query ? String(args.query) : '';
    case 'task_create':
      return args?.subject ? String(args.subject) : '';
    case 'task_update':
      return args?.task_id ? `#${args.task_id}${args.status ? ` → ${args.status}` : ''}` : '';
    case 'task_get':
      return args?.task_id ? `#${args.task_id}` : '';
    case 'task_list':
      return '';
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
    case 'skill':
      // "Used <name> skill" reads naturally alongside "Ran <cmd>" and
      // "Read <path>" — the tool name would be redundant otherwise.
      return args?.name ? `${args.name} skill` : '';
    case 'agent':
      return args?.prompt ? shortCmd(String(args.prompt)) : '';
    case 'plan':
    case 'ask_user':
      return '';
    default:
      return '';
  }
}

function safeParse(s: string): any {
  try { return JSON.parse(s); } catch { return null; }
}

/** Tools whose target reads as a file path / identifier — earns the
 *  inline-code pill so `Read foo.rs` matches the Codex look. Bash
 *  commands, grep patterns, task numbers, etc. stay as plain mono
 *  text (a pill around a shell command would look weird). */
function isPathishTool(name: string): boolean {
  switch (name) {
    case 'read_file':
    case 'write_file':
    case 'edit_file':
    case 'rustfmt':
    case 'glob':
    case 'web_fetch':
    case 'memory_read':
    case 'memory_append':
    case 'memory_edit':
      return true;
    default:
      return false;
  }
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
