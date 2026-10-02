/**
 * "Bring your chats": other coding agents' history, by project, to pick
 * from and import. Each imported chat becomes a Mira chat on that agent —
 * readable at once, and the next message resumes the real session.
 *
 * Used by onboarding and by Settings → External agents, so it owns its
 * whole flow (scan → pick → import → done) and reports out once.
 */
import { useEffect, useMemo, useState } from 'react';
import { Check, ChevronRight, FolderGit2, Folder, Loader2, RefreshCw } from 'lucide-react';
import { importChats, scanImportableChats, type ImportScan, type ImportSource } from '../api';
import { automatedCount, chatKey, defaultSelection, groupProjects, groupState, type ProjectGroup } from '../lib/importSelection';
import { AgentIcon } from './AgentIcon';
import { cn } from '@/lib/utils';

function ago(ms: number | null): string {
  if (!ms) return '';
  const s = Math.max(0, (Date.now() - ms) / 1000);
  if (s < 3600) return `${Math.max(1, Math.round(s / 60))}m ago`;
  if (s < 86400) return `${Math.round(s / 3600)}h ago`;
  if (s < 86400 * 30) return `${Math.round(s / 86400)}d ago`;
  return new Date(ms).toLocaleDateString(undefined, { month: 'short', day: 'numeric' });
}

function Tick({ state, onClick, disabled }: { state: 'all' | 'some' | 'none'; onClick?: () => void; disabled?: boolean }) {
  return (
    <button
      type="button"
      role="checkbox"
      aria-checked={state === 'all' ? true : state === 'some' ? 'mixed' : false}
      disabled={disabled}
      onClick={onClick}
      className={cn(
        'grid size-4 shrink-0 place-items-center rounded-[5px] border transition-colors',
        state === 'none' ? 'border-border bg-transparent hover:border-foreground/40' : 'border-mira-blue bg-mira-blue text-white',
        disabled && 'opacity-40',
      )}
    >
      {state === 'all' && <Check className="size-3" strokeWidth={3} />}
      {state === 'some' && <span className="h-0.5 w-2 rounded-full bg-white" />}
    </button>
  );
}

function sourceCounts(g: ProjectGroup): [ImportSource, number][] {
  const m = new Map<ImportSource, number>();
  for (const c of g.chats) m.set(c.source, (m.get(c.source) ?? 0) + 1);
  return [...m];
}

function GroupRow({
  g,
  selected,
  setSelected,
  defaultOpen = false,
}: {
  g: ProjectGroup;
  selected: Set<string>;
  setSelected: (fn: (s: Set<string>) => Set<string>) => void;
  defaultOpen?: boolean;
}) {
  const [open, setOpen] = useState(defaultOpen);
  const state = groupState(g, selected);
  const toggleAll = () =>
    setSelected((s) => {
      const next = new Set(s);
      for (const c of g.chats) {
        if (c.imported_as) continue;
        if (state === 'all') next.delete(chatKey(c));
        else next.add(chatKey(c));
      }
      return next;
    });
  const Icon = g.isGit ? FolderGit2 : Folder;
  return (
    <div className="rounded-xl border border-border/60 bg-white/[0.02]">
      <div className="flex items-center gap-3 px-3 py-2.5">
        <Tick state={state} onClick={toggleAll} disabled={g.chats.every((c) => c.imported_as)} />
        <button type="button" onClick={() => setOpen((v) => !v)} className="flex min-w-0 flex-1 items-center gap-2.5 text-left">
          <Icon className="size-4 shrink-0 text-muted-foreground" />
          <span className="min-w-0 flex-1">
            <span className="block truncate text-[13px] font-medium text-foreground">{g.label}</span>
            <span className="block truncate font-mono text-[10.5px] text-muted-foreground/70" title={g.paths.join('\n')}>
              {g.paths[0]}
              {g.paths.length > 1 && ` +${g.paths.length - 1}`}
              {!g.exists && ' · folder missing'}
            </span>
          </span>
          <span className="flex shrink-0 items-center gap-2">
            {sourceCounts(g).map(([src, n]) => (
              <span key={src} className="flex items-center gap-1 text-[11.5px] tabular-nums text-muted-foreground" title={`${n} from ${src === 'codex' ? 'Codex' : 'Claude Code'}`}>
                <AgentIcon kind={src} name={src} size="xs" tile={false} />
                {n}
              </span>
            ))}
            <span className="w-14 text-right text-[11px] text-muted-foreground/70">{ago(g.lastActive)}</span>
            <ChevronRight className={cn('size-3.5 text-muted-foreground/60 transition-transform', open && 'rotate-90')} />
          </span>
        </button>
      </div>
      {open && (
        <div className="max-h-64 overflow-y-auto border-t border-border/50 py-1">
          {g.chats.map((c) => {
            const key = chatKey(c);
            const done = c.imported_as != null;
            return (
              <label
                key={key}
                className={cn('flex cursor-pointer items-center gap-3 px-3 py-1.5 transition-colors hover:bg-white/[0.03]', done && 'cursor-default opacity-60')}
              >
                <Tick
                  state={done || selected.has(key) ? 'all' : 'none'}
                  disabled={done}
                  onClick={() =>
                    setSelected((s) => {
                      const next = new Set(s);
                      if (next.has(key)) next.delete(key);
                      else next.add(key);
                      return next;
                    })
                  }
                />
                <AgentIcon kind={c.source} name={c.source} size="xs" tile={false} />
                <span className="min-w-0 flex-1 truncate text-[12.5px] text-foreground/90">{c.title}</span>
                {done ? (
                  <span className="shrink-0 rounded-full bg-emerald-500/10 px-1.5 py-0.5 text-[10.5px] text-emerald-400">in Mira</span>
                ) : (
                  <span className="shrink-0 text-[11px] tabular-nums text-muted-foreground/70">{c.messages} msgs</span>
                )}
                <span className="w-14 shrink-0 text-right text-[11px] text-muted-foreground/60">{ago(c.updated_at)}</span>
              </label>
            );
          })}
        </div>
      )}
    </div>
  );
}

export function ImportChats({
  onDone,
  doneLabel = 'Done',
}: {
  /** Finished (imported, or nothing to do): how many chats came over. */
  onDone?: (imported: number) => void;
  doneLabel?: string;
}) {
  const [scan, setScan] = useState<ImportScan | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [showOther, setShowOther] = useState(false);
  const [showScripted, setShowScripted] = useState(false);
  const [progress, setProgress] = useState<{ done: number; total: number } | null>(null);
  const [result, setResult] = useState<{ imported: number; failed: number } | null>(null);

  const load = (fresh = false) => {
    setError(null);
    setScan(null);
    scanImportableChats(fresh)
      .then((s) => {
        setScan(s);
        setSelected(defaultSelection(groupProjects(s.projects).groups));
      })
      .catch((e) => setError((e as Error).message));
  };
  useEffect(() => load(), []);

  const { groups, other } = useMemo(() => groupProjects(scan?.projects ?? [], showScripted), [scan, showScripted]);
  const scripted = useMemo(() => automatedCount(scan?.projects ?? []), [scan]);
  const total = scan?.sources.reduce((n, s) => n + s.chats, 0) ?? 0;

  async function run() {
    const all = [...groups, ...other].flatMap((g) => g.chats).filter((c) => selected.has(chatKey(c)));
    setProgress({ done: 0, total: all.length });
    let imported = 0;
    let failed = 0;
    // In batches, so the bar moves and a slow chat doesn't hide progress.
    for (let i = 0; i < all.length; i += 10) {
      const batch = all.slice(i, i + 10);
      try {
        const r = await importChats(batch.map((c) => ({ source: c.source, id: c.id })));
        imported += r.imported.length;
        failed += r.failed.length;
      } catch {
        failed += batch.length;
      }
      setProgress({ done: Math.min(all.length, i + batch.length), total: all.length });
    }
    setResult({ imported, failed });
    setProgress(null);
  }

  if (error) {
    return (
      <div className="rounded-xl border border-destructive/30 bg-destructive/5 p-4 text-[13px] text-destructive">
        Couldn't read your other agents' history: {error}
      </div>
    );
  }
  if (!scan) {
    return (
      <div className="flex items-center gap-2.5 rounded-xl border border-border/60 bg-white/[0.02] px-4 py-6 text-[13px] text-muted-foreground">
        <Loader2 className="size-4 animate-spin" />
        Looking for chats from Claude Code and Codex…
      </div>
    );
  }
  if (result) {
    return (
      <div className="flex flex-col items-center gap-3 rounded-xl border border-emerald-500/25 bg-emerald-500/[0.05] px-6 py-8 text-center">
        <span className="grid size-10 place-items-center rounded-full bg-emerald-500/15 text-emerald-400">
          <Check className="size-5" strokeWidth={2.5} />
        </span>
        <div className="text-[15px] font-semibold text-foreground">
          {result.imported === 0 ? 'Nothing new to bring over' : `${result.imported} chat${result.imported === 1 ? '' : 's'} imported`}
        </div>
        <p className="max-w-[44ch] text-[12.5px] leading-relaxed text-muted-foreground">
          They're in your sidebar, under their projects. Open one and keep going — your next message continues the
          original session, with everything the agent remembers.
          {result.failed > 0 && ` ${result.failed} couldn't be read and were skipped.`}
        </p>
        {onDone && (
          <button type="button" onClick={() => onDone(result.imported)} className="mt-1 rounded-lg bg-foreground px-4 py-2 text-[13px] font-medium text-background">
            {doneLabel}
          </button>
        )}
      </div>
    );
  }
  if (total === 0) {
    return (
      <div className="rounded-xl border border-border/60 bg-white/[0.02] px-4 py-6 text-center text-[13px] text-muted-foreground">
        No Claude Code or Codex chats on this computer yet.
        {onDone && (
          <button type="button" onClick={() => onDone(0)} className="ml-2 underline underline-offset-2 hover:text-foreground">
            Continue
          </button>
        )}
      </div>
    );
  }

  const count = selected.size;
  const missing = scan.sources.filter((s) => s.chats > 0 && !s.found);
  return (
    <div className="flex flex-col gap-3">
      <div className="flex flex-wrap items-center gap-2">
        {scan.sources
          .filter((s) => s.found || s.chats > 0)
          .map((s) => (
            <span key={s.id} className="flex items-center gap-1.5 rounded-full border border-border/60 px-2.5 py-1 text-[12px] text-foreground/85">
              <AgentIcon kind={s.id} name={s.label} size="xs" tile={false} />
              {s.label}
              <span className="tabular-nums text-muted-foreground">{s.chats}</span>
            </span>
          ))}
        <span className="flex-1" />
        <button
          type="button"
          onClick={() => load(true)}
          className="flex items-center gap-1 rounded-md px-2 py-1 text-[11.5px] text-muted-foreground transition-colors hover:bg-white/[0.05] hover:text-foreground"
        >
          <RefreshCw className="size-3" /> Rescan
        </button>
      </div>
      {missing.length > 0 && (
        <p className="text-[11.5px] text-muted-foreground">
          {missing.map((m) => m.label).join(' and ')} isn't installed: its chats import and read fine, and continue once it is.
        </p>
      )}

      <div className="flex max-h-[46vh] flex-col gap-2 overflow-y-auto pr-1">
        {groups.map((g, i) => (
          <GroupRow key={g.key} g={g} selected={selected} setSelected={setSelected} defaultOpen={i === 0} />
        ))}
        {other.length > 0 && (
          <>
            <button
              type="button"
              onClick={() => setShowOther((v) => !v)}
              className="flex items-center gap-1.5 px-1 pt-1 text-[12px] text-muted-foreground transition-colors hover:text-foreground"
            >
              <ChevronRight className={cn('size-3.5 transition-transform', showOther && 'rotate-90')} />
              Other folders ({other.reduce((n, g) => n + g.chats.length, 0)} chats outside a repository)
            </button>
            {showOther && other.map((g) => <GroupRow key={g.key} g={g} selected={selected} setSelected={setSelected} />)}
          </>
        )}
        {scripted > 0 && (
          <button
            type="button"
            onClick={() => setShowScripted((v) => !v)}
            className="self-start px-1 pt-1 text-[11.5px] text-muted-foreground/70 transition-colors hover:text-foreground"
            title="Chats a program ran through the agent — test suites, scripts, other apps"
          >
            {showScripted ? 'Hide' : 'Show'} {scripted} scripted run{scripted === 1 ? '' : 's'}
          </button>
        )}
      </div>

      <div className="flex items-center gap-3 border-t border-border/50 pt-3">
        {progress ? (
          <div className="flex flex-1 items-center gap-3">
            <div className="h-1.5 flex-1 overflow-hidden rounded-full bg-white/[0.08]">
              <div className="h-full rounded-full bg-mira-blue transition-[width] duration-300" style={{ width: `${(progress.done / Math.max(1, progress.total)) * 100}%` }} />
            </div>
            <span className="text-[12px] tabular-nums text-muted-foreground">
              {progress.done}/{progress.total}
            </span>
          </div>
        ) : (
          <>
            <span className="flex-1 text-[12px] text-muted-foreground">
              {count === 0 ? 'Nothing selected' : `${count} chat${count === 1 ? '' : 's'} selected`}
            </span>
            {onDone && (
              <button type="button" onClick={() => onDone(0)} className="rounded-lg px-3 py-2 text-[13px] text-muted-foreground transition-colors hover:text-foreground">
                Skip
              </button>
            )}
            <button
              type="button"
              disabled={count === 0}
              onClick={() => void run()}
              className="rounded-lg bg-mira-blue px-4 py-2 text-[13px] font-medium text-white transition-opacity disabled:opacity-40"
            >
              Import {count > 0 ? count : ''} chat{count === 1 ? '' : 's'}
            </button>
          </>
        )}
      </div>
    </div>
  );
}
