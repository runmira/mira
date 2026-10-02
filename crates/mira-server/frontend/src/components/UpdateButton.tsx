/**
 * The toolbar's update button and the popup behind it. Always there: a
 * quiet disc when Mira is up to date (the popup says so and can check
 * again), a green one with a soft ping when a newer build is out (the
 * popup says what shipped and installs it — or, for the CLI behind a
 * browser, gives the command).
 */
import { Fragment, type ReactNode, useMemo, useState } from 'react';
import { ArrowDownToLine, ArrowUpRight, Check, CheckCircle2, Copy, Loader2, RefreshCw, RotateCw, Sparkles, WifiOff } from 'lucide-react';
import { Dialog, DialogContent, DialogDescription, DialogTitle } from './ui/dialog';
import { parseNotes, RELEASES_URL, useAppUpdate, type UpdateInfo, type UpdatePhase } from '../lib/updates';
import { isDesktop } from '../lib/desktop';
import miraLogo from '../assets/mira-logo.png';
import { cn } from '@/lib/utils';

export function UpdateButton() {
  const u = useAppUpdate();
  const [open, setOpen] = useState(false);
  const { update } = u;
  const tip = update
    ? `Mira ${update.version} is available`
    : u.current
      ? `Mira ${u.current} · up to date`
      : 'Updates';
  return (
    <>
      <div className="group relative" data-tauri-drag-region="false">
        <button
          type="button"
          onClick={() => setOpen(true)}
          aria-label={update ? `Update to Mira ${update.version}` : 'Mira is up to date'}
          className="relative grid size-8 place-items-center rounded-lg transition-colors hover:bg-secondary"
        >
          {update ? (
            <>
              {/* A slow ping behind the disc, so it's noticed without nagging. */}
              <span className="absolute size-[22px] animate-[ping_2.4s_cubic-bezier(0,0,0.2,1)_infinite] rounded-full bg-emerald-400/35" />
              <span className="relative grid size-[22px] place-items-center rounded-full bg-emerald-500 text-white shadow-[0_0_0_1px_rgba(16,185,129,0.5),0_4px_12px_-2px_rgba(16,185,129,0.6)]">
                <ArrowDownToLine className="size-3.5" strokeWidth={2.5} />
              </span>
            </>
          ) : (
            <span className="relative grid size-[22px] place-items-center rounded-full bg-fg/[0.06] text-muted-foreground ring-1 ring-fg/[0.08] transition-colors group-hover:text-foreground">
              {u.checking ? <Loader2 className="size-3.5 animate-spin" /> : <ArrowDownToLine className="size-3.5" strokeWidth={2.25} />}
            </span>
          )}
        </button>
        {/* Above: the button sits at the bottom of the sidebar. */}
        <span className="tooltip pointer-events-none absolute bottom-full right-0 z-30 mb-1.5 whitespace-nowrap opacity-0 transition-opacity delay-300 group-hover:opacity-100">
          {tip}
        </span>
      </div>
      {update ? (
        <UpdateDialog open={open} onOpenChange={setOpen} update={update} phase={u.phase} onInstall={() => void u.install()} />
      ) : (
        <UpToDateDialog
          open={open}
          onOpenChange={setOpen}
          current={u.current}
          channel={u.channel}
          checking={u.checking}
          checkedAt={u.checkedAt}
          failed={u.checkFailed}
          onCheck={() => void u.check()}
        />
      )}
    </>
  );
}

function ago(ms: number): string {
  const s = Math.round((Date.now() - ms) / 1000);
  if (s < 45) return 'just now';
  if (s < 3600) return `${Math.round(s / 60)} min ago`;
  return `${Math.round(s / 3600)} h ago`;
}

/** The popup when there's nothing newer: what you're on, and a way to check. */
function UpToDateDialog({
  open,
  onOpenChange,
  current,
  channel,
  checking,
  checkedAt,
  failed,
  onCheck,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  current: string | null;
  channel: UpdateInfo['channel'];
  checking: boolean;
  checkedAt: number | null;
  failed: boolean;
  onCheck: () => void;
}) {
  const desktop = isDesktop();
  const product = !desktop ? 'Mira' : channel === 'stable' ? 'Mira' : `Mira ${channel === 'alpha' ? 'Alpha' : 'Beta'}`;
  const status = checking
    ? 'Checking for updates…'
    : failed
      ? "Couldn't check — are you offline?"
      : checkedAt
        ? `You're up to date · checked ${ago(checkedAt)}`
        : "You're up to date";
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-[440px] gap-0 overflow-hidden p-0">
        <div className="relative overflow-hidden px-7 pb-6 pt-8 text-center">
          <div className="pointer-events-none absolute -top-24 left-1/2 h-56 w-[380px] -translate-x-1/2 rounded-full bg-[radial-gradient(closest-side,rgb(var(--mira-blue)/0.16),transparent)]" />
          <div className="relative mx-auto w-fit">
            <img src={miraLogo} alt="" className="size-14 rounded-full shadow-[0_8px_24px_-6px_rgba(30,58,255,0.5)]" draggable={false} />
            <span
              className={cn(
                'absolute -bottom-1 -right-1 grid size-6 place-items-center rounded-full border-[3px] border-background text-white',
                failed ? 'bg-amber-500' : 'bg-emerald-500',
              )}
            >
              {failed ? <WifiOff className="size-3" strokeWidth={2.5} /> : <Check className="size-3" strokeWidth={3.5} />}
            </span>
          </div>
          <DialogTitle className="relative mt-4 text-[19px] font-semibold tracking-[-0.02em] text-foreground">
            {product} {current ?? ''}
          </DialogTitle>
          <DialogDescription className="relative mt-1 flex items-center justify-center gap-1.5 text-[13px] text-muted-foreground">
            {checking ? <Loader2 className="size-3.5 animate-spin" /> : !failed && <CheckCircle2 className="size-3.5 text-emerald-500" />}
            {status}
          </DialogDescription>
          {desktop && channel !== 'stable' && (
            <p className="relative mt-2 text-[11.5px] text-muted-foreground/80">
              {channel === 'alpha' ? 'Alpha' : 'Beta'} channel: early builds, updated often.
            </p>
          )}
        </div>

        <div className="flex flex-col gap-1 border-y border-fg/[0.06] bg-shade/[0.15] px-4 py-3">
          <a
            href={RELEASES_URL}
            target="_blank"
            rel="noreferrer"
            className="flex items-center gap-3 rounded-lg px-3 py-2 text-[13px] text-foreground/85 transition-colors hover:bg-fg/[0.05] hover:text-foreground"
          >
            <Sparkles className="size-4 text-mira-blue" />
            <span className="flex-1">What's new in Mira</span>
            <ArrowUpRight className="size-3.5 text-muted-foreground" />
          </a>
          {!desktop && (
            <a
              href="https://github.com/runmira/mira/releases/download/desktop-updates/Mira-alpha-arm64.dmg"
              className="flex items-center gap-3 rounded-lg px-3 py-2 text-[13px] text-foreground/85 transition-colors hover:bg-fg/[0.05] hover:text-foreground"
            >
              <ArrowDownToLine className="size-4 text-mira-blue" />
              <span className="flex-1">Get Mira for Mac — updates itself</span>
              <ArrowUpRight className="size-3.5 text-muted-foreground" />
            </a>
          )}
        </div>

        <div className="flex items-center justify-end gap-2 px-6 py-4">
          <button
            type="button"
            onClick={() => onOpenChange(false)}
            className="h-9 rounded-lg px-3.5 text-[13px] text-muted-foreground transition-colors hover:bg-fg/[0.05] hover:text-foreground"
          >
            Done
          </button>
          <button
            type="button"
            disabled={checking}
            onClick={onCheck}
            className="inline-flex h-9 items-center gap-2 rounded-lg bg-foreground px-4 text-[13px] font-semibold text-background transition-opacity hover:opacity-90 disabled:opacity-60"
          >
            <RefreshCw className={cn('size-3.5', checking && 'animate-spin')} />
            {checking ? 'Checking…' : failed ? 'Try again' : 'Check for updates'}
          </button>
        </div>
      </DialogContent>
    </Dialog>
  );
}

function UpdateDialog({
  open,
  onOpenChange,
  update,
  phase,
  onInstall,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  update: UpdateInfo;
  phase: UpdatePhase;
  onInstall: () => void;
}) {
  // GitHub's generated notes end each line "by @user in <PR url>": keep
  // just the PR number.
  const sections = useMemo(
    () =>
      parseNotes(
        (update.notes ?? '').replace(
          / by @[\w-]+ in https:\/\/github\.com\/[^/\s]+\/[^/\s]+\/pull\/(\d+)/g,
          ' (#$1)',
        ),
      ),
    [update.notes],
  );
  const busy = phase.kind === 'downloading' || phase.kind === 'restarting';
  const product =
    update.target === 'cli' ? 'Mira CLI' : update.channel === 'stable' ? 'Mira' : `Mira ${update.channel === 'alpha' ? 'Alpha' : 'Beta'}`;
  const date = update.date ? new Date(update.date) : null;

  return (
    <Dialog open={open} onOpenChange={(v) => !busy && onOpenChange(v)}>
      <DialogContent className="max-w-[520px] gap-0 overflow-hidden border-fg/[0.08] bg-popover dark:bg-[#111215] p-0">
        {/* Header: the mark on a green glow, the version, what you're on now. */}
        <div className="relative overflow-hidden px-7 pb-6 pt-8">
          <div className="pointer-events-none absolute -top-24 left-1/2 h-56 w-[420px] -translate-x-1/2 rounded-full bg-[radial-gradient(closest-side,rgba(16,185,129,0.22),transparent)]" />
          <div className="relative flex items-center gap-4">
            <span className="relative">
              <img src={miraLogo} alt="" className="size-12 rounded-full shadow-[0_8px_24px_-6px_rgba(30,58,255,0.6)]" draggable={false} />
              <span className="absolute -bottom-1 -right-1 grid size-5 place-items-center rounded-full border-2 border-popover dark:border-[#111215] bg-emerald-500 text-white">
                <ArrowDownToLine className="size-2.5" strokeWidth={3} />
              </span>
            </span>
            <div className="min-w-0">
              <span className="inline-flex items-center gap-1 rounded-full border border-emerald-400/25 bg-emerald-400/10 px-2 py-px text-[10.5px] font-semibold uppercase tracking-[0.08em] text-emerald-300">
                <Sparkles className="size-3" /> New version
              </span>
              <DialogTitle className="mt-1.5 text-[20px] font-semibold tracking-[-0.02em] text-foreground">
                {product} {update.version}
              </DialogTitle>
              <DialogDescription className="mt-0.5 text-[12.5px] text-muted-foreground">
                You're on {update.current}
                {date && !Number.isNaN(date.getTime()) && (
                  <> · released {date.toLocaleDateString(undefined, { month: 'short', day: 'numeric', year: 'numeric' })}</>
                )}
              </DialogDescription>
            </div>
          </div>
        </div>

        {/* What shipped. */}
        <div className="max-h-[46vh] overflow-y-auto border-y border-fg/[0.06] bg-shade/20 px-7 py-5">
          {sections.length === 0 ? (
            <p className="text-[13px] leading-relaxed text-muted-foreground">
              {update.notes?.trim() || 'Fixes and improvements.'}
            </p>
          ) : (
            <div className="flex flex-col gap-5">
              {sections.map((s, i) => (
                <section key={i}>
                  {s.heading && (
                    <h3 className="mb-2.5 text-[11px] font-semibold uppercase tracking-[0.12em] text-muted-foreground/80">
                      {s.heading}
                    </h3>
                  )}
                  <ul className="flex flex-col gap-2.5">
                    {s.items.map((it, j) => (
                      <li key={j} className="flex gap-3">
                        <span className="mt-[3px] grid size-4 shrink-0 place-items-center rounded-full bg-emerald-500/15 text-emerald-400">
                          <Check className="size-2.5" strokeWidth={3.5} />
                        </span>
                        <span className="text-[13px] leading-relaxed text-muted-foreground">
                          {it.title && <span className="font-medium text-foreground">{inline(it.title)}. </span>}
                          {inline(it.body)}
                        </span>
                      </li>
                    ))}
                  </ul>
                </section>
              ))}
            </div>
          )}
        </div>

        {update.target === 'cli' ? (
          <CliFooter onLater={() => onOpenChange(false)} />
        ) : (
          <Footer phase={phase} onLater={() => onOpenChange(false)} onInstall={onInstall} />
        )}
      </DialogContent>
    </Dialog>
  );
}

function Footer({ phase, onLater, onInstall }: { phase: UpdatePhase; onLater: () => void; onInstall: () => void }) {
  if (phase.kind === 'downloading' || phase.kind === 'restarting') {
    const pct =
      phase.kind === 'restarting' ? 100 : phase.total ? Math.min(100, (phase.downloaded / phase.total) * 100) : null;
    return (
      <div className="flex flex-col gap-2.5 px-7 py-5">
        <div className="flex items-center justify-between text-[12.5px]">
          <span className="flex items-center gap-2 text-foreground">
            {phase.kind === 'restarting' ? <RotateCw className="size-3.5 animate-spin" /> : <Loader2 className="size-3.5 animate-spin" />}
            {phase.kind === 'restarting' ? 'Restarting into the new version…' : 'Downloading the update…'}
          </span>
          {phase.kind === 'downloading' && (
            <span className="tabular-nums text-muted-foreground">
              {mb(phase.downloaded)}
              {phase.total ? ` / ${mb(phase.total)}` : ''}
            </span>
          )}
        </div>
        <div className="h-1.5 overflow-hidden rounded-full bg-fg/[0.08]">
          <div
            className={cn('h-full rounded-full bg-emerald-500 transition-[width] duration-300', pct === null && 'w-1/3 animate-pulse')}
            style={pct === null ? undefined : { width: `${pct}%` }}
          />
        </div>
      </div>
    );
  }
  return (
    <div className="flex flex-col gap-3 px-7 py-5">
      {phase.kind === 'error' && (
        <p className="rounded-lg border border-red-500/20 bg-red-500/[0.07] px-3 py-2 text-[12px] text-red-300">
          The update didn't install: {phase.message}
        </p>
      )}
      <div className="flex items-center justify-end gap-2">
        <span className="mr-auto text-[11.5px] text-muted-foreground/70">Your chats stay as they are.</span>
        <button
          type="button"
          onClick={onLater}
          className="h-9 rounded-lg px-3.5 text-[13px] text-muted-foreground transition-colors hover:bg-fg/[0.05] hover:text-foreground"
        >
          Later
        </button>
        <button
          type="button"
          onClick={onInstall}
          className="inline-flex h-9 items-center gap-2 rounded-lg bg-emerald-500 px-4 text-[13px] font-semibold text-white shadow-[0_8px_20px_-8px_rgba(16,185,129,0.8)] transition-[filter,transform] hover:brightness-110 active:translate-y-px"
        >
          <ArrowDownToLine className="size-4" strokeWidth={2.5} />
          {phase.kind === 'error' ? 'Try again' : 'Update now'}
        </button>
      </div>
    </div>
  );
}

const CLI_COMMANDS = [
  { label: 'Homebrew', command: 'brew upgrade runmira/tap/mira' },
  { label: 'Install script', command: 'curl -fsSL https://runmira.dev/install.sh | bash' },
];

/** The CLI behind a browser can't update itself from a page: copy and run. */
function CliFooter({ onLater }: { onLater: () => void }) {
  const [copied, setCopied] = useState<string | null>(null);
  return (
    <div className="flex flex-col gap-2.5 px-7 py-5">
      <p className="text-[12px] text-muted-foreground">Run one of these, then restart <code className="font-mono text-foreground/80">mira serve</code>:</p>
      {CLI_COMMANDS.map((c) => (
        <button
          key={c.command}
          type="button"
          onClick={() =>
            void navigator.clipboard.writeText(c.command).then(() => {
              setCopied(c.command);
              setTimeout(() => setCopied(null), 1400);
            })
          }
          className="group flex items-center gap-3 rounded-lg border border-fg/[0.08] bg-shade/[0.2] px-3 py-2 text-left transition-colors hover:border-fg/[0.16]"
        >
          <span className="w-24 shrink-0 text-[11.5px] text-muted-foreground">{c.label}</span>
          <code className="min-w-0 flex-1 truncate font-mono text-[12px] text-foreground/90">{c.command}</code>
          {copied === c.command ? <Check className="size-3.5 text-emerald-500" /> : <Copy className="size-3.5 text-muted-foreground group-hover:text-foreground" />}
        </button>
      ))}
      <div className="flex justify-end">
        <button
          type="button"
          onClick={onLater}
          className="h-9 rounded-lg px-3.5 text-[13px] text-muted-foreground transition-colors hover:bg-fg/[0.05] hover:text-foreground"
        >
          Done
        </button>
      </div>
    </div>
  );
}

function mb(bytes: number): string {
  return `${(bytes / 1_048_576).toFixed(1)} MB`;
}

/** `code` and **bold** inside a note line. */
function inline(text: string): ReactNode {
  return text.split(/(`[^`]+`|\*\*[^*]+\*\*)/g).map((part, i) =>
    part.startsWith('`') && part.endsWith('`') && part.length > 2 ? (
      <code key={i} className="rounded bg-fg/[0.07] px-1 py-px font-mono text-[11.5px] text-foreground/85">
        {part.slice(1, -1)}
      </code>
    ) : part.startsWith('**') && part.endsWith('**') && part.length > 4 ? (
      <span key={i} className="font-medium text-foreground">
        {part.slice(2, -2)}
      </span>
    ) : (
      <Fragment key={i}>{part}</Fragment>
    ),
  );
}
