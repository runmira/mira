import { BranchElbow } from './BranchElbow';
import { ActivityShimmer } from './ActivityShimmer';
import { useTranscriptDisclosure } from './TranscriptDisclosure';
import { useMemo, useState } from 'react';
import { AnimatePresence, motion } from 'framer-motion';
import { Tip } from './ui/Tip';
import {
  ChevronDown,
  Check,
  LoaderCircle,
  FileText,
  X,
} from 'lucide-react';
import type { ApprovalScope, DiffLine, DiffPreview, Mode, ToolCall, ToolResult } from '../types';
import { InlineDiff } from './diffs/InlineDiff';
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
  /** Approve/deny with an optional scope. `once` (or omitted) matches
   *  the legacy single-shot behavior; `session` also adds the exact
   *  target to the in-memory allowlist; `always` also persists it to
   *  `~/.mira/mira.yaml`. Denies are always single-shot. */
  onDecide: (allow: boolean, scope?: ApprovalScope) => void;
  /** Current session mode. Legacy: the card used to offer an
   *  "Always allow" that swaps the whole session to `edit` mode. That
   *  option is now hidden — scope-widening on the approve buttons
   *  covers the case without a blanket mode change. Retained on the
   *  type for backward-compat with call sites (e.g. subagent panel
   *  replay) that already pass it. */
  mode?: Mode;
  onSetMode?: (m: Mode) => void;
  /** Open the file in the side-panel file viewer. When present, file
   *  names in write/edit tool rows become clickable links. */
  onOpenFile?: (path: string, diff: DiffPreview | null) => void;
  /** Live output lines streamed via `tool_progress` frames, used by
   *  `run_background` and other long-running tools. */
  progressLines?: string[];
  /** When an external agent drives the session, decisions happen in the
   *  unified approval dialog — not here. The card keeps its preview and an
   *  "awaiting approval" state, but renders no buttons, so there is exactly
   *  one place to answer. */
  decisionsViaDialog?: boolean;
  /** A singleton command group already owns the disclosure header. */
  detailsOnly?: boolean;
};

/** Two shapes:
 *  - `pending`   → preview card with inline Allow / Deny (+ Always allow
 *                  when the current mode still gates). Y/N keyboard
 *                  shortcut is bound at the App level so it fires no
 *                  matter which card is on screen.
 *  - anything else → compact row, click to expand args + result. */
export function ToolCard({ call, preview, status, result, onDecide, mode: _mode, onSetMode: _onSetMode, onOpenFile, progressLines, decisionsViaDialog, detailsOnly = false }: Props) {
  if (status === 'pending') {
    return (
      <PendingApprovalCard
        call={call}
        preview={preview}
        onDecide={onDecide}
        onOpenFile={onOpenFile}
        decisionsViaDialog={decisionsViaDialog}
      />
    );
  }
  return <CompactToolRow call={call} status={status} result={result} preview={preview} onOpenFile={onOpenFile} progressLines={progressLines} detailsOnly={detailsOnly} />;
}

/* ---------- pending approval ---------- */

function PendingApprovalCard({
  call, preview, onDecide, onOpenFile, decisionsViaDialog,
}: {
  call: ToolCall;
  preview: DiffPreview | null;
  onDecide: (allow: boolean, scope?: ApprovalScope) => void;
  onOpenFile?: (path: string, diff: DiffPreview | null) => void;
  decisionsViaDialog?: boolean;
  /** A singleton command group already owns the disclosure header. */
  detailsOnly?: boolean;
}) {
  // Pending = about to run → use the present-continuous verb ("Reading",
  // "Running", "Editing") so the header reads as a proposal, not a receipt.
  const summary = useMemo(() => summarize(call, 'pending'), [call]);
  const prettyArgs = useMemo(() => prettyPrint(call.function.arguments), [call.function.arguments]);
  const kindLabel = preview ? labelFor(preview.kind) : null;
  const [scopeMenuOpen, setScopeMenuOpen] = useState(false);
  const isDiffTool = call.function.name in DIFF_CARD_TOOLS;

  return (
    <div className="flex w-full max-w-[78%] flex-col gap-2 rounded-2xl border border-border/40 elev-card dark:bg-card/80 dark:backdrop-blur p-3.5">
      <div className="flex items-center gap-2 font-mono text-[12.5px]">
        <span className="text-mira-tool">{summary.icon}</span>
        <span className="font-medium text-foreground">
          {summary.verb}{' '}
          {isDiffTool && onOpenFile && preview?.path ? (
            <button
              type="button"
              onClick={() => onOpenFile(preview.path, preview)}
              className="font-normal text-muted-foreground underline-offset-2 transition-colors hover:text-mira-blue hover:underline"
            >
              {summary.target}
            </button>
          ) : (
            <span className="font-normal text-muted-foreground">{summary.target}</span>
          )}
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
        <ReadableArgsBlock tool={call.function.name} argsText={call.function.arguments} fallback={prettyArgs} />
      )}

      {/* Buttons: Deny — Allow (primary) — caret opens a scope menu.
       *  Primary click is `allow: true, scope: once` (same as before).
       *  The caret exposes the widening scopes without cluttering the
       *  default action. Y/N keyboard shortcut still fires the default
       *  Allow/Deny; scope choice is mouse-only for now.
       *
       *  Suppressed entirely when an agent drives the session: the unified
       *  approval dialog owns the decision, and a second set of buttons here
       *  would be a second dialog wearing a card's clothes. */}
      {decisionsViaDialog ? (
        <div className="px-1.5 pb-1 pt-0 text-right text-[10.5px] text-muted-foreground/60">
          Decide in the approval dialog above.
        </div>
      ) : (
      <>
      <div className="flex items-center gap-1.5 pt-1 relative">
        <Tip label="Deny" shortcut="N" className="ml-auto">
          <button
            type="button"
            onClick={() => onDecide(false)}
            className="rounded-md px-2.5 py-1.5 text-[11.5px] font-medium text-muted-foreground transition-colors hover:bg-secondary/60 hover:text-foreground"
          >
            Deny
          </button>
        </Tip>
        <div className="inline-flex rounded-full bg-foreground">
          <Tip label="Allow this one call" shortcut="Y">
            <button
              type="button"
              onClick={() => onDecide(true, 'once')}
              className="rounded-l-full px-3.5 py-1.5 text-[11.5px] font-semibold text-background transition-all hover:brightness-95"
            >
              Allow
            </button>
          </Tip>
          <Tip label="More options" hint="Allow for this chat, or always" align="end">
            <button
              type="button"
              onClick={() => setScopeMenuOpen(v => !v)}
              aria-expanded={scopeMenuOpen}
              aria-label="More approve options"
              className="rounded-r-full border-l border-background/25 px-2 py-1.5 text-background transition-all hover:brightness-95"
            >
              <ChevronDown size={12} strokeWidth={2.5} />
            </button>
          </Tip>
        </div>
        <AnimatePresence initial={false}>
        {scopeMenuOpen && (
          <motion.div
            initial={{ opacity: 0, y: -4, scale: 0.98 }}
            animate={{ opacity: 1, y: 0, scale: 1, transition: { duration: 0.13 } }}
            exit={{ opacity: 0, y: -4, scale: 0.98, transition: { duration: 0.1 } }}
            className="absolute right-0 top-full z-20 mt-1 flex min-w-[220px] origin-top-right flex-col rounded-md border border-border/40 bg-popover p-1 shadow-lg"
            onMouseLeave={() => setScopeMenuOpen(false)}
          >
            <ScopeMenuItem
              label="Allow for this session"
              hint="Skip the prompt for this exact target until the process restarts."
              onClick={() => { setScopeMenuOpen(false); onDecide(true, 'session'); }}
            />
            <ScopeMenuItem
              label="Always allow"
              hint="Also save the rule to ~/.mira/mira.yaml — persists across restarts."
              onClick={() => { setScopeMenuOpen(false); onDecide(true, 'always'); }}
            />
          </motion.div>
        )}
        </AnimatePresence>
      </div>
      <div className="text-right text-[10.5px] text-muted-foreground/60">
        <kbd className="rounded bg-secondary/70 px-1 py-0.5 font-mono text-[10px]">y</kbd> allow · <kbd className="rounded bg-secondary/70 px-1 py-0.5 font-mono text-[10px]">n</kbd> deny
      </div>
      </>
      )}
    </div>
  );
}


function ReadableArgsBlock({
  tool,
  argsText,
  fallback,
  maxHeight = '22vh',
}: {
  tool: string;
  argsText: string;
  fallback?: string;
  maxHeight?: string;
}) {
  const args = safeParse(argsText);
  if (tool === 'bash' && typeof args?.command === 'string') {
    return <CommandBlock command={String(args.command)} />;
  }

  const rows = readableArgRows(tool, args);
  if (rows.length > 0) {
    return (
      <div className="rounded-md border border-border/35 bg-mira-elev1/45 px-3 py-2 text-[12.5px]">
        <dl className="grid grid-cols-[auto_minmax(0,1fr)] gap-x-3 gap-y-1.5">
          {rows.map((row) => (
            <div key={row.label} className="contents">
              <dt className="select-none text-muted-foreground/65">{row.label}</dt>
              <dd className="min-w-0 break-words font-mono text-foreground/85">{row.value}</dd>
            </div>
          ))}
        </dl>
      </div>
    );
  }

  const text = fallback ?? prettyPrint(argsText);
  if (!text.trim() || text.trim() === '{}') return null;
  return (
    <pre
      className="m-0 overflow-auto whitespace-pre-wrap rounded-md bg-background/55 px-3 py-2 font-mono text-xs text-muted-foreground"
      style={{ maxHeight }}
    >
      {text}
    </pre>
  );
}

function readableArgRows(tool: string, args: any): { label: string; value: string }[] {
  if (!args || typeof args !== 'object') return [];
  const rows: { label: string; value: string }[] = [];
  const add = (label: string, value: unknown) => {
    if (typeof value !== 'string' && typeof value !== 'number' && typeof value !== 'boolean') return;
    const text = String(value).trim();
    if (!text) return;
    rows.push({ label, value: text });
  };
  switch (tool) {
    case 'read_file':
    case 'write_file':
    case 'edit_file':
    case 'rustfmt':
      add('file', args.path ?? args.file_path);
      if (tool === 'write_file' && typeof args.content === 'string') add('content', summarizeText(args.content));
      return rows;
    case 'web_fetch':
      add('url', args.url);
      return rows;
    case 'web_search':
      add('query', args.query);
      return rows;
    case 'grep':
      add('pattern', args.pattern);
      add('path', args.path);
      add('glob', args.glob);
      return rows;
    case 'glob':
      add('pattern', args.pattern);
      add('path', args.path);
      return rows;
    default:
      add('target', args.path ?? args.file_path ?? args.url ?? args.query ?? args.pattern);
      return rows;
  }
}

function summarizeText(text: string): string {
  const one = text.replace(/\s+/g, ' ').trim();
  return one.length > 120 ? `${one.slice(0, 120)}…` : one;
}

function ScopeMenuItem({
  label,
  hint,
  onClick,
}: {
  label: string;
  hint: string;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      className="flex flex-col items-start gap-0.5 rounded-sm px-2.5 py-1.5 text-left text-[12px] transition-colors hover:bg-secondary/70"
    >
      <span className="font-medium text-foreground">{label}</span>
      <span className="text-[10.5px] text-muted-foreground">{hint}</span>
    </button>
  );
}

/* ---------- compact row (running / complete / denied) ---------- */

function CompactToolRow({
  call, status, result, preview, onOpenFile, progressLines, detailsOnly = false,
}: {
  call: ToolCall;
  status: ToolStatus;
  result: ToolResult | null;
  preview: DiffPreview | null;
  onOpenFile?: (path: string, diff: DiffPreview | null) => void;
  progressLines?: string[];
  detailsOnly?: boolean;
}) {
  const [expanded, setExpanded] = useTranscriptDisclosure(`tool:${call.id}`, detailsOnly);
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
  // What opening the file shows. The server's live preview when there is
  // one; otherwise the diff rebuilt from the call's own arguments. Passing
  // only the live preview opened agent edits and reloaded sessions with no
  // diff at all, even though the row itself showed one.
  const openDiff: DiffPreview | null = useMemo(() => {
    if (preview) return preview;
    if (!effectiveDiff || !filePath) return null;
    return {
      path: filePath,
      kind: call.function.name === 'write_file' ? 'create' : 'edit',
      lines: effectiveDiff,
    };
  }, [preview, effectiveDiff, filePath, call.function.name]);

  // Filename-ish targets get a subtle inline-code pill so the "verb
  // target" line reads as "prefix + identifier" the way Codex renders
  // ("Read find_symbol.rs"). Non-path targets (bash commands, grep
  // patterns) stay as plain mono text — a pill around a shell
  // command would look strange.
  const targetIsPath = isPathishTool(call.function.name);
  const completedCommand = call.function.name === 'bash' && status === 'complete';

  return (
    <div className="w-full max-w-[78%]">
      {/* Use div+role instead of <button> so the filename chip (also a
          button) doesn't trigger the "nested interactive content" HTML
          violation that causes browsers to hoist or swallow the inner click. */}
      {!detailsOnly && <div
        role="button"
        tabIndex={0}
        onClick={() => setExpanded((v) => !v)}
        onKeyDown={(e) => { if (e.key === 'Enter' || e.key === ' ') setExpanded((v) => !v); }}
        className={cn("group relative flex w-full min-w-0 cursor-pointer items-center gap-1.5 rounded-md px-1 py-0.5 text-left text-[13px] transition-colors hover:bg-accent/40", completedCommand && "pl-4")}
      >
        {completedCommand && <BranchElbow className="left-0 top-0" />}
        <span aria-hidden="true" className="inline-flex size-3.5 shrink-0 items-center justify-center text-muted-foreground">{summary.icon}</span>
        {/* The verb and the +/- stats never shrink; the target does. A
            single truncating span cut the whole line to "Edited…" in a
            narrow transcript, hiding the file chip but leaving it
            clickable. */}
        <span className="flex min-w-0 flex-1 items-center">
          <ActivityShimmer active={status === 'running'} className="shrink-0">{summary.verb}</ActivityShimmer>
          {summary.target && (
            isDiffCardTool && onOpenFile ? (
              // Filename chip: click opens the file panel (with diff if available).
              <Tip label="Open in file viewer" className="ml-1.5 min-w-0">
                <button
                  type="button"
                  onClick={(e) => { e.stopPropagation(); onOpenFile(filePath, openDiff); }}
                  className="min-w-0 truncate rounded bg-mira-elev1/70 px-1.5 py-0.5 font-mono text-[12px] text-foreground transition-colors hover:bg-mira-blue/15 hover:text-mira-blue"
                >
                  {summary.target}
                </button>
              </Tip>
            ) : targetIsPath ? (
              <span className="ml-1.5 min-w-0 truncate rounded bg-mira-elev1/70 px-1.5 py-0.5 font-mono text-[12px] text-foreground">
                {summary.target}
              </span>
            ) : (
              <span className="ml-1.5 min-w-0 truncate font-mono text-[12px] text-foreground/85">
                {summary.target}
              </span>
            )
          )}
          {stats && (
            <span className="ml-2 shrink-0 font-mono text-[12px]">
              <span className="text-emerald-400">+{stats.adds}</span>
              {' '}
              <span className="text-rose-400">-{stats.dels}</span>
            </span>
          )}
        </span>
        <ChevronDown
          strokeWidth={2.5}
          className={cn(
            'size-3 shrink-0 text-foreground/70 transition-all',
            !expanded && '-rotate-90',
            !expanded && 'opacity-0 group-hover:opacity-100',
          )}
        />
        <StatusMark status={status} />
      </div>}

      {/* Live output panel — visible whenever the tool has streamed lines,
          even before or after the result lands. Shows a compact tail by
          default; expands to the full buffer when the row is open. */}
      {progressLines && progressLines.length > 0 && (
        <LiveOutputPanel
          lines={progressLines}
          running={status === 'running'}
          expanded={expanded}
        />
      )}

      <AnimatePresence initial={false}>
        {expanded && (
        <motion.div
          initial={{ height: 0, opacity: 0 }}
          animate={{ height: 'auto', opacity: 1, transition: { duration: 0.16 } }}
          exit={{ height: 0, opacity: 0, transition: { duration: 0.12 } }}
          className="ml-6 mb-1.5 mt-1 flex flex-col gap-1.5 overflow-hidden"
        >
          {isDiffCardTool && effectiveDiff && (
            <DiffCard
              filePath={filePath}
              tool={call.function.name}
              lines={effectiveDiff}
              stats={stats}
              diffPreview={openDiff}
              onOpenFile={onOpenFile}
            />
          )}
          {!isDiffCardTool && preview && (
            <InlineDiff
              path={filePath || call.function.name}
              lines={preview.lines}
              maxHeight="20rem"
              className="rounded-lg border border-border/50"
            />
          )}
          {!isDiffCardTool && !preview && <ReconstructedPreview call={call} />}
          {/* Result block skipped for write/edit — the diff already
              shows what changed; "wrote N bytes" adds nothing. Any
              other tool (bash / grep / read_file / …) still surfaces
              its output because the output IS the point there. */}
          {result &&
            !(call.function.name in RESULT_SUPPRESSED) && (
              <ResultBlock stateKey={call.id} content={result.content} isError={result.is_error ?? false} live={status === 'running'} />
            )}
          {result?.images?.map((img, i) => (
            <img
              key={i}
              className="tool-screenshot"
              src={`data:${img.media_type};base64,${img.data}`}
              alt="Screenshot returned by the tool"
              style={{ maxWidth: '100%', borderRadius: 6, marginTop: 8, display: 'block' }}
            />
          ))}
        </motion.div>
        )}
      </AnimatePresence>
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
  filePath, tool, lines, stats, diffPreview, onOpenFile,
}: {
  filePath: string;
  tool: string;
  lines: DiffLine[];
  stats: { adds: number; dels: number } | null;
  diffPreview: DiffPreview | null;
  onOpenFile?: (path: string, diff: DiffPreview | null) => void;
}) {
  const shortName = filePath ? filePath.split('/').pop() ?? filePath : tool;
  const dir = filePath && filePath.includes('/')
    ? filePath.slice(0, filePath.lastIndexOf('/'))
    : '';

  return (
    <div className="overflow-hidden rounded-lg border border-border/50 bg-mira-elev1/60">
      <div className="flex items-center gap-2 border-b border-border/40 bg-mira-elev1/80 px-3 py-2 text-[12px]">
        <FileText className="size-3.5 shrink-0 text-muted-foreground" />
        {onOpenFile && filePath ? (
          <Tip label="Open in file viewer" className="shrink-0">
            <button
              type="button"
              onClick={() => onOpenFile(filePath, diffPreview)}
              className="font-medium text-foreground transition-colors hover:text-mira-blue hover:underline underline-offset-2"
            >
              {shortName}
            </button>
          </Tip>
        ) : (
          <span className="shrink-0 font-medium text-foreground">{shortName}</span>
        )}
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
      <InlineDiff path={filePath || tool} lines={lines} />
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
    const path = typeof args?.path === 'string' ? args.path : typeof args?.file_path === 'string' ? args.file_path : tool;
    return (
      <InlineDiff path={path} lines={pseudo} maxHeight="20rem" className="rounded-lg border border-border/50" />
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
function ResultBlock({ content, isError, live = false, stateKey }: { content: string; isError: boolean; live?: boolean; stateKey: string }) {
  const [showAll, setShowAll] = useTranscriptDisclosure(`output:${stateKey}`);
  const lines = content.split('\n');
  const CAP = 6;
  const overflow = lines.length > CAP;
  const shown = showAll ? content : (live ? lines.slice(-CAP) : lines.slice(0, CAP)).join('\n');

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
        {!showAll && overflow && live ? `… ${lines.length - CAP} earlier lines\n` : ''}
        {shown}
        {!showAll && overflow && !live ? `\n… ${lines.length - CAP} more lines` : ''}
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
    case 'running':  return <LoaderCircle className="size-3 shrink-0 animate-spin text-mira-blue" />;
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
  const icon = <Icon aria-hidden="true" className="size-3.5 shrink-0" />;

  const args = safeParse(call.function.arguments);
  const target = pickTarget(call.function.name, args);
  return { verb, target, icon };
}

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
    case 'delegate':
      return args?.query ? shortCmd(String(args.query)) : '';
    case 'browser':
      return args?.url ? String(args.url) : args?.action ? String(args.action) : '';
    case 'tool_search':
      return quote(args?.query);
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

/** Scrollable terminal-style panel for live process output.
 *
 * Compact mode (row collapsed): shows the last 5 lines with a subtle
 * dark terminal background so a running process doesn't look hung.
 * Expanded mode (row open): full buffer in a taller scrollable block.
 *
 * A pulsing green dot indicates the process is still running; a static
 * grey dot means it has exited. */
function LiveOutputPanel({
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
          <span className="ml-auto text-[10px] text-muted-foreground/40">
            {totalLines} lines
          </span>
        )}
      </div>
      {/* Output lines */}
      <div className="px-3 py-1.5">
        {visibleLines.map((line, i) => (
          <div
            key={i}
            className="whitespace-pre-wrap break-all text-[#24292f] dark:text-[#abb2bf]"
          >
            {line || <span className="text-transparent">{'.'}</span>}
          </div>
        ))}
      </div>
    </div>
  );
}
