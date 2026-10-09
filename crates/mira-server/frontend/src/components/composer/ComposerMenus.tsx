import { Popover, PopoverContent, PopoverTrigger } from '@/components/ui/popover';
import { cn } from '@/lib/utils';
import { Camera, Folder, Link, Loader, Paperclip, Check as PhCheck, Plus } from 'lucide-react';
import { useEffect, useState } from 'react';
import { getGitStatus } from '../../api';
import type { Mode } from '../../types';
import { ApprovalModeIcon } from '../ApprovalDialog';
export const MODES: { value: Mode; label: string; desc: string }[] = [
  { value: 'plan', label: 'Plan only', desc: 'Reads and searches. No edits, no commands.' },
  { value: 'manual', label: 'Ask each time', desc: 'Prompt before every edit or command.' },
  { value: 'auto', label: 'Auto edits', desc: 'Auto-approve edits. Prompt on commands.' },
  { value: 'edit', label: 'Auto everything', desc: 'Edits + commands run unless a rule blocks.' },
  { value: 'yolo', label: 'Yolo', desc: 'No gating at all.' },
];

export const AGENT_APPROVAL_MODES = MODES.filter((m) =>
  ['manual', 'auto', 'edit'].includes(m.value),
).map((m) => ({
  ...m,
  desc:
    m.value === 'edit'
      ? 'Approve every agent permission request.'
      : m.value === 'manual'
        ? 'Ask before every edit, command and web fetch.'
        : 'Auto-approve edits. Commands and web fetches ask.',
}));

/* ---------- attach menu (+) ---------- */

export function AttachMenu({
  onAttachFile,
  loading,
  onScreenshot,
}: {
  onAttachFile: () => void;
  loading: boolean;
  onScreenshot: () => Promise<void>;
}) {
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
          {loading ? <Loader className="size-3.5 animate-spin" /> : <Plus className="size-4" />}
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
            onClick={() => {
              setOpen(false);
              onAttachFile();
            }}
          />
          <MenuButton
            icon={<Camera className="size-3.5" />}
            label="Screenshot"
            hint="Capture a screen region or window"
            onClick={() => {
              setOpen(false);
              void onScreenshot();
            }}
            disabled={loading}
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

export function MenuButton({
  icon,
  label,
  hint,
  disabled,
  onClick,
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
      <span
        className={cn(
          'mt-0.5 shrink-0',
          disabled ? 'text-muted-foreground/40' : 'text-muted-foreground',
        )}
      >
        {icon}
      </span>
      <div className="flex flex-col min-w-0">
        <span className="text-[13px]">{label}</span>
        {hint && <span className="text-[11.5px] text-muted-foreground/70">{hint}</span>}
      </div>
    </button>
  );
}

/* ---------- mode picker (popover) ---------- */

export function ModePicker({
  mode,
  label,
  onPick,
  modes,
  disabled,
  disabledTitle,
}: {
  mode: Mode;
  label: string;
  onPick: (m: Mode) => void;
  modes?: { value: Mode; label: string; desc: string }[];
  disabled?: boolean;
  disabledTitle?: string;
}) {
  const listed = modes ?? MODES;
  const [open, setOpen] = useState(false);
  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>
        <button
          type="button"
          disabled={disabled}
          title={disabled ? disabledTitle : undefined}
          // shrink-0 + whitespace-nowrap prevents this chip from being
          // squeezed when a long branch name pushes the row past the
          // composer width — before, the label would wrap onto two
          // lines ("Ask each time" → "Ask each\ntime") and vertically
          // bloat the whole toolbar.
          className={cn(
            'inline-flex shrink-0 items-center gap-1.5 whitespace-nowrap rounded-full px-2.5 py-1.5 text-[12.5px] leading-none transition-colors touch:p-2.5',
            mode === 'edit' || mode === 'yolo'
              ? 'text-orange-600 dark:text-orange-400 hover:text-orange-600 dark:hover:text-orange-400'
              : 'text-muted-foreground',
            disabled ? 'cursor-default opacity-80' : 'hover:bg-mira-elev2',
            !disabled && mode !== 'edit' && mode !== 'yolo' && 'hover:text-foreground',
          )}
        >
          <ApprovalModeIcon mode={mode} />
          {/* Icon only on a phone; the menu names every mode. */}
          <span className="block leading-none max-md:sr-only">{label}</span>
        </button>
      </PopoverTrigger>
      <PopoverContent className="w-64 p-1.5" align="start">
        <div className="px-2 pb-1.5 pt-1 text-[10.5px] font-semibold uppercase tracking-wider text-muted-foreground">
          Approval
        </div>
        <div className="flex flex-col">
          {listed.map((m) => (
            <button
              key={m.value}
              type="button"
              className={cn(
                'flex flex-col items-start rounded-md px-2.5 py-2 text-left transition-colors',
                'hover:bg-accent/10 hover:text-foreground',
                m.value === mode && 'text-foreground',
              )}
              onClick={() => {
                onPick(m.value);
                setOpen(false);
              }}
            >
              <span
                className={cn(
                  'flex items-center gap-2 text-[13px] leading-none',
                  (m.value === 'edit' || m.value === 'yolo') &&
                    'text-orange-600 dark:text-orange-400',
                )}
              >
                <ApprovalModeIcon mode={m.value} />
                <span>{m.label}</span>
                {m.value === mode && (
                  <PhCheck aria-hidden="true" className="ml-auto size-3.5 shrink-0" />
                )}
              </span>
              <span
                className={cn(
                  'ml-[22px] mt-1 text-[11.5px]',
                  m.value === 'edit' || m.value === 'yolo'
                    ? 'text-orange-600 dark:text-orange-400'
                    : 'text-muted-foreground',
                )}
              >
                {m.desc}
              </span>
            </button>
          ))}
        </div>
      </PopoverContent>
    </Popover>
  );
}

export function basename(p: string): string {
  if (!p) return '';
  const trimmed = p.replace(/\/+$/, '');
  const i = trimmed.lastIndexOf('/');
  return i >= 0 ? trimmed.slice(i + 1) || '/' : trimmed;
}

/* ---------- project chip (opens folder picker) ---------- */

export function ProjectChip({ cwd, onClick }: { cwd: string; onClick: () => void }) {
  // Prefer the primary-worktree name when the cwd is a linked worktree so
  // switching branches doesn't visually change the project. Falls back to
  // the cwd basename in every other case (no git, load error, primary tree).
  const [primary, setPrimary] = useState<string | null>(null);
  useEffect(() => {
    let cancelled = false;
    if (!cwd) {
      setPrimary(null);
      return;
    }
    getGitStatus()
      .then((s) => {
        if (cancelled) return;
        setPrimary(s.is_worktree ? (s.primary_project ?? null) : null);
      })
      .catch(() => {
        if (!cancelled) setPrimary(null);
      });
    return () => {
      cancelled = true;
    };
  }, [cwd]);

  const fallback = cwd ? basename(cwd) : 'Choose project';
  const label = primary ?? fallback;
  const title = primary ? `${primary} (worktree at ${cwd})` : cwd || 'Choose a folder';
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
