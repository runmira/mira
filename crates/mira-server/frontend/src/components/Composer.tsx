import { useEffect, useMemo, useState } from 'react';
import {
  ArrowUp,
  Camera,
  CaretDown,
  Circle,
  CircleNotch,
  File as FileIcon,
  Folder,
  GitBranch,
  IconContext,
  Lightbulb,
  Link,
  Paperclip,
  Plus,
  Square,
  Target,
  X,
} from '@phosphor-icons/react';
import { createWorktree, getGitStatus, listModels, putCwd, readFile, type GitStatusView, type ModelInfo } from '../api';
import type { Goal, Mode } from '../types';
import { Popover, PopoverContent, PopoverTrigger } from '@/components/ui/popover';
import { Command, CommandEmpty, CommandGroup, CommandInput, CommandItem, CommandList, CommandSeparator } from '@/components/ui/command';
import { FilePicker } from './FilePicker';
import { filterCommands, slashState, type SlashCommand } from './commands';
import { cn } from '@/lib/utils';

const MODES: { value: Mode; label: string; desc: string }[] = [
  { value: 'plan',   label: 'Plan only',       desc: 'Reads and searches. No edits, no commands.' },
  { value: 'manual', label: 'Ask each time',   desc: 'Prompt before every edit or command.' },
  { value: 'auto',   label: 'Auto edits',      desc: 'Auto-approve edits. Prompt on commands.' },
  { value: 'edit',   label: 'Auto everything', desc: 'Edits + commands run unless a rule blocks.' },
  { value: 'yolo',   label: 'Yolo',            desc: 'No gating at all.' },
];

type Props = {
  disabled: boolean;
  busy: boolean;
  mode: Mode;
  model: string;
  cwd: string;
  /** Pre-formatted usage string (`↑12.3k ↓4.1k · $0.024`) or null when there's
   *  nothing to show yet. Formatting owned by App so per-model pricing lives
   *  in one place. */
  usage: string | null;
  onSend: (text: string) => void;
  onSetMode: (m: Mode) => void;
  onSetModel: (m: string) => void;
  /** Reasoning-effort setter. Pass `null` (or "off") to disable. */
  onSetEffort: (e: string | null) => void;
  onOpenPicker: () => void;
  onInterrupt: () => void;
  onNewChat: () => void;
  onOpenSettings: () => void;
  onRunReview: (args: string) => void;
  /** Kick off an autonomous run. `maxIterations` is optional; server
   *  defaults to `mira_harness::DEFAULT_MAX_ITERATIONS` when omitted. */
  onSetGoal: (condition: string, maxIterations?: number) => void;
  /** Drop the standing goal (if any). */
  onClearGoal: () => void;
  /** Session's standing `/goal`, if any. Renders a purple chip at the
   *  top of the composer while active — mirrors the Plan chip pattern
   *  so users know autonomy is on. */
  goal: Goal | null;
  onRemember: (scope: 'user' | 'project', text: string) => Promise<string>;
  onUndo: (count: number) => Promise<string>;
};

type Attachment = { path: string; content: string; bytes: number };

export function Composer({
  disabled, busy, mode, model, cwd, usage,
  onSend, onSetMode, onSetModel, onSetEffort, onOpenPicker, onInterrupt, onNewChat, onOpenSettings, onRunReview, onSetGoal, onClearGoal, goal, onRemember, onUndo,
}: Props) {
  const [text, setText] = useState('');
  const [attachments, setAttachments] = useState<Attachment[]>([]);
  const [attachError, setAttachError] = useState<string | null>(null);
  const [attachLoading, setAttachLoading] = useState(false);
  const [filePickerOpen, setFilePickerOpen] = useState(false);
  const [slashIdx, setSlashIdx] = useState(0);
  const [slashFeedback, setSlashFeedback] = useState<string | null>(null);
  const [modelPopOpen, setModelPopOpen] = useState(false);
  // "Goal compose" — flipped on by `/goal` (bare). While on, the next
  // Enter fires `onSetGoal(text)` instead of `onSend`. Chip stays until
  // the user either submits the condition or clicks it to abort.
  const [goalComposing, setGoalComposing] = useState(false);
  // Remember the mode we were on before entering plan mode, so toggling the
  // Plan chip off returns you to that mode instead of hardcoding `manual`.
  const [priorMode, setPriorMode] = useState<Mode>(mode === 'plan' ? 'manual' : mode);
  useEffect(() => {
    if (mode !== 'plan') setPriorMode(mode);
  }, [mode]);
  const planActive = mode === 'plan';
  function togglePlan() {
    onSetMode(planActive ? priorMode : 'plan');
  }

  const slash = slashState(text);
  const paletteVisible = slash.mode === 'palette';
  const paletteMatches = useMemo(
    () => (paletteVisible ? filterCommands((slash as { query: string }).query) : []),
    [paletteVisible, slash],
  );

  useEffect(() => { setSlashIdx(0); }, [text]);

  const ctx = useMemo(
    () => ({
      onNewChat,
      onSetMode,
      onSetModel,
      onOpenModelPicker: () => setModelPopOpen(true),
      onOpenFolderPicker: onOpenPicker,
      onOpenAttachPicker: () => setFilePickerOpen(true),
      onOpenSettings,
      onRunReview,
      onSetGoal,
      onClearGoal,
      onEnterGoalCompose: () => setGoalComposing(true),
      onRemember,
      onUndo,
    }),
    [onNewChat, onSetMode, onSetModel, onOpenPicker, onOpenSettings, onRunReview, onSetGoal, onClearGoal, onRemember, onUndo],
  );

  function executeCommand(cmd: SlashCommand, args: string) {
    setSlashFeedback(null);
    try {
      const template = cmd.run(args, ctx);
      if (template === undefined) {
        // Control command — clear the composer.
        setText('');
      } else {
        // Template — replace composer text so the user can review + send.
        setText(template);
      }
    } catch (e) {
      // Control command threw a usage/validation error. Keep the text so
      // the user can fix it inline, and surface the error under the
      // composer as an amber hint.
      setSlashFeedback(e instanceof Error ? e.message : String(e));
    }
  }

  function commitPaletteChoice(cmd: SlashCommand) {
    if (cmd.takesArgs) {
      // Prefill `/<name> ` so the user starts typing arguments; palette
      // auto-hides because there's now a space in the text.
      setText(`/${cmd.name} `);
    } else {
      executeCommand(cmd, '');
    }
  }

  async function attachFile(path: string) {
    setAttachError(null);
    setAttachLoading(true);
    try {
      const f = await readFile(path);
      // De-dupe by path — re-attaching the same file just refreshes it.
      setAttachments((prev) => [
        ...prev.filter((a) => a.path !== f.path),
        { path: f.path, content: f.content, bytes: f.bytes },
      ]);
    } catch (e) {
      setAttachError(String((e as Error).message));
    } finally {
      setAttachLoading(false);
    }
  }

  function removeAttachment(path: string) {
    setAttachments((prev) => prev.filter((a) => a.path !== path));
  }

  function submit() {
    const trimmed = text.trim();
    if ((!trimmed && attachments.length === 0) || disabled || busy) return;

    // If the user typed a full `/foo bar` command and hit Enter, execute
    // the command instead of sending it as a chat message. Control
    // commands vanish; template commands fill the composer so the user
    // can review — Enter again to actually send.
    if (slash.mode === 'args') {
      executeCommand(slash.command, slash.args);
      return;
    }

    // Goal-compose mode: submit the text as the goal condition instead
    // of a chat message. Clears the compose flag so the next Enter goes
    // back to normal sends.
    if (goalComposing) {
      onSetGoal(trimmed);
      setGoalComposing(false);
      setText('');
      setSlashFeedback(null);
      return;
    }

    const body = attachments.length > 0 ? renderAttachments(attachments, cwd) + '\n\n' + trimmed : trimmed;
    onSend(body);
    setText('');
    setAttachments([]);
    setAttachError(null);
    setSlashFeedback(null);
  }

  const modeLabel = MODES.find((m) => m.value === mode)?.label ?? mode;

  return (
    // Nested IconContext: icons rendered inside the composer bar use the
    // `fill` weight for a chunkier, more Codex-like look. Rest of the app
    // stays on the root duotone default.
    <IconContext.Provider value={{ weight: 'fill', size: '1em', mirrored: false }}>
    <div className="flex flex-col items-center gap-1.5 px-4 pb-4 pt-2">
      <form
        className="w-full max-w-3xl flex flex-col gap-1.5 rounded-[22px] border border-border bg-secondary/60 p-2.5"
        onSubmit={(e) => { e.preventDefault(); submit(); }}
      >
        {(planActive || goal || goalComposing) && (
          <div className="flex flex-wrap items-center gap-1.5 px-1.5 pt-0.5">
            {planActive && <PlanChip onExit={togglePlan} />}
            {goalComposing && !goal && (
              <GoalComposeChip onCancel={() => setGoalComposing(false)} />
            )}
            {goal && <GoalChip goal={goal} onClear={onClearGoal} />}
          </div>
        )}

        {(attachments.length > 0 || attachError) && (
          <div className="flex flex-wrap gap-1.5 px-1.5">
            {attachments.map((a) => (
              <AttachmentChip key={a.path} attachment={a} onRemove={() => removeAttachment(a.path)} cwd={cwd} />
            ))}
            {attachError && (
              <span className="rounded-md border border-destructive/40 bg-destructive/10 px-2 py-1 text-[11.5px] text-destructive">
                {attachError}
              </span>
            )}
          </div>
        )}

        <div className="relative">
          <textarea
            value={text}
            onChange={(e) => { setText(e.target.value); setSlashFeedback(null); }}
            onKeyDown={(e) => {
              // Palette is open → arrows navigate, Enter picks, Esc closes.
              if (paletteVisible && paletteMatches.length > 0) {
                if (e.key === 'ArrowDown') {
                  e.preventDefault();
                  setSlashIdx((i) => Math.min(i + 1, paletteMatches.length - 1));
                  return;
                }
                if (e.key === 'ArrowUp') {
                  e.preventDefault();
                  setSlashIdx((i) => Math.max(i - 1, 0));
                  return;
                }
                if (e.key === 'Enter' && !e.shiftKey) {
                  e.preventDefault();
                  const cmd = paletteMatches[slashIdx];
                  if (cmd) commitPaletteChoice(cmd);
                  return;
                }
                if (e.key === 'Tab') {
                  e.preventDefault();
                  const cmd = paletteMatches[slashIdx];
                  if (cmd) setText(`/${cmd.name}${cmd.takesArgs ? ' ' : ''}`);
                  return;
                }
                if (e.key === 'Escape') { e.preventDefault(); setText(''); return; }
              }
              // Escape while goal-composing → abort the compose flow
              // without sending anything, matching how Esc dismisses
              // the slash palette above.
              if (goalComposing && e.key === 'Escape') {
                e.preventDefault();
                setGoalComposing(false);
                setText('');
                return;
              }
              if (e.key === 'Enter' && !e.shiftKey) { e.preventDefault(); submit(); }
            }}
            placeholder={
              disabled
                ? 'Waiting for connection…'
                : goalComposing
                  ? "Describe what 'done' looks like — mira will loop until it's met."
                  : planActive
                    ? 'Describe your task to generate a plan…'
                    : 'Work with mira — try /'
            }
            disabled={disabled}
            rows={1}
            className="min-h-[1.7rem] w-full max-h-48 resize-none border-0 bg-transparent px-2.5 py-1.5 text-sm outline-none placeholder:text-muted-foreground/60 disabled:opacity-60"
          />

          {paletteVisible && paletteMatches.length > 0 && (
            <SlashPalette
              matches={paletteMatches}
              activeIdx={slashIdx}
              onHover={setSlashIdx}
              onPick={commitPaletteChoice}
            />
          )}

          {slash.mode === 'args' && (
            <div className="pointer-events-none absolute -top-6 left-0 rounded-md border border-border/60 bg-popover px-2 py-0.5 text-[11px] text-muted-foreground shadow-lg">
              <span className="font-mono text-foreground">/{slash.command.name}</span>{' '}
              <span>{slash.command.usage.replace(`/${slash.command.name}`, '').trim()}</span>
            </div>
          )}
        </div>

        {slashFeedback && (
          <div className="mx-1 mb-1 rounded-md border border-amber-500/30 bg-amber-500/10 px-2 py-1 text-[11.5px] text-amber-200">
            {slashFeedback}
          </div>
        )}

        <div className="flex items-center gap-1.5 px-1">
          <AttachMenu onAttachFile={() => setFilePickerOpen(true)} loading={attachLoading} />

          <SlashButton onClick={() => setText((t) => (t.startsWith('/') ? t : '/' + t))} />

          <ModelPicker
            current={model}
            onPick={onSetModel}
            onSetEffort={onSetEffort}
            open={modelPopOpen}
            onOpenChange={setModelPopOpen}
          />

          <ProjectChip cwd={cwd} onClick={onOpenPicker} />

          <WorktreeChip cwd={cwd} />

          <span className="flex-1" />

          <ModePicker mode={mode} label={modeLabel} onPick={onSetMode} />

          {busy ? (
            <button
              type="button"
              onClick={onInterrupt}
              className="flex size-8 items-center justify-center rounded-full bg-mira-error text-[#0e1013] transition-colors hover:brightness-110"
              title="Stop"
              aria-label="Stop"
            >
              <Square className="size-3.5 fill-current" />
            </button>
          ) : (
            <button
              type="submit"
              disabled={disabled || (!text.trim() && attachments.length === 0)}
              className="flex size-8 items-center justify-center rounded-full bg-foreground text-background transition-opacity hover:opacity-90 disabled:opacity-35"
              title="Send"
              aria-label="Send"
            >
              <ArrowUp className="size-4" />
            </button>
          )}
        </div>
      </form>

      {usage && (
        <div
          className="w-full max-w-3xl px-3 text-right text-[11px] font-mono text-muted-foreground/60"
          title="Session tokens & estimated cost"
        >
          {usage}
        </div>
      )}

      <FilePicker
        open={filePickerOpen}
        startPath={cwd || undefined}
        onClose={() => setFilePickerOpen(false)}
        onPicked={(p) => { attachFile(p); }}
      />
    </div>
    </IconContext.Provider>
  );
}

/* ---------- slash palette ---------- */

function SlashPalette({
  matches, activeIdx, onHover, onPick,
}: {
  matches: SlashCommand[];
  activeIdx: number;
  onHover: (i: number) => void;
  onPick: (cmd: SlashCommand) => void;
}) {
  // Alphabetize so the palette reads as an at-a-glance menu (Codex /
  // Claude Code pattern) — no cognitive hunting for the item you want
  // just because it happened to be registered late. Sort is stable so
  // aliases don't shuffle unpredictably run-to-run.
  const sorted = useMemo(
    () => [...matches].sort((a, b) => displayName(a).localeCompare(displayName(b))),
    [matches],
  );
  // Keyboard navigation still targets the caller's `matches` order —
  // remap the caller's activeIdx onto the sorted index so ↑↓ + Enter
  // land on the same visual row regardless of registration order.
  const activeName = matches[activeIdx]?.name;
  const activeSortedIdx = sorted.findIndex((c) => c.name === activeName);

  return (
    <div
      className={cn(
        'absolute bottom-full left-0 right-0 z-10 mb-2 mx-auto max-w-xl overflow-hidden',
        // Slightly larger radius + subtle inner ring for the Codex-style
        // "elevated card" feel. Shadow is soft and blurred so the popover
        // reads as floating rather than stamped on.
        'rounded-2xl border border-white/[0.07] bg-[#1f2024]/95 backdrop-blur-md',
        'shadow-[0_20px_50px_-16px_rgba(0,0,0,0.85)] ring-1 ring-black/40',
        'animate-fade-in',
      )}
      role="listbox"
    >
      <div className="max-h-[24rem] overflow-y-auto py-1.5">
        {sorted.map((cmd, i) => {
          const Icon = cmd.icon;
          const active = i === activeSortedIdx;
          return (
            <button
              key={cmd.name}
              type="button"
              onMouseEnter={() => {
                // Hover reports back in caller's index space so the
                // parent's state stays coherent with its `matches`.
                const idx = matches.findIndex((c) => c.name === cmd.name);
                if (idx >= 0) onHover(idx);
              }}
              onClick={() => onPick(cmd)}
              role="option"
              aria-selected={active}
              className={cn(
                // Palette rows are borderless with generous horizontal
                // padding so the highlighted row reads as a soft band
                // rather than a discrete button. Vertical rhythm is
                // tight but breathable (py-1.5) — matches the density
                // in Codex's screenshot.
                'flex w-full items-baseline gap-3 px-4 py-1.5 text-left transition-colors',
                active ? 'bg-white/[0.06]' : 'hover:bg-white/[0.035]',
              )}
            >
              <Icon
                className={cn(
                  // Icons stay muted at rest, brighten a touch on
                  // hover / active — mirrors the way Codex fades chrome
                  // into the background until you look at it.
                  'size-[15px] shrink-0 self-center transition-colors',
                  active ? 'text-foreground/85' : 'text-foreground/55',
                )}
              />
              <span
                className={cn(
                  'shrink-0 text-[13.5px] font-medium tracking-tight',
                  active ? 'text-foreground' : 'text-foreground/90',
                )}
              >
                {displayName(cmd)}
              </span>
              <span
                className={cn(
                  'min-w-0 flex-1 truncate text-[13px]',
                  active ? 'text-muted-foreground' : 'text-muted-foreground/70',
                )}
              >
                {cmd.description}
              </span>
            </button>
          );
        })}
      </div>
    </div>
  );
}

/** Human-facing label for a command. Falls back to Title-Case of the
 *  `name` so single-word commands ("new" → "New") don't need a hand-
 *  written label, while multi-word commands ("pull-request") can
 *  override with a proper display string ("Pull request"). */
function displayName(cmd: SlashCommand): string {
  const n = cmd.name;
  if (!n) return n;
  return n.charAt(0).toUpperCase() + n.slice(1).replace(/-/g, ' ');
}

/** Small keyboard-key badge used inside model-picker meta rows. */
function Kbd({ children }: { children: React.ReactNode }) {
  return (
    <kbd className="rounded border border-white/10 bg-white/5 px-1 py-0.5 font-mono text-[10px] text-mira-cyan/80">
      {children}
    </kbd>
  );
}

/* ---------- attach menu (+) ---------- */

function AttachMenu({
  onAttachFile, loading,
}: { onAttachFile: () => void; loading: boolean }) {
  const [open, setOpen] = useState(false);
  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>
        <button
          type="button"
          className="inline-flex size-8 items-center justify-center rounded-full text-muted-foreground transition-colors hover:bg-secondary hover:text-foreground"
          title="Attach"
          aria-label="Attach"
        >
          {/* Override the composer's `fill` default — a fill-weight Plus is chunky
           *  and stands out too much next to the softer / model / mode chips.
           *  Regular weight reads as a clean, standard `+`. */}
          {loading ? <CircleNotch className="size-3.5 animate-spin" /> : <Plus weight="regular" className="size-4" />}
        </button>
      </PopoverTrigger>
      <PopoverContent className="w-60 p-1.5" align="start">
        <div className="px-2 pb-1.5 pt-1 text-[10.5px] font-semibold uppercase tracking-wider text-muted-foreground">
          Attach
        </div>
        <div className="flex flex-col">
          <MenuButton
            icon={<Paperclip className="size-3.5" />}
            label="Attach file…"
            hint="Pick a file to inline into the message"
            onClick={() => { setOpen(false); onAttachFile(); }}
          />
          <MenuButton
            icon={<Camera className="size-3.5" />}
            label="Screenshot"
            hint="Needs a vision model"
            disabled
          />
          <MenuButton
            icon={<Link className="size-3.5" />}
            label="From URL"
            hint="Coming soon"
            disabled
          />
        </div>
      </PopoverContent>
    </Popover>
  );
}

function MenuButton({
  icon, label, hint, disabled, onClick,
}: {
  icon: React.ReactNode;
  label: string;
  hint?: string;
  disabled?: boolean;
  onClick?: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={disabled}
      className={cn(
        'flex items-start gap-2.5 rounded-md px-2.5 py-2 text-left transition-colors',
        disabled
          ? 'cursor-not-allowed text-muted-foreground/40'
          : 'text-foreground hover:bg-accent/10',
      )}
    >
      <span className={cn('mt-0.5 shrink-0', disabled ? 'text-muted-foreground/40' : 'text-muted-foreground')}>
        {icon}
      </span>
      <div className="flex flex-col min-w-0">
        <span className="text-[13px]">{label}</span>
        {hint && <span className="text-[11.5px] text-muted-foreground/70">{hint}</span>}
      </div>
    </button>
  );
}

/* ---------- attachment chip ---------- */

function AttachmentChip({
  attachment, cwd, onRemove,
}: { attachment: Attachment; cwd: string; onRemove: () => void }) {
  const label = relativeTo(attachment.path, cwd);
  return (
    <span
      className="inline-flex items-center gap-1.5 rounded-md border border-border/60 bg-background/60 px-2 py-1 text-[12px]"
      title={`${attachment.path} · ${formatBytes(attachment.bytes)}`}
    >
      <FileIcon className="size-3 shrink-0 text-mira-blue" />
      <span className="max-w-[16rem] truncate">{label}</span>
      <span className="text-[10.5px] text-muted-foreground/70">{formatBytes(attachment.bytes)}</span>
      <button
        type="button"
        onClick={onRemove}
        className="rounded-sm p-0.5 text-muted-foreground transition-colors hover:bg-accent/30 hover:text-foreground"
        aria-label={`Remove ${label}`}
      >
        <X className="size-3" />
      </button>
    </span>
  );
}

function relativeTo(abs: string, base: string): string {
  if (base && abs.startsWith(base + '/')) return abs.slice(base.length + 1);
  if (base && abs === base) return '.';
  const parts = abs.split('/');
  if (parts.length <= 3) return abs;
  return '…/' + parts.slice(-2).join('/');
}

function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${Math.round(n / 1024)} KB`;
  return `${(n / (1024 * 1024)).toFixed(1)} MB`;
}

/** Inline attachments at the top of the message the model receives. Uses
 *  a stable "## Attached files" header and per-file fenced blocks with the
 *  path as the info string, so the model can trivially cite `path:line` in
 *  its reply. Language guess = extension. */
function renderAttachments(atts: Attachment[], cwd: string): string {
  const parts = atts.map((a) => {
    const rel = relativeTo(a.path, cwd);
    const lang = extToLang(a.path);
    return `### ${rel}\n\`\`\`${lang}\n${a.content}\n\`\`\``;
  });
  return `## Attached files\n\n${parts.join('\n\n')}`;
}

function extToLang(path: string): string {
  const dot = path.lastIndexOf('.');
  if (dot < 0) return '';
  const ext = path.slice(dot + 1).toLowerCase();
  const map: Record<string, string> = {
    ts: 'ts', tsx: 'tsx', js: 'js', jsx: 'jsx',
    py: 'python', rb: 'ruby', go: 'go', rs: 'rust', java: 'java',
    kt: 'kotlin', swift: 'swift', c: 'c', h: 'c', cpp: 'cpp', hpp: 'cpp',
    cs: 'csharp', php: 'php', sh: 'bash', bash: 'bash', zsh: 'bash',
    yml: 'yaml', yaml: 'yaml', toml: 'toml', json: 'json', md: 'md',
    html: 'html', css: 'css', scss: 'scss', sql: 'sql',
  };
  return map[ext] ?? '';
}

/* ---------- mode picker (popover) ---------- */

function ModePicker({
  mode, label, onPick,
}: { mode: Mode; label: string; onPick: (m: Mode) => void }) {
  const [open, setOpen] = useState(false);
  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>
        <button
          type="button"
          // shrink-0 + whitespace-nowrap prevents this chip from being
          // squeezed when a long branch name pushes the row past the
          // composer width — before, the label would wrap onto two
          // lines ("Ask each time" → "Ask each\ntime") and vertically
          // bloat the whole toolbar.
          className="inline-flex shrink-0 items-center gap-1.5 whitespace-nowrap rounded-full px-2.5 py-1.5 text-[12.5px] text-muted-foreground hover:bg-mira-elev2 hover:text-foreground transition-colors"
        >
          <Circle className="size-3 shrink-0" />
          <span>{label}</span>
        </button>
      </PopoverTrigger>
      <PopoverContent className="w-64 p-1.5" align="start">
        <div className="px-2 pb-1.5 pt-1 text-[10.5px] font-semibold uppercase tracking-wider text-muted-foreground">
          Approval
        </div>
        <div className="flex flex-col">
          {MODES.map((m) => (
            <button
              key={m.value}
              type="button"
              className={cn(
                'flex flex-col items-start rounded-md px-2.5 py-2 text-left transition-colors',
                'hover:bg-accent/10 hover:text-foreground',
                m.value === mode && 'text-foreground',
              )}
              onClick={() => { onPick(m.value); setOpen(false); }}
            >
              <span className="text-[13px]">{m.label}{m.value === mode && ' ✓'}</span>
              <span className="text-[11.5px] text-muted-foreground">{m.desc}</span>
            </button>
          ))}
        </div>
      </PopoverContent>
    </Popover>
  );
}

/* ---------- model picker (split-pane: Model / Provider / Effort + list) ---------- */

/**
 * Reasoning effort levels — matches OpenAI's `reasoning_effort` field
 * (`minimal` | `low` | `medium` | `high`), plus an `off` state for
 * non-reasoning models. Rendered as a 5-dot horizontal selector.
 *
 * Persisted per model in `localStorage` so switching models remembers
 * the last-used effort for each. Backend plumbing (passing this into
 * `ChatRequest`) is a follow-up — for now the value is UI-only.
 */
const EFFORTS = ['off', 'minimal', 'low', 'medium', 'high'] as const;
type Effort = typeof EFFORTS[number];

const EFFORT_KEY = 'mira.model-effort';
function loadEffort(model: string): Effort {
  try {
    const raw = localStorage.getItem(EFFORT_KEY);
    if (!raw) return 'medium';
    const map = JSON.parse(raw) as Record<string, Effort>;
    return map[model] ?? 'medium';
  } catch { return 'medium'; }
}
function saveEffort(model: string, effort: Effort) {
  try {
    const raw = localStorage.getItem(EFFORT_KEY);
    const map = raw ? JSON.parse(raw) : {};
    map[model] = effort;
    localStorage.setItem(EFFORT_KEY, JSON.stringify(map));
  } catch { /* private mode etc. */ }
}
function prettyEffort(e: Effort): string {
  return e === 'off' ? 'Off' : e.charAt(0).toUpperCase() + e.slice(1);
}

/** Curated shortlist for the "Best for coding" section. Matches the id
 *  substring so it works across providers (OpenRouter's `anthropic/…`,
 *  Groq's `llama-3.3-70b-versatile`, etc.). Order = display order. */
const CODING_MATCHERS: { label: string; match: (id: string) => boolean }[] = [
  { label: 'Claude Sonnet',      match: (id) => /claude.*sonnet/i.test(id) },
  { label: 'Claude Opus',        match: (id) => /claude.*opus/i.test(id) },
  { label: 'GPT-5',              match: (id) => /gpt-5/i.test(id) },
  { label: 'GPT-4o',             match: (id) => /gpt-4o(?!-mini)/i.test(id) },
  { label: 'Gemini 2.5 Pro',     match: (id) => /gemini-2\.5-pro/i.test(id) },
  { label: 'Gemini 2.5 Flash',   match: (id) => /gemini-2\.5-flash/i.test(id) },
  { label: 'DeepSeek V3',        match: (id) => /deepseek.*v3/i.test(id) },
  { label: 'Qwen 3 Coder',       match: (id) => /qwen.*coder/i.test(id) },
  { label: 'Llama 3.3 70B',      match: (id) => /llama-3\.3-70b/i.test(id) },
];

function ModelPicker({
  current, onPick, onSetEffort, open, onOpenChange,
}: {
  current: string;
  onPick: (m: string) => void;
  onSetEffort: (e: string | null) => void;
  open: boolean;
  onOpenChange: (o: boolean) => void;
}) {
  const setOpen = onOpenChange;
  const [models, setModels] = useState<ModelInfo[] | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [query, setQuery] = useState('');
  const [effort, setEffort] = useState<Effort>(() => loadEffort(current));
  // Inner popover: opens beside the meta card when the "Model" row is
  // clicked. Kept separate so the meta card stays open while browsing.
  const [modelListOpen, setModelListOpen] = useState(false);

  useEffect(() => {
    // Pre-fetch models when the outer popover opens so the inner list
    // isn't blank on first hover.
    if (!open || models !== null) return;
    listModels()
      .then((v) => { setModels(v.models); setLoadError(null); })
      .catch((e) => { setModels([]); setLoadError(String((e as Error).message)); });
  }, [open, models]);

  useEffect(() => { if (modelListOpen) setQuery(''); }, [modelListOpen]);
  useEffect(() => {
    // On model swap, load that model's remembered effort and push it up
    // so the backend applies it on the next turn (rather than carrying
    // the previous model's setting).
    const e = loadEffort(current);
    setEffort(e);
    onSetEffort(e === 'off' ? null : e);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [current]);
  // Close the inner list when the outer closes so state doesn't leak
  // across popover reopens.
  useEffect(() => { if (!open) setModelListOpen(false); }, [open]);

  function pickEffort(e: Effort) {
    setEffort(e);
    saveEffort(current, e);
    // "off" tells the backend to omit the field entirely so non-reasoning
    // providers (Groq, OpenRouter for most models, …) aren't hit with an
    // unrecognised parameter.
    onSetEffort(e === 'off' ? null : e);
  }

  function commit(id: string) {
    if (id.trim()) {
      onPick(id.trim());
      setModelListOpen(false);
      setOpen(false);
    }
  }

  const currentInfo = models?.find((m) => m.id === current) ?? null;
  const providerLabel = prettyVendor(vendorOf(current) || currentInfo?.owned_by || 'other');

  const { suggested, groups } = useMemo(() => {
    if (!models) return { suggested: [] as ModelInfo[], groups: [] as Group[] };
    const suggestedIds = new Set<string>();
    const suggested: ModelInfo[] = [];
    for (const matcher of CODING_MATCHERS) {
      const hit = models.find((m) => !suggestedIds.has(m.id) && matcher.match(m.id));
      if (hit) { suggested.push(hit); suggestedIds.add(hit.id); }
    }
    return { suggested, groups: groupModels(models) };
  }, [models]);

  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>
        <button
          type="button"
          className={cn(
            // Capsule dropped: the button lives on the composer's own
            // background now, matching the ChatGPT / Codex "text-only
            // model chip" pattern. Padding stays so the click target is
            // comfortable; hover is a subtle text-color shift instead
            // of a chip-fill.
            'inline-flex items-center gap-2 rounded-md px-2 py-1.5 text-foreground transition-colors',
            'hover:text-foreground/85 max-w-[20rem]',
          )}
        >
          <span className={cn('size-2 rounded-full shrink-0', vendorDotClass(vendorOf(current)))} />
          {/* `leading-none` on both spans normalises the visual baseline —
           *  without it, a smaller effort label sits slightly higher because
           *  each span's line-box is centred separately. `shrink-0` on the
           *  effort guarantees it never gets ellipsised when the model name
           *  is long; `min-w-0` on the model name lets truncate work. */}
          <span className="min-w-0 truncate text-[14px] font-semibold leading-none">
            {prettyLabel(current) || 'model'}
          </span>
          <span className="shrink-0 text-[11.5px] font-medium leading-none text-muted-foreground/80">
            {prettyEffort(effort)}
          </span>
          {/* Same CaretDown as ToolGroup's expand handle — signals
           *  "this opens" without stealing focus from the label. */}
          <CaretDown
            weight="bold"
            className="size-3 shrink-0 text-muted-foreground/60"
          />
        </button>
      </PopoverTrigger>
      <PopoverContent
        className="w-[15rem] p-1.5 rounded-xl border border-border/60 bg-popover/95 backdrop-blur"
        align="end"
        sideOffset={8}
      >
        {/* Meta card: Model / Provider / Effort. Model row is itself a
         *  popover trigger for the actual list, which floats out to the
         *  right so both stay visible side-by-side. */}
        <Popover open={modelListOpen} onOpenChange={setModelListOpen}>
          <PopoverTrigger asChild>
            <button
              type="button"
              className={cn(
                'group flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left transition-colors',
                modelListOpen ? 'bg-accent/60' : 'hover:bg-accent/40',
              )}
              onMouseEnter={() => setModelListOpen(true)}
            >
              {/* Same left column as MetaRow so Model / Provider / Effort
               *  labels line up perfectly. Vendor dot lives with the value
               *  on the right instead of before the label. */}
              <span className="text-[12.5px] text-foreground/80 w-16 shrink-0">Model</span>
              <span
                className="ml-auto flex min-w-0 items-center gap-1.5"
                title={current}
              >
                <span className={cn('size-2 rounded-full shrink-0', vendorDotClass(vendorOf(current)))} />
                <span className="min-w-0 truncate text-[13px] font-semibold text-foreground">
                  {prettyLabel(current) || '—'}
                </span>
              </span>
            </button>
          </PopoverTrigger>
          <PopoverContent
            className="w-[26rem] p-0"
            side="left"
            align="start"
            sideOffset={12}
          >
            <Command shouldFilter>
              <CommandInput
                placeholder={models === null ? 'loading models…' : 'Search models, or type an id…'}
                value={query}
                onValueChange={setQuery}
              />
              <CommandList className="max-h-[26rem]">
                {loadError && (
                  <div className="px-3 py-2 text-[11.5px] text-destructive">{loadError}</div>
                )}
                {models === null && !loadError && (
                  <div className="px-3 py-3 text-xs text-muted-foreground">fetching /v1/models…</div>
                )}

                {!query.trim() && suggested.length > 0 && (
                  <CommandGroup heading="Best for coding">
                    {suggested.map((m) => (
                      <ModelRow key={`sug-${m.id}`} model={m} current={current} onPick={commit} vendor={vendorClass(vendorOf(m.id))} />
                    ))}
                  </CommandGroup>
                )}

                {groups.length > 0 && groups.map((g) => (
                  <CommandGroup key={g.name} heading={g.name}>
                    {g.models.map((m) => (
                      <ModelRow key={m.id} model={m} current={current} onPick={commit} vendor={g.vendor} />
                    ))}
                  </CommandGroup>
                ))}

                {models && models.length > 0 && query.trim() && (
                  <>
                    <CommandSeparator />
                    <CommandGroup heading="Free text">
                      <CommandItem value={`__use__${query}`} onSelect={() => commit(query)}>
                        <span className="text-muted-foreground text-[12.5px]">Use</span>
                        <code className="ml-1 rounded border border-mira-tool/20 bg-mira-tool/[0.12] px-1.5 py-0.5 text-xs font-mono text-mira-tool">
                          {query.trim()}
                        </code>
                      </CommandItem>
                    </CommandGroup>
                  </>
                )}

                <CommandEmpty>no matches</CommandEmpty>
              </CommandList>
              <div className="flex gap-3 border-t border-border px-3 py-1.5 text-[11px] text-muted-foreground">
                <span><Kbd>↑↓</Kbd> nav</span>
                <span><Kbd>↵</Kbd> pick</span>
                <span><Kbd>esc</Kbd> close</span>
              </div>
            </Command>
          </PopoverContent>
        </Popover>

        <MetaRow label="Provider">
          <span className="text-[13px] text-foreground/80">{providerLabel}</span>
        </MetaRow>
        <MetaRow label="Effort">
          <EffortDots value={effort} onChange={pickEffort} />
        </MetaRow>
      </PopoverContent>
    </Popover>
  );
}

function MetaRow({
  label, children,
}: { label: string; children: React.ReactNode }) {
  return (
    <div className="flex items-center gap-2 rounded-md px-2 py-1.5">
      {/* Fixed-width label column keeps Model / Provider / Effort in a
       *  tight left-aligned stack even when the model name is long. */}
      <span className="text-[12.5px] text-foreground/80 w-16 shrink-0">{label}</span>
      <span className="ml-auto min-w-0 flex-1 flex justify-end">{children}</span>
    </div>
  );
}

function EffortDots({ value, onChange }: { value: Effort; onChange: (e: Effort) => void }) {
  const activeIdx = EFFORTS.indexOf(value);
  const lastIdx = EFFORTS.length - 1;
  return (
    <div
      className="flex items-center gap-1.5"
      role="radiogroup"
      aria-label="Reasoning effort"
    >
      {EFFORTS.map((e, i) => {
        const isActive = i === activeIdx;
        const isMax = i === lastIdx;
        return (
          <button
            key={e}
            type="button"
            role="radio"
            aria-checked={isActive}
            aria-label={prettyEffort(e)}
            onClick={() => onChange(e)}
            title={prettyEffort(e)}
            className={cn(
              'rounded-full transition-all shrink-0',
              // Active dot: much bigger + solid white — matches Codex.
              // Inactive dots stay tiny; the "max" slot uses a magenta
              // accent to hint that dialling this all the way up costs.
              isActive
                ? 'size-3 bg-white shadow-[0_0_0_1px_rgba(255,255,255,0.15)]'
                : isMax
                  ? 'size-1.5 bg-fuchsia-400/70 hover:bg-fuchsia-400'
                  : 'size-1.5 bg-muted-foreground/40 hover:bg-muted-foreground/70',
            )}
          />
        );
      })}
    </div>
  );
}

function ModelRow({
  model, current, onPick, vendor,
}: { model: ModelInfo; current: string; onPick: (id: string) => void; vendor: string }) {
  return (
    <CommandItem
      value={`${model.id} ${model.display_name ?? ''} ${model.owned_by ?? ''}`}
      onSelect={() => onPick(model.id)}
      className="flex items-center gap-2"
    >
      <span className={cn('size-2 rounded-full shrink-0', vendorDotClass(vendor))} />
      <div className="min-w-0 flex-1">
        <div className="flex items-center gap-1.5 text-[13.5px] truncate">
          {model.display_name || prettyLabel(model.id)}
          {model.id === current && <span className="text-mira-blue text-xs">✓</span>}
        </div>
        <div className="text-[10.5px] text-muted-foreground/70 font-mono truncate">{model.id}</div>
      </div>
      {model.context_length && (
        <span className="shrink-0 rounded-full border border-mira-blue/25 bg-mira-blue/[0.09] px-1.5 py-0.5 text-[10.5px] text-mira-blue font-mono">
          {formatCtx(model.context_length)}
        </span>
      )}
    </CommandItem>
  );
}

/* ---------- helpers ---------- */

type Group = { name: string; vendor: string; models: ModelInfo[] };

function groupModels(models: ModelInfo[]): Group[] {
  const byKey = new Map<string, ModelInfo[]>();
  for (const m of models) {
    const key = vendorOf(m.id) || (m.owned_by ?? 'other');
    const bucket = byKey.get(key);
    if (bucket) bucket.push(m); else byKey.set(key, [m]);
  }
  return [...byKey.entries()]
    .map(([name, list]) => ({ name: prettyVendor(name), vendor: vendorClass(name), models: list }))
    .sort((a, b) => b.models.length - a.models.length);
}

const VENDOR_TOKENS: Record<string, string> = {
  openai: 'openai', anthropic: 'anthropic', google: 'google', 'google-vertex': 'google',
  meta: 'meta', 'meta-llama': 'meta', mistralai: 'mistral', mistral: 'mistral',
  deepseek: 'deepseek', xai: 'xai', qwen: 'qwen', 'nousresearch': 'nous',
  microsoft: 'microsoft', amazon: 'amazon', cohere: 'cohere', groq: 'groq',
  perplexity: 'perplexity', moonshotai: 'moonshot',
};

// Tailwind-friendly vendor dot palette — keep colours inline so no
// dedicated CSS file is needed. Radial gradients would be nicer; solid is
// good enough at 8px.
const VENDOR_BG: Record<string, string> = {
  openai:    'bg-emerald-500',
  anthropic: 'bg-orange-500',
  google:    'bg-blue-500',
  meta:      'bg-blue-600',
  mistral:   'bg-orange-600',
  deepseek:  'bg-indigo-500',
  xai:       'bg-neutral-200',
  qwen:      'bg-violet-500',
  microsoft: 'bg-sky-500',
  amazon:    'bg-amber-500',
  cohere:    'bg-rose-400',
  groq:      'bg-red-500',
  perplexity:'bg-cyan-500',
  nous:      'bg-purple-500',
  moonshot:  'bg-yellow-400',
  other:     'bg-neutral-500',
};

function vendorOf(id: string): string {
  if (!id) return '';
  const slash = id.indexOf('/');
  if (slash > 0) return id.slice(0, slash).toLowerCase();
  return id.split('-')[0].toLowerCase();
}

function vendorClass(key: string): string {
  return VENDOR_TOKENS[key.toLowerCase()] ?? 'other';
}

function vendorDotClass(vendorKey: string): string {
  const bucket = VENDOR_TOKENS[vendorKey.toLowerCase()] ?? 'other';
  return VENDOR_BG[bucket] ?? VENDOR_BG.other;
}

function prettyVendor(key: string): string {
  const overrides: Record<string, string> = {
    openai: 'OpenAI', anthropic: 'Anthropic', google: 'Google', xai: 'xAI',
    meta: 'Meta', 'meta-llama': 'Meta', mistralai: 'Mistral', deepseek: 'DeepSeek',
    qwen: 'Qwen', amazon: 'Amazon', cohere: 'Cohere', groq: 'Groq',
    perplexity: 'Perplexity', microsoft: 'Microsoft', moonshotai: 'Moonshot AI',
  };
  return overrides[key.toLowerCase()] ?? key.charAt(0).toUpperCase() + key.slice(1);
}

function prettyLabel(id: string): string {
  if (!id) return '';
  const slash = id.indexOf('/');
  return slash > 0 ? id.slice(slash + 1) : id;
}

function formatCtx(n: number): string {
  if (n >= 1000) return `${Math.round(n / 1000)}k`;
  return String(n);
}

function basename(p: string): string {
  if (!p) return '';
  const trimmed = p.replace(/\/+$/, '');
  const i = trimmed.lastIndexOf('/');
  return i >= 0 ? trimmed.slice(i + 1) || '/' : trimmed;
}

/* ---------- slash button (opens the palette by seeding "/") ---------- */

/**
 * Custom slash glyph — Phosphor's `*-Slash` icons are all "crossed-out"
 * variants (BellSlash, ChatSlash, …), and a bare `/` character is too
 * thin to sit alongside chunky filled icons in the composer bar.
 *
 * SVG: rounded, italic-slanted forward slash with a thick stroke.
 * Sized like the other 16px composer icons; `currentColor` inherits the
 * button's text colour so the muted → foreground hover transition still
 * works.
 */
function SlashGlyph({ className }: { className?: string }) {
  return (
    <svg
      viewBox="0 0 24 24"
      className={className}
      fill="none"
      stroke="currentColor"
      strokeWidth="2.75"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      <path d="M15.5 4.5 L8.5 19.5" />
    </svg>
  );
}

function SlashButton({ onClick }: { onClick: () => void }) {
  return (
    <button
      type="button"
      onClick={onClick}
      title="Slash commands"
      aria-label="Open slash palette"
      className="inline-flex size-8 items-center justify-center rounded-full text-muted-foreground transition-colors hover:bg-secondary hover:text-foreground"
    >
      <SlashGlyph className="size-4" />
    </button>
  );
}

/* ---------- plan chip (visible only while plan mode is on) ---------- */

/** Active-state indicator + one-click exit. Renders only when plan mode is
 *  on; clicking it drops the user back to their prior mode. Entering plan
 *  mode happens via `/plan` (autocompletes) or the mode picker. */
function PlanChip({ onExit }: { onExit: () => void }) {
  return (
    <button
      type="button"
      onClick={onExit}
      title="Plan mode on — click to exit"
      className="inline-flex items-center gap-1.5 rounded-full bg-mira-blue/15 px-2.5 py-1.5 text-[12.5px] text-mira-blue transition-colors hover:bg-mira-blue/25"
    >
      <Lightbulb className="size-3 shrink-0" weight="fill" />
      <span>Plan</span>
      <span className="text-mira-blue/70">·</span>
    </button>
  );
}

/* ---------- goal compose chip (visible while typing a new goal condition) ---------- */

/** Sibling of [`PlanChip`]: renders while the user is composing the
 *  goal condition (after `/goal`). Uses a dashed border to signal
 *  "empty / awaiting input" so it visually distinguishes from an
 *  active goal. Click cancels the compose flow. */
function GoalComposeChip({ onCancel }: { onCancel: () => void }) {
  return (
    <button
      type="button"
      onClick={onCancel}
      title="Composing a goal — press Enter to set, or click to cancel"
      className="inline-flex items-center gap-1.5 rounded-full border border-dashed border-mira-purple/50 bg-mira-purple/10 px-2.5 py-1.5 text-[12.5px] text-mira-purple transition-colors hover:bg-mira-purple/20"
    >
      <Target className="size-3 shrink-0" weight="fill" />
      <span className="font-medium">Goal</span>
      <span className="text-mira-purple/70">·</span>
      <span className="opacity-90">describe the condition ↵</span>
    </button>
  );
}

/* ---------- goal chip (visible while a `/goal` is set) ---------- */

/** Mirrors [`PlanChip`]'s pattern so autonomy shows up in the same spot,
 *  just tinted purple to visually distinguish "goal-directed" from
 *  "plan-only". Body shows iteration progress + a truncated condition
 *  so the user sees at a glance how many rounds the autonomous loop
 *  has consumed. Click clears the goal (same as `/goal clear`). */
function GoalChip({ goal, onClear }: { goal: Goal; onClear: () => void }) {
  const running = goal.status === 'active';
  const shortCond =
    goal.condition.length > 48
      ? `${goal.condition.slice(0, 48).trim()}…`
      : goal.condition;
  // Non-active statuses keep the chip visible with a tint so the user
  // can see the terminal state at a glance without having to scroll to
  // the GoalPanel; hover still says "click to clear".
  const tone = running
    ? 'bg-mira-purple/15 text-mira-purple hover:bg-mira-purple/25'
    : goal.status === 'met'
      ? 'bg-emerald-500/15 text-emerald-300 hover:bg-emerald-500/25'
      : goal.status === 'needs_user' || goal.status === 'exhausted'
        ? 'bg-amber-500/15 text-amber-300 hover:bg-amber-500/25'
        : goal.status === 'impossible'
          ? 'bg-red-500/15 text-red-300 hover:bg-red-500/25'
          : 'bg-secondary/60 text-muted-foreground hover:bg-secondary';
  const title = running
    ? `Goal running (${goal.iterations}/${goal.max_iterations}) — click to clear`
    : `Goal ${goal.status.replace('_', ' ')} — click to clear`;
  return (
    <button
      type="button"
      onClick={onClear}
      title={title}
      className={cn(
        'inline-flex items-center gap-1.5 rounded-full px-2.5 py-1.5 text-[12.5px] transition-colors',
        tone,
      )}
    >
      <Target className="size-3 shrink-0" weight="fill" />
      <span className="font-medium">Goal</span>
      <span className="opacity-70">·</span>
      <span className="tabular-nums opacity-90">
        {goal.iterations}/{goal.max_iterations}
      </span>
      <span className="opacity-70">·</span>
      <span className="max-w-[240px] truncate opacity-90">{shortCond}</span>
    </button>
  );
}

/* ---------- worktree chip (branch + dirty + worktree switcher) ---------- */

function WorktreeChip({ cwd }: { cwd: string }) {
  const [open, setOpen] = useState(false);
  const [status, setStatus] = useState<GitStatusView | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [newBranch, setNewBranch] = useState('');

  async function refresh() {
    try {
      const s = await getGitStatus();
      setStatus(s);
      setLoadError(null);
    } catch (e) {
      setLoadError(String((e as Error).message));
    }
  }

  useEffect(() => {
    if (!cwd) { setStatus(null); return; }
    refresh();
  }, [cwd]);

  async function switchTo(path: string) {
    setBusy(true);
    try {
      await putCwd(path);
      // `Ready` broadcast will refresh the parent; also refresh git state so
      // the chip label reflects the new worktree before the WS event lands.
      await refresh();
      setOpen(false);
    } catch (e) {
      setLoadError(String((e as Error).message));
    } finally {
      setBusy(false);
    }
  }

  async function createAndSwitch() {
    const branch = newBranch.trim();
    if (!branch) return;
    setBusy(true);
    try {
      const wt = await createWorktree(branch);
      setNewBranch('');
      await switchTo(wt.path);
    } catch (e) {
      setLoadError(String((e as Error).message));
    } finally {
      setBusy(false);
    }
  }

  const label = status?.in_repo
    ? (status.branch ?? 'detached')
    : 'no git';
  const primary = status?.worktrees.find((w) => !isLinkedWorktree(w.path, status.worktrees));

  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>
        <button
          type="button"
          className="inline-flex items-center gap-1.5 rounded-full px-2.5 py-1.5 text-[12.5px] text-muted-foreground hover:bg-mira-elev2 hover:text-foreground transition-colors max-w-[10rem] min-w-0"
          title={status?.in_repo ? `branch: ${label}${status.dirty ? ' (dirty)' : ''}` : 'not a git repo'}
        >
          <GitBranch className="size-3 shrink-0" />
          <span className="truncate">{label}</span>
          {status?.dirty && <span className="size-1.5 rounded-full bg-amber-500 shrink-0" title="uncommitted changes" />}
          {status?.is_worktree && (
            <span className="rounded-sm bg-mira-blue/15 px-1 text-[9.5px] uppercase tracking-wider text-mira-blue">wt</span>
          )}
        </button>
      </PopoverTrigger>
      <PopoverContent className="w-72 p-1.5" align="start">
        <div className="px-2 pb-1.5 pt-1 text-[10.5px] font-semibold uppercase tracking-wider text-muted-foreground">
          Working tree
        </div>
        {!status?.in_repo && (
          <div className="px-2.5 py-2 text-[12px] text-muted-foreground/70">Not a git repo.</div>
        )}
        {status?.in_repo && (
          <>
            <div className="flex flex-col">
              {status.worktrees.map((w) => (
                <button
                  key={w.path}
                  type="button"
                  disabled={busy || w.is_current}
                  onClick={() => switchTo(w.path)}
                  className={cn(
                    'flex items-center gap-2 rounded-md px-2.5 py-1.5 text-left text-[13px] transition-colors',
                    w.is_current ? 'text-foreground' : 'text-muted-foreground hover:bg-accent/40 hover:text-foreground',
                    busy && !w.is_current && 'opacity-50',
                  )}
                >
                  <GitBranch className="size-3.5 shrink-0" />
                  <span className="min-w-0 flex-1 truncate">{w.branch ?? 'detached'}</span>
                  {w.is_current && <span className="text-mira-blue text-xs">✓</span>}
                  {primary && w.path === primary.path && (
                    <span className="rounded-sm bg-secondary px-1 text-[9.5px] uppercase tracking-wider text-muted-foreground">main</span>
                  )}
                </button>
              ))}
            </div>
            <div className="mt-1.5 border-t border-border/60 pt-1.5">
              <div className="px-2.5 pb-1 text-[10.5px] font-semibold uppercase tracking-wider text-muted-foreground">
                New worktree
              </div>
              <div className="flex items-center gap-1.5 px-1.5">
                <input
                  value={newBranch}
                  onChange={(e) => setNewBranch(e.target.value)}
                  onKeyDown={(e) => { if (e.key === 'Enter') { e.preventDefault(); createAndSwitch(); } }}
                  placeholder="branch name"
                  disabled={busy}
                  className="flex-1 min-w-0 rounded-md border border-border bg-background/60 px-2 py-1 text-[12.5px] outline-none focus:border-mira-blue/60"
                />
                <button
                  type="button"
                  onClick={createAndSwitch}
                  disabled={busy || !newBranch.trim()}
                  className="rounded-md bg-foreground px-2.5 py-1 text-[12px] text-background transition-opacity hover:opacity-90 disabled:opacity-35"
                >
                  Add
                </button>
              </div>
            </div>
          </>
        )}
        {loadError && (
          <div className="mx-1.5 mt-1.5 rounded-md border border-destructive/40 bg-destructive/10 px-2 py-1 text-[11px] text-destructive">
            {loadError}
          </div>
        )}
      </PopoverContent>
    </Popover>
  );
}

/** A worktree is "linked" (not the primary) when it lives under any other
 *  worktree's `.git/worktrees/<name>`. We approximate cheaply: if the path
 *  is *inside* one of the other entries' paths, treat it as linked. Good
 *  enough for display sorting; the backend already tags `is_current`. */
function isLinkedWorktree(path: string, all: { path: string }[]): boolean {
  return all.some((o) => o.path !== path && path.startsWith(o.path + '/'));
}

/* ---------- project chip (opens folder picker) ---------- */

function ProjectChip({ cwd, onClick }: { cwd: string; onClick: () => void }) {
  // Prefer the primary-worktree name when the cwd is a linked worktree so
  // switching branches doesn't visually change the project. Falls back to
  // the cwd basename in every other case (no git, load error, primary tree).
  const [primary, setPrimary] = useState<string | null>(null);
  useEffect(() => {
    let cancelled = false;
    if (!cwd) { setPrimary(null); return; }
    getGitStatus()
      .then((s) => {
        if (cancelled) return;
        setPrimary(s.is_worktree ? (s.primary_project ?? null) : null);
      })
      .catch(() => { if (!cancelled) setPrimary(null); });
    return () => { cancelled = true; };
  }, [cwd]);

  const fallback = cwd ? basename(cwd) : 'Choose project';
  const label = primary ?? fallback;
  const title = primary
    ? `${primary} (worktree at ${cwd})`
    : (cwd || 'Choose a folder');
  return (
    <button
      type="button"
      onClick={onClick}
      title={title}
      className="inline-flex items-center gap-1.5 rounded-full px-2.5 py-1.5 text-[12.5px] text-muted-foreground hover:bg-mira-elev2 hover:text-foreground transition-colors max-w-[9rem] min-w-0"
    >
      <Folder className="size-3 shrink-0" />
      <span className="truncate">{label}</span>
    </button>
  );
}
