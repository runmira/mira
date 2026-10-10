import { cn } from '@/lib/utils';
import { AnimatePresence, m } from 'framer-motion';
import { Check, ChevronDown, LoaderCircle, X } from 'lucide-react';
import { useMemo, useState } from 'react';
import { imageSrc } from '../lib/images';
import { parseBashOutput } from '../lib/toolOutput.mjs';
import type { ApprovalScope, DiffLine, DiffPreview, Mode, ToolCall, ToolResult } from '../types';
import { ActivityShimmer } from './ActivityShimmer';
import { BranchElbow } from './BranchElbow';
import { InlineDiff } from './diffs/LazyDiffs';
import {
  isPathishTool,
  labelFor,
  prettyPrint,
  readableArgRows,
  safeParse,
  summarize,
} from './tools/arguments';
import {
  countDiff,
  DiffCard,
  DiffRow,
  reconstructDiff,
  ReconstructedPreview,
} from './tools/ToolDiff';
import { CommandBlock, LiveOutputPanel, ResultBlock } from './tools/ToolResult';
import type { ToolStatus } from './tools/types';
import { useTranscriptDisclosure } from './TranscriptDisclosure';
import { Tip } from './ui/Tip';
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

/** Tools that get the Codex-style diff card treatment (header row
 *  outside with `+N -N` summary, elevated card with its own header +
 *  line numbers). Any tool that produces a `DiffPreview` today —
 *  write, edit, create — falls in this bucket. */
const DIFF_CARD_TOOLS: Record<string, true> = {
  write_file: true,
  edit_file: true,
};

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
export function ToolCard({
  call,
  preview,
  status,
  result,
  onDecide,
  mode: _mode,
  onSetMode: _onSetMode,
  onOpenFile,
  progressLines,
  decisionsViaDialog,
  detailsOnly = false,
}: Props) {
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
  return (
    <CompactToolRow
      call={call}
      status={status}
      result={result}
      preview={preview}
      onOpenFile={onOpenFile}
      progressLines={progressLines}
      detailsOnly={detailsOnly}
    />
  );
}

/* ---------- pending approval ---------- */

function PendingApprovalCard({
  call,
  preview,
  onDecide,
  onOpenFile,
  decisionsViaDialog,
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
          {preview.lines.map((line, i) => (
            <DiffRow key={i} line={line} />
          ))}
        </div>
      ) : (
        <ReadableArgsBlock
          tool={call.function.name}
          argsText={call.function.arguments}
          fallback={prettyArgs}
        />
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
                className="rounded-full bg-fg/[0.04] ring-1 ring-fg/[0.08] px-4 py-1.5 text-[11.5px] font-medium text-muted-foreground transition-colors hover:bg-secondary/60 hover:text-foreground"
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
                  onClick={() => setScopeMenuOpen((v) => !v)}
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
                <m.div
                  initial={{ opacity: 0, y: -4, scale: 0.98 }}
                  animate={{ opacity: 1, y: 0, scale: 1, transition: { duration: 0.13 } }}
                  exit={{ opacity: 0, y: -4, scale: 0.98, transition: { duration: 0.1 } }}
                  className="absolute right-0 top-full z-20 mt-1 flex min-w-[220px] origin-top-right flex-col rounded-md border border-border/40 bg-popover p-1 shadow-lg"
                  onMouseLeave={() => setScopeMenuOpen(false)}
                >
                  <ScopeMenuItem
                    label="Allow for this session"
                    hint="Skip the prompt for this exact target until the process restarts."
                    onClick={() => {
                      setScopeMenuOpen(false);
                      onDecide(true, 'session');
                    }}
                  />
                  <ScopeMenuItem
                    label="Always allow"
                    hint="Also save the rule to ~/.mira/mira.yaml — persists across restarts."
                    onClick={() => {
                      setScopeMenuOpen(false);
                      onDecide(true, 'always');
                    }}
                  />
                </m.div>
              )}
            </AnimatePresence>
          </div>
          <div className="text-right text-[10.5px] text-muted-foreground/60">
            <kbd className="rounded bg-secondary/70 px-1 py-0.5 font-mono text-[10px]">y</kbd> allow
            · <kbd className="rounded bg-secondary/70 px-1 py-0.5 font-mono text-[10px]">n</kbd>{' '}
            deny
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
  call,
  status,
  result,
  preview,
  onOpenFile,
  progressLines,
  detailsOnly = false,
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
  const stats = useMemo(() => (effectiveDiff ? countDiff(effectiveDiff) : null), [effectiveDiff]);
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
  const bashOutput =
    call.function.name === 'bash' && result ? parseBashOutput(result.content) : null;
  const commandFailed =
    completedCommand &&
    (result?.is_error === true || (bashOutput?.exitCode != null && bashOutput.exitCode !== 0));
  const bashCommand = useMemo(() => {
    if (call.function.name !== 'bash') return undefined;
    const args = safeParse(call.function.arguments);
    return typeof args?.command === 'string' ? args.command : undefined;
  }, [call.function.name, call.function.arguments]);

  return (
    <div className="w-full max-w-[78%]">
      {/* Use div+role instead of <button> so the filename chip (also a
          button) doesn't trigger the "nested interactive content" HTML
          violation that causes browsers to hoist or swallow the inner click. */}
      {!detailsOnly && (
        <div
          role="button"
          tabIndex={0}
          onClick={() => setExpanded((v) => !v)}
          onKeyDown={(e) => {
            if (e.key === 'Enter' || e.key === ' ') setExpanded((v) => !v);
          }}
          className={cn(
            'group relative flex w-full min-w-0 cursor-pointer items-center gap-1.5 rounded-md px-1 py-0.5 text-left text-[13px] transition-colors hover:bg-accent/40',
            completedCommand && 'pl-4',
            commandFailed && 'rounded-md border border-rose-500/30 bg-rose-500/5 text-rose-200',
          )}
        >
          {completedCommand && <BranchElbow className="left-0 top-0" />}
          <span
            aria-hidden="true"
            className="inline-flex size-3.5 shrink-0 items-center justify-center text-muted-foreground"
          >
            {summary.icon}
          </span>
          {/* The verb and the +/- stats never shrink; the target does. A
            single truncating span cut the whole line to "Edited…" in a
            narrow transcript, hiding the file chip but leaving it
            clickable. */}
          <span className="flex min-w-0 flex-1 items-center">
            <ActivityShimmer active={status === 'running'} className="shrink-0">
              {summary.verb}
            </ActivityShimmer>
            {summary.target &&
              (isDiffCardTool && onOpenFile ? (
                // Filename chip: click opens the file panel (with diff if available).
                <Tip label="Open in file viewer" className="ml-1.5 min-w-0">
                  <button
                    type="button"
                    onClick={(e) => {
                      e.stopPropagation();
                      onOpenFile(filePath, openDiff);
                    }}
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
              ))}
            {stats && (
              <span className="ml-2 shrink-0 font-mono text-[12px]">
                <span className="text-emerald-400">+{stats.adds}</span>{' '}
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
          <StatusMark status={status} failed={commandFailed} />
        </div>
      )}

      {/* Live output panel — visible whenever the tool has streamed lines,
          even before or after the result lands. Shows a compact tail by
          default; expands to the full buffer when the row is open. */}
      {progressLines && progressLines.length > 0 && (
        <LiveOutputPanel lines={progressLines} running={status === 'running'} expanded={expanded} />
      )}

      <AnimatePresence initial={false}>
        {expanded && (
          <m.div
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
            {result && !(call.function.name in RESULT_SUPPRESSED) && (
              <ResultBlock
                stateKey={call.id}
                content={bashOutput?.output ?? result.content}
                isError={result.is_error ?? false}
                exitCode={bashOutput?.exitCode}
                timedOut={bashOutput?.timedOut}
                command={bashCommand}
              />
            )}
            {result?.images?.map((img, i) => (
              <img
                key={i}
                className="tool-screenshot"
                src={imageSrc(img)}
                loading="lazy"
                alt="Screenshot returned by the tool"
                style={{ maxWidth: '100%', borderRadius: 6, marginTop: 8, display: 'block' }}
              />
            ))}
          </m.div>
        )}
      </AnimatePresence>
    </div>
  );
}

/* ---------- bits ---------- */

function StatusMark({ status, failed = false }: { status: ToolStatus; failed?: boolean }) {
  switch (status) {
    case 'running':
      return <LoaderCircle className="size-3 shrink-0 animate-spin text-mira-blue" />;
    case 'denied':
      return <X className="size-3.5 shrink-0 text-destructive" />;
    case 'complete':
      return failed ? (
        <X className="size-3.5 shrink-0 text-rose-400" />
      ) : (
        <Check className="size-3.5 shrink-0 text-emerald-500" />
      );
    default:
      return null;
  }
}

function Pill({ children }: { children: React.ReactNode }) {
  return (
    <span className="rounded-full border border-border bg-background px-1.5 py-0.5 text-[10.5px] uppercase tracking-wider text-muted-foreground">
      {children}
    </span>
  );
}

export type { ToolStatus } from './tools/types';
