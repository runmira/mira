// The update popups behind UpdateButton, in their own chunk: they're
// opened rarely, and most of the button's weight is here.
import { Fragment, type ReactNode, useEffect, useMemo, useState } from 'react';
import { ArrowDownToLine, ArrowUpRight, Check, CheckCircle2, ChevronDown, Copy, Loader2, RefreshCw, RotateCw, Sparkles, WifiOff } from 'lucide-react';
import { Dialog, DialogContent, DialogDescription, DialogTitle } from './ui/dialog';
import { checkReleaseChannel, newerVersion, parseNotes, RELEASES_URL, type useAppUpdate, type UpdateInfo, type UpdatePhase } from '../lib/updates';
import { isDesktop } from '../lib/desktop';
import miraLogo from '../assets/mira-logo.png';
import { cn } from '@/lib/utils';

export function UpdateDialogs({ u, open, onOpenChange }: { u: ReturnType<typeof useAppUpdate>; open: boolean; onOpenChange: (open: boolean) => void }) {
  const [selected, setSelected] = useState<UpdateInfo | null>(null);
  const update = selected ?? u.update;
  const changeOpen = (value: boolean) => { onOpenChange(value); if (!value) { setSelected(null); u.resetError(); } };
  const early = useEarlyReleases(isDesktop() && open, u.checkedAt);
  const selectRelease = (release: UpdateInfo) => { u.resetError(); setSelected(release); };
  const earlyReleases = isDesktop() ? <EarlyReleases current={u.current} channel={u.channel} onSelect={selectRelease} state={early} /> : null;
  return update ? (
    <UpdateDialog open={open} onOpenChange={changeOpen} update={update} phase={u.phase} onInstall={() => void u.install(selected?.channel)} earlyReleases={earlyReleases} />
  ) : (
    <UpToDateDialog
      open={open}
      onOpenChange={changeOpen}
      current={u.current}
      channel={u.channel}
      checking={u.checking}
      checkedAt={u.checkedAt}
      failed={u.checkFailed}
      earlyReleases={earlyReleases}
      onCheck={() => void u.check()}
    />
  );
}

type EarlyChannel = 'alpha' | 'beta';
type ChannelResult = { update?: UpdateInfo; error?: string; loading?: boolean };

/** Small illustrations drawn for the release channels. */
function ReleaseChannelIcon({ channel }: { channel: EarlyChannel }) {
  return channel === 'beta' ? (
    <svg aria-hidden="true" viewBox="0 0 40 40" className="size-10 overflow-visible" fill="none">
      <path d="M19.5 20C16 7 4 7 6 18c.7 4 6 6 13.5 4" fill="#a78bfa" />
      <path d="M20.5 20C24 7 36 7 34 18c-.7 4-6 6-13.5 4" fill="#c4b5fd" />
      <path d="M19 22C8 20 8 33 14 32c4-.5 5-5 6-9" fill="#8b5cf6" />
      <path d="M21 22c11-2 11 11 5 10-4-.5-5-5-6-9" fill="#a78bfa" />
      <path d="m17 12 3 5 3-5M20 18v9" stroke="#5b21b6" strokeWidth="2" strokeLinecap="round" />
      <circle cx="11" cy="15" r="2" fill="#ede9fe" /><circle cx="29" cy="15" r="2" fill="#ede9fe" />
      <path d="m33 5 .8 2.2L36 8l-2.2.8L33 11l-.8-2.2L30 8l2.2-.8Z" fill="#c4b5fd" />
    </svg>
  ) : (
    <svg aria-hidden="true" viewBox="0 0 40 40" className="size-10 overflow-visible" fill="none">
      <path d="M6 32 22 11M12 34l15-19M5 25l13-15" stroke="#fb923c" strokeWidth="3" strokeLinecap="round" />
      <path d="m26 6 3.2 6.4 7.1 1-5.2 5 1.2 7.1-6.3-3.3-6.3 3.3 1.2-7.1-5.2-5 7.1-1Z" fill="#fbbf24" stroke="#f59e0b" strokeWidth="1.2" strokeLinejoin="round" />
      <path d="m23 13 2-3" stroke="#fef3c7" strokeWidth="2" strokeLinecap="round" />
      <circle cx="25" cy="16" r=".9" fill="#92400e" /><circle cx="29" cy="16" r=".9" fill="#92400e" />
      <path d="M25.5 19q1.5 1.5 3 0" stroke="#92400e" strokeWidth="1.2" strokeLinecap="round" />
      <path d="m8 5 .7 2.3L11 8l-2.3.7L8 11l-.7-2.3L5 8l2.3-.7Z" fill="#fdba74" /><circle cx="34" cy="31" r="1.5" fill="#fdba74" />
    </svg>
  );
}

function useEarlyReleases(enabled: boolean, checkedAt: number | null) {
  const [expanded, setExpanded] = useState(false);
  const [attempt, setAttempt] = useState(0);
  const [results, setResults] = useState<Partial<Record<EarlyChannel, ChannelResult>>>({});
  useEffect(() => {
    if (!enabled) return;
    let live = true;
    setResults({ alpha: { loading: true }, beta: { loading: true } });
    for (const name of ['beta', 'alpha'] as const) {
      void checkReleaseChannel(name).then(update => {
        if (live) setResults(prev => ({ ...prev, [name]: update ? { update } : {} }));
      }).catch(error => {
        const message = String(error);
        const unpublished = /404|release not found|no releases|Could not fetch a valid release JSON/i.test(message);
        if (live) setResults(prev => ({ ...prev, [name]: unpublished ? {} : { error: 'Couldn’t check this channel.' } }));
      });
    }
    return () => { live = false; };
  }, [enabled, checkedAt, attempt]);
  return { expanded, setExpanded, results, retry: () => setAttempt(value => value + 1) };
}

function EarlyReleases({ current, channel, onSelect, state }: { current: string | null; channel: UpdateInfo['channel']; onSelect: (update: UpdateInfo) => void; state: ReturnType<typeof useEarlyReleases> }) {
  const { expanded, setExpanded, results, retry } = state;
  const newerChannels = (['beta', 'alpha'] as const).filter(name => {
    const release = results[name]?.update;
    return !!current && !!release && newerVersion(release.version, current);
  });
  const availability = newerChannels.length === 2 ? 'New Alpha and Beta releases' : newerChannels.length ? `New ${newerChannels[0] === 'alpha' ? 'Alpha' : 'Beta'} release` : null;
  return (
    <section className="border-b border-fg/[0.06] px-5 py-3">
      <button type="button" aria-expanded={expanded} onClick={() => setExpanded(value => !value)} className="flex w-full items-center gap-2.5 rounded-lg px-2 py-2 text-left hover:bg-fg/[0.04]">
        <span aria-hidden="true" className="relative h-10 w-14 shrink-0">
          <span className="absolute left-0 top-0 -rotate-12"><ReleaseChannelIcon channel="beta" /></span>
          <span className="absolute left-5 top-1 rotate-6"><ReleaseChannelIcon channel="alpha" /></span>
        </span>
        <span className="min-w-0 flex-1"><span className="flex items-center gap-2 text-[13px] font-medium">Early releases{availability && <span className="rounded-full bg-emerald-500/10 px-1.5 py-0.5 text-[10px] font-semibold text-emerald-600 dark:text-emerald-400">New</span>}</span><span aria-live="polite" className={cn('block text-[11.5px]', availability ? 'text-emerald-600 dark:text-emerald-400' : 'text-muted-foreground')}>{availability ?? 'Try what’s next with Beta and Alpha'}</span></span>
        <ChevronDown className={cn('size-4 text-muted-foreground transition-transform', expanded && 'rotate-180')} />
      </button>
      {expanded && <div className="mt-2 space-y-2 pb-1">
        <p className="px-2 pb-1 text-[11.5px] leading-relaxed text-muted-foreground">Early builds may have bugs. Choose a release to review it before updating.</p>
        {(['beta', 'alpha'] as const).map(name => {
          const result = results[name];
          const release = result?.update;
          const installed = release?.version === current;
          const older = !!release && name === channel && !!current && !newerVersion(release.version, current);
          return <div key={name} className="flex items-center gap-3 rounded-xl border border-fg/[0.08] bg-fg/[0.025] px-3 py-3">
            <span className="grid size-10 shrink-0 place-items-center"><ReleaseChannelIcon channel={name} /></span>
            <div className="min-w-0 flex-1"><div className="flex items-center gap-2 text-[13px] font-medium">{name === 'beta' ? 'Beta' : 'Alpha'}{name === channel && <span className="rounded bg-fg/[0.06] px-1.5 text-[10px] text-muted-foreground">Current channel</span>}</div>
              <p className="text-[11.5px] text-muted-foreground">{result?.loading ? 'Checking…' : result?.error ?? (release ? release.version : 'No release available yet')}</p>
              <p className="mt-0.5 text-[11px] text-muted-foreground/80">{name === 'beta' ? 'Upcoming features, closer to release' : 'Latest experiments, more frequent changes'}</p>
            </div>
            {result?.loading ? <Loader2 className="size-4 animate-spin text-muted-foreground" /> : result?.error ? <button type="button" onClick={retry} className="text-[12px] text-mira-blue">Retry</button> : release && <button type="button" disabled={installed || older} onClick={() => onSelect(release)} className="shrink-0 rounded-lg border border-fg/[0.1] bg-background px-3 py-1.5 text-[12px] font-medium hover:bg-fg/[0.05] disabled:opacity-50">{installed ? 'Installed' : older ? 'Up to date' : 'Review update'}</button>}
          </div>;
        })}
      </div>}
    </section>
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
  earlyReleases,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  current: string | null;
  channel: UpdateInfo['channel'];
  checking: boolean;
  checkedAt: number | null;
  failed: boolean;
  onCheck: () => void;
  earlyReleases: ReactNode;
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
      <DialogContent className="max-h-[90vh] max-w-[440px] gap-0 overflow-y-auto p-0">
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
            <svg aria-hidden="true" viewBox="0 0 32 32" className="size-6 shrink-0" fill="none">
              <path d="M6 15h20v13H6z" fill="#a78bfa" />
              <path d="M4 11h24v6H4z" fill="#c4b5fd" />
              <path d="M14 11h4v17h-4z" fill="#fbbf24" />
              <path d="M16 11C5 11 7 2 12 5c3 2 4 6 4 6Zm0 0c11 0 9-9 4-6-3 2-4 6-4 6Z" fill="#fbbf24" stroke="#f59e0b" strokeWidth="1.2" strokeLinejoin="round" />
              <path d="M9 20v4" stroke="#ede9fe" strokeWidth="2" strokeLinecap="round" />
              <path d="M6 28h20" stroke="#8b5cf6" strokeWidth="1.5" strokeLinecap="round" />
            </svg>
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

        {earlyReleases}
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
  earlyReleases,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  update: UpdateInfo;
  phase: UpdatePhase;
  onInstall: () => void;
  earlyReleases: ReactNode;
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
      <DialogContent className="max-h-[90vh] max-w-[520px] gap-0 overflow-y-auto border-fg/[0.08] bg-popover dark:bg-[#111215] p-0">
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
                {update.channel !== (window.__MIRA_CHANNEL__ ?? 'stable') && update.target === 'desktop' && <span className="block mt-1">Switching to {update.channel} replaces this app. Sign-in and preferences may need to be set again.</span>}
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

        {!busy && earlyReleases}
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
