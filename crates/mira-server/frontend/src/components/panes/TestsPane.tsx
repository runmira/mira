/**
 * Tests: run the project's tests, see each failure on its own, and hand
 * failures to the agent ("Fix this" puts a ready prompt in the chat box —
 * it never sends on its own).
 *
 * The command comes from `/api/tests/detect` (cargo, npm/pnpm/yarn/bun,
 * pytest, go…) and can be edited; the last one used is remembered per
 * folder. Output streams from `/api/tests/run`; Stop closes the request,
 * which stops the run. The last run is kept per chat while the app is open.
 */
import { useEffect, useMemo, useRef, useState, useSyncExternalStore } from 'react';
import {
  CheckCircle2,
  ChevronDown,
  ChevronRight,
  CornerDownLeft,
  FlaskConical,
  Loader2,
  Play,
  Square,
  XCircle,
} from 'lucide-react';
import { composeText } from '@/lib/attachBridge';
import { postNdjson, sessionQuery } from '@/lib/ndjson';
import { liveCounts, parseTestOutput, type TestFailure } from '@/lib/testParse';
import { cn } from '@/lib/utils';
import { PaneBar, PaneButton, PaneEmpty } from './paneUi';

type Suggestion = { command: string; label: string };
type RunEvent = { type: 'line'; text: string } | { type: 'exit'; code: number };

type Run = {
  command: string;
  lines: string[];
  startedAt: number;
  endedAt: number | null;
  /** Null while running; -1 when stopped or it couldn't start. */
  code: number | null;
  stopped: boolean;
};

/* Runs outlive the pane (switching tabs mid-run keeps it going). */
const runs = new Map<string, Run>();
const aborts = new Map<string, AbortController>();
const listeners = new Set<() => void>();
const emit = () => listeners.forEach((l) => l());
const subscribe = (l: () => void) => {
  listeners.add(l);
  return () => listeners.delete(l);
};

function prefKey(cwd: string) {
  return `mira.testCommand:${cwd}`;
}

export function TestsPane({ sessionId, cwd }: { sessionId: string; cwd: string }) {
  const key = sessionId || 'none';
  const run = useSyncExternalStore(subscribe, () => runs.get(key) ?? null);
  const [suggestions, setSuggestions] = useState<Suggestion[] | null>(null);
  const [command, setCommand] = useState('');
  const [picker, setPicker] = useState(false);
  const [showLog, setShowLog] = useState(false);
  const [, tick] = useState(0);
  const running = run !== null && run.code === null;

  // Suggested commands for this folder; the remembered one wins.
  useEffect(() => {
    let live = true;
    fetch(`/api/tests/detect${sessionQuery(sessionId)}`)
      .then((r) => r.json())
      .then((j: { suggestions?: Suggestion[] }) => {
        if (!live) return;
        const list = j.suggestions ?? [];
        setSuggestions(list);
        let saved = '';
        try {
          saved = localStorage.getItem(prefKey(cwd)) ?? '';
        } catch {
          /* ignore */
        }
        setCommand((c) => c || saved || list[0]?.command || '');
      })
      .catch(() => live && setSuggestions([]));
    return () => {
      live = false;
    };
  }, [sessionId, cwd]);

  // Elapsed time while running.
  useEffect(() => {
    if (!running) return;
    const t = setInterval(() => tick((n) => n + 1), 500);
    return () => clearInterval(t);
  }, [running]);

  async function start(cmd = command) {
    const c = cmd.trim();
    if (!c || running) return;
    try {
      localStorage.setItem(prefKey(cwd), c);
    } catch {
      /* ignore */
    }
    const ctrl = new AbortController();
    aborts.set(key, ctrl);
    let cur: Run = { command: c, lines: [], startedAt: Date.now(), endedAt: null, code: null, stopped: false };
    const put = (next: Run) => {
      cur = next;
      runs.set(key, next);
      emit();
    };
    put(cur);
    // Lines arrive fast; batch renders to one per frame.
    let pending: string[] = [];
    let raf = 0;
    const flush = () => {
      raf = 0;
      if (pending.length) put({ ...cur, lines: [...cur.lines, ...pending] });
      pending = [];
    };
    try {
      await postNdjson<RunEvent>(
        `/api/tests/run${sessionQuery(sessionId)}`,
        { command: c },
        (ev) => {
          if (ev.type === 'line') {
            pending.push(ev.text);
            if (!raf) raf = requestAnimationFrame(flush);
          } else if (ev.type === 'exit') {
            flush();
            put({ ...cur, code: ev.code, endedAt: Date.now() });
          }
        },
        ctrl.signal,
      );
      if (raf) cancelAnimationFrame(raf);
      flush();
      if (cur.code === null) put({ ...cur, code: -1, endedAt: Date.now() });
    } catch (e) {
      if (raf) cancelAnimationFrame(raf);
      flush();
      const stopped = ctrl.signal.aborted;
      put({
        ...cur,
        lines: stopped ? cur.lines : [...cur.lines, (e as Error).message],
        code: -1,
        stopped,
        endedAt: Date.now(),
      });
    } finally {
      aborts.delete(key);
    }
  }

  const summary = useMemo(() => (run ? parseTestOutput(run.lines) : null), [run]);
  const live = useMemo(() => (run && running ? liveCounts(run.lines) : null), [run, running]);
  const elapsed = run ? ((run.endedAt ?? Date.now()) - run.startedAt) / 1000 : 0;

  function fixPrompt(failures: TestFailure[]) {
    const body = failures
      .slice(0, 8)
      .map((f) => `**${f.name}**${f.file ? ` (${f.file})` : ''}\n\`\`\`\n${f.detail.slice(0, 30).join('\n')}\n\`\`\``)
      .join('\n\n');
    const n = failures.length;
    composeText(
      `${n === 1 ? 'This test fails' : `These ${n} tests fail`} when I run \`${run?.command}\`:\n\n${body}\n\nFind the cause and fix it, then run the tests again.`,
    );
  }

  return (
    <div className="flex h-full min-h-0 flex-col">
      <PaneBar>
        <div className="relative flex h-7 min-w-0 flex-1 items-center rounded-md border border-border/70 bg-background/60 focus-within:border-mira-blue/50">
          <FlaskConical className="ml-2 size-3.5 shrink-0 text-muted-foreground/70" />
          <input
            value={command}
            onChange={(e) => setCommand(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === 'Enter') void start();
            }}
            placeholder={suggestions?.length === 0 ? 'Test command (e.g. npm test)' : 'Test command'}
            className="min-w-0 flex-1 bg-transparent px-2 font-mono text-[12px] text-foreground outline-none placeholder:font-sans placeholder:text-muted-foreground/60"
          />
          {suggestions && suggestions.length > 0 && (
            <button
              type="button"
              title="Detected commands"
              onClick={() => setPicker((v) => !v)}
              className="flex h-full items-center border-l border-border/60 px-1.5 text-muted-foreground hover:text-foreground"
            >
              <ChevronDown className="size-3.5" />
            </button>
          )}
          {picker && suggestions && (
            <div className="absolute left-0 right-0 top-[calc(100%+4px)] z-20 overflow-hidden rounded-lg border border-border bg-popover py-1 shadow-lg">
              {suggestions.map((s) => (
                <button
                  key={s.command}
                  type="button"
                  onClick={() => {
                    setCommand(s.command);
                    setPicker(false);
                  }}
                  className="flex w-full items-center gap-2 px-2.5 py-1.5 text-left text-[12px] hover:bg-fg/[0.05]"
                >
                  <span className="font-mono text-foreground">{s.command}</span>
                  <span className="ml-auto text-[11px] text-muted-foreground">{s.label}</span>
                </button>
              ))}
            </div>
          )}
        </div>
        {running ? (
          <PaneButton onClick={() => aborts.get(key)?.abort()}>
            <Square className="size-3 fill-current" />
            Stop
          </PaneButton>
        ) : (
          <PaneButton primary onClick={() => void start()} disabled={!command.trim()}>
            <Play className="size-3 fill-current" />
            Run
          </PaneButton>
        )}
      </PaneBar>

      <div className="min-h-0 flex-1 overflow-y-auto">
        {!run ? (
          <PaneEmpty
            icon={<FlaskConical className="size-4" />}
            title={suggestions === null ? 'Looking for tests…' : command ? 'Run your tests' : 'No test command found'}
            action={
              command ? (
                <PaneButton primary onClick={() => void start()}>
                  <Play className="size-3 fill-current" />
                  Run {command}
                </PaneButton>
              ) : undefined
            }
          >
            {command
              ? 'Failures show up one by one, each with a “Fix this” that hands it to the agent.'
              : 'Type the command that runs this project’s tests above.'}
          </PaneEmpty>
        ) : (
          <div className="flex flex-col gap-3 p-3">
            <StatusCard
              running={running}
              stopped={run.stopped}
              code={run.code}
              passed={running ? live!.passed : summary!.passed}
              failed={running ? live!.failed : summary!.failed}
              skipped={running ? 0 : summary!.skipped}
              elapsed={elapsed}
            />

            {summary && summary.failures.length > 0 && (
              <div className="flex flex-col gap-1.5">
                <div className="flex items-center gap-2 px-0.5">
                  <span className="text-[10.5px] font-semibold uppercase tracking-wider text-muted-foreground/80">
                    Failures
                  </span>
                  <span className="flex-1" />
                  {summary.failures.length > 1 && !running && (
                    <button
                      type="button"
                      onClick={() => fixPrompt(summary.failures)}
                      className="inline-flex items-center gap-1 rounded-md px-1.5 py-0.5 text-[11.5px] font-medium text-mira-blue transition-colors hover:bg-mira-blue/10"
                    >
                      <CornerDownLeft className="size-3" />
                      Fix all {summary.failures.length}
                    </button>
                  )}
                </div>
                {summary.failures.map((f) => (
                  <FailureCard key={f.name} failure={f} onFix={() => fixPrompt([f])} />
                ))}
              </div>
            )}

            <div className="overflow-hidden rounded-lg border border-border/60">
              <button
                type="button"
                onClick={() => setShowLog((v) => !v)}
                className="flex w-full items-center gap-1.5 px-2.5 py-1.5 text-left text-[12px] text-muted-foreground hover:text-foreground"
              >
                <ChevronRight className={cn('size-3.5 transition-transform', showLog && 'rotate-90')} />
                Output
                <span className="text-muted-foreground/60">· {run.lines.length} lines</span>
              </button>
              {showLog && <Log lines={run.lines} />}
            </div>
          </div>
        )}
      </div>
    </div>
  );
}

function StatusCard({
  running,
  stopped,
  code,
  passed,
  failed,
  skipped,
  elapsed,
}: {
  running: boolean;
  stopped: boolean;
  code: number | null;
  passed: number;
  failed: number;
  skipped: number;
  elapsed: number;
}) {
  const ok = !running && !stopped && code === 0 && failed === 0;
  const bad = !running && !stopped && (code !== 0 || failed > 0);
  const title = running
    ? 'Running…'
    : stopped
      ? 'Stopped'
      : ok
        ? passed > 0
          ? `All ${passed} passed`
          : 'Passed'
        : failed > 0
          ? `${failed} failed`
          : `Exited with code ${code}`;
  return (
    <div
      className={cn(
        'flex items-center gap-3 rounded-xl border px-3.5 py-3',
        ok && 'border-emerald-500/30 bg-emerald-500/[0.06]',
        bad && 'border-red-500/30 bg-red-500/[0.05]',
        !ok && !bad && 'border-border/60 bg-fg/[0.02]',
      )}
    >
      {running ? (
        <Loader2 className="size-5 shrink-0 animate-spin text-muted-foreground" />
      ) : ok ? (
        <CheckCircle2 className="size-5 shrink-0 text-emerald-600 dark:text-emerald-400" />
      ) : bad ? (
        <XCircle className="size-5 shrink-0 text-red-600 dark:text-red-400" />
      ) : (
        <Square className="size-4 shrink-0 text-muted-foreground" />
      )}
      <div className="min-w-0 flex-1">
        <div className="text-[13.5px] font-medium text-foreground">{title}</div>
        <div className="mt-0.5 flex flex-wrap items-center gap-x-2.5 text-[11.5px] text-muted-foreground">
          <span className="text-emerald-700 dark:text-emerald-400">{passed} passed</span>
          <span className={failed ? 'text-red-600 dark:text-red-400' : undefined}>{failed} failed</span>
          {skipped > 0 && <span>{skipped} skipped</span>}
          <span className="tabular-nums">{elapsed.toFixed(1)}s</span>
        </div>
      </div>
    </div>
  );
}

function FailureCard({ failure: f, onFix }: { failure: TestFailure; onFix: () => void }) {
  const [open, setOpen] = useState(true);
  return (
    <div className="overflow-hidden rounded-lg border border-red-500/25 bg-red-500/[0.03]">
      <div className="flex items-center gap-2 px-2.5 py-2">
        <button type="button" onClick={() => setOpen((v) => !v)} className="flex min-w-0 flex-1 items-center gap-2 text-left">
          <ChevronRight className={cn('size-3.5 shrink-0 text-muted-foreground/70 transition-transform', open && 'rotate-90')} />
          <XCircle className="size-3.5 shrink-0 text-red-600 dark:text-red-400" />
          <div className="min-w-0">
            <div className="truncate font-mono text-[12px] text-foreground">{f.name}</div>
            {f.file && <div className="truncate text-[11px] text-muted-foreground">{f.file}</div>}
          </div>
        </button>
        <button
          type="button"
          onClick={onFix}
          title="Put a prompt for this failure in the chat box"
          className="inline-flex shrink-0 items-center gap-1 rounded-md border border-border/70 bg-background/60 px-2 py-1 text-[11.5px] font-medium text-foreground/90 transition-colors hover:bg-fg/[0.05]"
        >
          <CornerDownLeft className="size-3" />
          Fix this
        </button>
      </div>
      {open && f.detail.length > 0 && (
        <pre className="max-h-56 overflow-auto border-t border-red-500/15 bg-background/50 px-3 py-2 font-mono text-[11.5px] leading-[1.55] text-foreground/85">
          {f.detail.join('\n')}
        </pre>
      )}
    </div>
  );
}

function Log({ lines }: { lines: string[] }) {
  const ref = useRef<HTMLPreElement | null>(null);
  const stick = useRef(true);
  useEffect(() => {
    const el = ref.current;
    if (el && stick.current) el.scrollTop = el.scrollHeight;
  }, [lines]);
  return (
    <pre
      ref={ref}
      onScroll={(e) => {
        const el = e.currentTarget;
        stick.current = el.scrollHeight - el.scrollTop - el.clientHeight < 24;
      }}
      className="max-h-80 overflow-auto border-t border-border/50 bg-[#fbfbfc] px-3 py-2 font-mono text-[11.5px] leading-[1.55] text-foreground/85 dark:bg-[#0b0b0d]"
    >
      {lines.join('\n') || ' '}
    </pre>
  );
}
