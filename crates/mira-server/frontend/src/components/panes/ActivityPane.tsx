/**
 * Activity: the chat as a log of what happened, one card per message —
 * files changed (with line counts), commands run, what was read and
 * searched. From a card you can open a changed file, jump to the message,
 * or put every file back the way it was before that message (the same
 * checkpoint restore the message menu offers, preview first).
 */
import { useMemo, useState } from 'react';
import {
  Activity,
  ArrowUpRight,
  FileText,
  History,
  Loader2,
  Search,
  Terminal,
} from 'lucide-react';
import type { Entry } from '../../App';
import type { DiffPreview } from '../../types';
import { categoryFor, targetOf } from '../ToolGroup';
import { cn } from '@/lib/utils';
import { PaneBar, PaneEmpty } from './paneUi';

/** One message and everything that ran in reply to it. */
export type ActivityTurn = {
  /** The transcript anchor (`turn-<i>`), for jumping to it. */
  id: string;
  /** Index of the user message in the transcript (for restore). */
  userIdx: number;
  text: string;
  body: Entry[];
};

type FileChange = { path: string; add: number; del: number; created: boolean; preview: DiffPreview | null };
type Command = { command: string; failed: boolean; running: boolean; background: boolean };

function summarize(body: Entry[]) {
  const files = new Map<string, FileChange>();
  const commands: Command[] = [];
  let reads = 0;
  let searches = 0;
  let tools = 0;
  for (const e of body) {
    if (e.kind !== 'tool' || e.status === 'denied') continue;
    tools++;
    const name = e.call.function.name;
    const cat = categoryFor(name);
    if (cat === 'write' || cat === 'edit') {
      const path = e.preview?.path ?? (name === 'write_file' || name === 'edit_file' ? targetOf(e.call) : '');
      if (!path) continue;
      const f = files.get(path) ?? { path, add: 0, del: 0, created: false, preview: null };
      for (const l of e.preview?.lines ?? []) {
        if (l.tag === 'add') f.add++;
        else if (l.tag === 'del') f.del++;
      }
      if (e.preview?.kind === 'create') f.created = true;
      f.preview = e.preview ?? f.preview;
      files.set(path, f);
    } else if (name === 'bash' || name === 'run_background') {
      commands.push({
        command: targetOf(e.call),
        failed: !!e.result?.is_error,
        running: e.status === 'running' || e.status === 'pending',
        background: name === 'run_background',
      });
    } else if (cat === 'read') {
      reads++;
    } else if (cat === 'search') {
      searches++;
    }
  }
  return { files: [...files.values()], commands, reads, searches, tools };
}

export function ActivityPane({
  turns,
  busy,
  onJump,
  onRestore,
  onOpenFile,
}: {
  turns: ActivityTurn[];
  /** The agent is mid-turn: the newest card is live, restore waits. */
  busy: boolean;
  onJump: (id: string) => void;
  onRestore: (userIdx: number) => void;
  onOpenFile: (path: string, preview: DiffPreview | null) => void;
}) {
  const [onlyChanges, setOnlyChanges] = useState(false);
  const rows = useMemo(
    () =>
      turns
        .map((t, i) => ({ turn: t, n: i + 1, ...summarize(t.body) }))
        .reverse(),
    [turns],
  );
  const shown = onlyChanges ? rows.filter((r) => r.files.length > 0) : rows;
  const totalFiles = new Set(rows.flatMap((r) => r.files.map((f) => f.path))).size;

  return (
    <div className="flex h-full min-h-0 flex-col">
      <PaneBar>
        <span className="text-[11.5px] text-muted-foreground">
          {rows.length} message{rows.length === 1 ? '' : 's'} · {totalFiles} file{totalFiles === 1 ? '' : 's'} changed
        </span>
        <span className="flex-1" />
        <label className="flex cursor-pointer select-none items-center gap-1.5 text-[11.5px] text-muted-foreground hover:text-foreground">
          <input
            type="checkbox"
            checked={onlyChanges}
            onChange={(e) => setOnlyChanges(e.target.checked)}
            className="size-3 accent-mira-blue"
          />
          Only with changes
        </label>
      </PaneBar>

      <div className="min-h-0 flex-1 overflow-y-auto">
        {shown.length === 0 ? (
          <PaneEmpty icon={<Activity className="size-4" />} title={rows.length ? 'No changes yet' : 'Nothing yet'}>
            {rows.length
              ? 'No message in this chat has changed a file so far.'
              : 'Each message you send gets a card here: the files it changed and the commands it ran.'}
          </PaneEmpty>
        ) : (
          <ol className="relative flex flex-col gap-2 p-3">
            {shown.map((r, i) => {
              const live = busy && i === 0 && !onlyChanges;
              return (
                <li key={r.turn.id} className="relative">
                  <div
                    className={cn(
                      'group overflow-hidden rounded-xl border bg-fg/[0.015] transition-colors',
                      live ? 'border-mira-blue/40' : 'border-border/60 hover:border-border',
                    )}
                  >
                    <div className="flex items-start gap-2.5 px-3 pt-2.5">
                      <span className="mt-px flex h-5 min-w-5 shrink-0 items-center justify-center rounded-md bg-fg/[0.06] px-1 font-mono text-[10.5px] text-muted-foreground">
                        {r.n}
                      </span>
                      <button
                        type="button"
                        onClick={() => onJump(r.turn.id)}
                        title="Jump to this message"
                        className="min-w-0 flex-1 text-left"
                      >
                        <p className="line-clamp-2 text-[12.5px] leading-snug text-foreground/90 group-hover:text-foreground">
                          {r.turn.text || <span className="italic text-muted-foreground">(no text)</span>}
                        </p>
                      </button>
                      {live && <Loader2 className="mt-0.5 size-3.5 shrink-0 animate-spin text-mira-blue" />}
                    </div>

                    <div className="mt-1.5 flex flex-wrap items-center gap-x-3 gap-y-1 px-3 pl-[42px] text-[11px] text-muted-foreground/80">
                      {r.files.length > 0 && (
                        <span>
                          {r.files.length} file{r.files.length === 1 ? '' : 's'}
                        </span>
                      )}
                      {r.commands.length > 0 && (
                        <span className="inline-flex items-center gap-1">
                          <Terminal className="size-3" />
                          {r.commands.length}
                        </span>
                      )}
                      {r.reads > 0 && (
                        <span className="inline-flex items-center gap-1">
                          <FileText className="size-3" />
                          {r.reads} read
                        </span>
                      )}
                      {r.searches > 0 && (
                        <span className="inline-flex items-center gap-1">
                          <Search className="size-3" />
                          {r.searches}
                        </span>
                      )}
                      {r.tools === 0 && <span>Reply only</span>}
                    </div>

                    {r.files.length > 0 && (
                      <div className="mt-2 flex flex-col px-1.5">
                        {r.files.map((f) => (
                          <button
                            key={f.path}
                            type="button"
                            onClick={() => onOpenFile(f.path, f.preview)}
                            className="flex min-w-0 items-center gap-2 rounded-md px-1.5 py-1 text-left text-[12px] transition-colors hover:bg-fg/[0.05]"
                          >
                            <span className="min-w-0 flex-1 truncate font-mono text-foreground/85" title={f.path}>
                              <span className="text-muted-foreground/70">{dirOf(f.path)}</span>
                              {baseOf(f.path)}
                            </span>
                            {f.created && (
                              <span className="shrink-0 rounded bg-emerald-500/10 px-1 text-[10px] font-medium text-emerald-700 dark:text-emerald-300">
                                new
                              </span>
                            )}
                            <span className="shrink-0 font-mono text-[11px] tabular-nums">
                              {f.add > 0 && <span className="text-emerald-600 dark:text-emerald-400">+{f.add}</span>}
                              {f.del > 0 && <span className="ml-1 text-red-600 dark:text-red-400">−{f.del}</span>}
                            </span>
                          </button>
                        ))}
                      </div>
                    )}

                    {r.commands.length > 0 && (
                      <div className="mx-3 mt-2 flex flex-col gap-0.5 rounded-md bg-fg/[0.035] px-2 py-1.5">
                        {r.commands.slice(0, 6).map((c, k) => (
                          <div key={k} className="flex min-w-0 items-center gap-1.5 font-mono text-[11px]">
                            <span
                              className={cn(
                                'shrink-0',
                                c.failed ? 'text-red-500' : c.running ? 'text-mira-blue' : 'text-muted-foreground/60',
                              )}
                            >
                              {c.background ? '&' : '$'}
                            </span>
                            <span className={cn('min-w-0 truncate', c.failed ? 'text-red-600 dark:text-red-300' : 'text-foreground/75')}>
                              {c.command}
                            </span>
                          </div>
                        ))}
                        {r.commands.length > 6 && (
                          <div className="text-[11px] text-muted-foreground/70">+{r.commands.length - 6} more</div>
                        )}
                      </div>
                    )}

                    <div className="mt-2 flex items-center gap-1 border-t border-border/40 px-1.5 py-1">
                      <button
                        type="button"
                        onClick={() => onJump(r.turn.id)}
                        className="inline-flex items-center gap-1 rounded-md px-1.5 py-0.5 text-[11.5px] text-muted-foreground transition-colors hover:bg-fg/[0.06] hover:text-foreground"
                      >
                        <ArrowUpRight className="size-3" />
                        Jump to message
                      </button>
                      <span className="flex-1" />
                      <button
                        type="button"
                        disabled={busy}
                        onClick={() => onRestore(r.turn.userIdx)}
                        title={busy ? 'Wait for the agent to finish' : 'Put the files back the way they were before this message'}
                        className="inline-flex items-center gap-1 rounded-md px-1.5 py-0.5 text-[11.5px] text-muted-foreground transition-colors hover:bg-fg/[0.06] hover:text-foreground disabled:opacity-40"
                      >
                        <History className="size-3" />
                        Restore to before
                      </button>
                    </div>
                  </div>
                </li>
              );
            })}
          </ol>
        )}
      </div>
    </div>
  );
}

function dirOf(p: string) {
  const i = p.lastIndexOf('/');
  return i >= 0 ? p.slice(0, i + 1) : '';
}
function baseOf(p: string) {
  return p.slice(p.lastIndexOf('/') + 1);
}
