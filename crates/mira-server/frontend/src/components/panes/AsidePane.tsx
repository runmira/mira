/**
 * Ask aside: a side conversation about the session that never interrupts
 * it. Questions go to `/api/aside`, which answers from a condensed copy of
 * the conversation (plus any `@path` files named) on the session's model,
 * with no tools. Nothing here reaches the main agent unless you send it.
 *
 * The thread is kept per session for as long as the app is open, so
 * switching tabs or chats and coming back finds it where you left it.
 */
import { useEffect, useMemo, useRef, useState, useSyncExternalStore } from 'react';
import { ArrowUp, Check, Copy, CornerDownLeft, MessageCircleQuestion, Square, Trash2 } from 'lucide-react';
import type { Entry } from '../../App';
import { Markdown } from '../Markdown';
import { composeText } from '@/lib/attachBridge';
import { postNdjson, sessionQuery } from '@/lib/ndjson';
import { cn } from '@/lib/utils';
import { PaneBar, PaneIconButton } from './paneUi';

type Turn = { role: 'user' | 'assistant'; content: string; error?: boolean; pending?: boolean };

/* Threads live outside React so they survive the pane unmounting. */
const threads = new Map<string, Turn[]>();
const listeners = new Set<() => void>();
function getThread(id: string): Turn[] {
  return threads.get(id) ?? EMPTY;
}
const EMPTY: Turn[] = [];
function setThread(id: string, next: Turn[]) {
  threads.set(id, next);
  listeners.forEach((l) => l());
}
function subscribe(l: () => void) {
  listeners.add(l);
  return () => listeners.delete(l);
}

const STARTERS = [
  'What is the agent doing right now?',
  'Summarize what has changed so far',
  'Anything risky about this approach?',
  'Which files matter most for this task?',
];

type AsideEvent = { type: 'delta'; text: string } | { type: 'done' } | { type: 'error'; message: string };

export function AsidePane({
  sessionId,
  entries,
  agentBusy,
}: {
  sessionId: string;
  entries: Entry[];
  /** The main agent is mid-turn — shown so it's clear asking is safe. */
  agentBusy: boolean;
}) {
  const key = sessionId || 'none';
  const thread = useSyncExternalStore(subscribe, () => getThread(key));
  const [input, setInput] = useState('');
  const [streaming, setStreaming] = useState(false);
  const abortRef = useRef<AbortController | null>(null);
  const scrollRef = useRef<HTMLDivElement | null>(null);
  const inputRef = useRef<HTMLTextAreaElement | null>(null);
  const stick = useRef(true);

  // Follow the answer as it streams, unless the user scrolled up to read.
  useEffect(() => {
    const el = scrollRef.current;
    if (el && stick.current) el.scrollTop = el.scrollHeight;
  }, [thread]);

  // A different chat is a different thread; stop the old answer.
  useEffect(() => () => abortRef.current?.abort(), [key]);

  // Fallback context for sessions the server holds no transcript for
  // (external agents): a digest of what's on screen.
  const digest = useMemo(() => clientDigest(entries), [entries]);

  async function ask(text: string) {
    const question = text.trim();
    if (!question || streaming) return;
    const history = getThread(key).filter((t) => !t.error && !t.pending);
    const base: Turn[] = [...history, { role: 'user', content: question }];
    setThread(key, [...base, { role: 'assistant', content: '', pending: true }]);
    setInput('');
    setStreaming(true);
    stick.current = true;
    const ctrl = new AbortController();
    abortRef.current = ctrl;
    let answer = '';
    const show = (extra: Partial<Turn> = {}) =>
      setThread(key, [...base, { role: 'assistant', content: answer, pending: true, ...extra }]);
    try {
      await postNdjson<AsideEvent>(
        `/api/aside${sessionQuery(sessionId)}`,
        {
          question,
          history: history.map(({ role, content }) => ({ role, content })),
          context: digest,
        },
        (ev) => {
          if (ev.type === 'delta') {
            answer += ev.text;
            show();
          } else if (ev.type === 'error') {
            throw new Error(ev.message);
          }
        },
        ctrl.signal,
      );
      setThread(key, [...base, { role: 'assistant', content: answer || '_(no answer)_' }]);
    } catch (e) {
      if (ctrl.signal.aborted) {
        setThread(key, [...base, { role: 'assistant', content: answer ? `${answer}\n\n_(stopped)_` : '_(stopped)_' }]);
      } else {
        setThread(key, [...base, { role: 'assistant', content: (e as Error).message, error: true }]);
      }
    } finally {
      setStreaming(false);
      abortRef.current = null;
    }
  }

  function clear() {
    abortRef.current?.abort();
    setThread(key, []);
    inputRef.current?.focus();
  }

  return (
    <div className="flex h-full min-h-0 flex-col">
      <PaneBar>
        <span className="flex items-center gap-1.5 text-[11.5px] text-muted-foreground">
          <span className={cn('size-1.5 rounded-full', agentBusy ? 'bg-emerald-500' : 'bg-fg/25')} />
          {agentBusy ? 'Agent is working — asking won’t interrupt it' : 'Separate from the chat — the agent won’t see this'}
        </span>
        <span className="flex-1" />
        <PaneIconButton title="Clear this thread" onClick={clear} disabled={thread.length === 0} tone="danger">
          <Trash2 className="size-3.5" />
        </PaneIconButton>
      </PaneBar>

      <div
        ref={scrollRef}
        onScroll={(e) => {
          const el = e.currentTarget;
          stick.current = el.scrollHeight - el.scrollTop - el.clientHeight < 40;
        }}
        className="min-h-0 flex-1 overflow-y-auto"
      >
        {thread.length === 0 ? (
          <div className="flex h-full flex-col justify-end gap-3 px-4 pb-4 pt-8">
            <div className="flex flex-col items-start gap-2">
              <div className="flex size-9 items-center justify-center rounded-xl border border-border/60 bg-fg/[0.03] text-muted-foreground">
                <MessageCircleQuestion className="size-4" />
              </div>
              <div className="text-[14px] font-medium text-foreground/90">Ask something on the side</div>
              <p className="max-w-[340px] text-[12.5px] leading-relaxed text-muted-foreground">
                Answers come from what’s happened in this chat so far. Mention files with{' '}
                <code className="rounded bg-fg/[0.06] px-1 font-mono text-[11.5px]">@path</code> to include them.
              </p>
            </div>
            <div className="flex flex-col gap-1.5">
              {STARTERS.map((s) => (
                <button
                  key={s}
                  type="button"
                  onClick={() => void ask(s)}
                  className="rounded-lg border border-border/60 px-3 py-2 text-left text-[12.5px] text-foreground/85 transition-colors hover:border-border hover:bg-fg/[0.04]"
                >
                  {s}
                </button>
              ))}
            </div>
          </div>
        ) : (
          <div className="flex flex-col gap-4 px-3.5 py-4">
            {thread.map((t, i) =>
              t.role === 'user' ? (
                <div key={i} className="flex justify-end">
                  <div className="max-w-[85%] whitespace-pre-wrap rounded-2xl rounded-br-md bg-fg/[0.07] px-3 py-2 text-[13px] leading-relaxed text-foreground">
                    {t.content}
                  </div>
                </div>
              ) : (
                <Answer key={i} turn={t} question={thread[i - 1]?.content ?? ''} />
              ),
            )}
          </div>
        )}
      </div>

      <div className="shrink-0 border-t border-border/60 p-2.5">
        <div className="flex items-end gap-2 rounded-xl border border-border/70 bg-background/60 px-3 py-2 transition-colors focus-within:border-mira-blue/50">
          <textarea
            ref={inputRef}
            value={input}
            rows={1}
            placeholder="Ask aside…"
            onChange={(e) => setInput(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === 'Enter' && !e.shiftKey && !e.nativeEvent.isComposing) {
                e.preventDefault();
                void ask(input);
              }
            }}
            className="max-h-32 min-h-[22px] flex-1 resize-none bg-transparent text-[13px] leading-[22px] text-foreground outline-none placeholder:text-muted-foreground/60 [field-sizing:content]"
          />
          {streaming ? (
            <button
              type="button"
              title="Stop"
              onClick={() => abortRef.current?.abort()}
              className="flex size-7 shrink-0 items-center justify-center rounded-lg bg-fg/[0.1] text-foreground transition-colors hover:bg-fg/[0.15]"
            >
              <Square className="size-3 fill-current" />
            </button>
          ) : (
            <button
              type="button"
              title="Ask (Enter)"
              onClick={() => void ask(input)}
              disabled={!input.trim()}
              className="flex size-7 shrink-0 items-center justify-center rounded-lg bg-mira-blue text-mira-on-accent transition-opacity disabled:opacity-30"
            >
              <ArrowUp className="size-4" />
            </button>
          )}
        </div>
      </div>
    </div>
  );
}

function Answer({ turn, question }: { turn: Turn; question: string }) {
  const [copied, setCopied] = useState(false);
  if (turn.error) {
    return (
      <div className="rounded-lg border border-red-500/30 bg-red-500/[0.06] px-3 py-2 text-[12.5px] text-red-600 dark:text-red-300">
        {turn.content}
      </div>
    );
  }
  return (
    <div className="group/answer min-w-0">
      {turn.pending && !turn.content ? (
        <div className="flex items-center gap-1 py-1.5">
          {[0, 1, 2].map((d) => (
            <span
              key={d}
              className="size-1.5 animate-pulse rounded-full bg-fg/40"
              style={{ animationDelay: `${d * 160}ms` }}
            />
          ))}
        </div>
      ) : (
        <div className="text-[13px]">
          <Markdown text={turn.content} />
        </div>
      )}
      {!turn.pending && turn.content && (
        <div className="mt-1.5 flex items-center gap-1 opacity-0 transition-opacity group-hover/answer:opacity-100">
          <button
            type="button"
            onClick={() => {
              void navigator.clipboard?.writeText(turn.content);
              setCopied(true);
              setTimeout(() => setCopied(false), 1200);
            }}
            className="inline-flex items-center gap-1 rounded-md px-1.5 py-0.5 text-[11.5px] text-muted-foreground transition-colors hover:bg-fg/[0.06] hover:text-foreground"
          >
            {copied ? <Check className="size-3" /> : <Copy className="size-3" />}
            {copied ? 'Copied' : 'Copy'}
          </button>
          <button
            type="button"
            title="Put this in the chat box, for the agent"
            onClick={() =>
              composeText(
                `From a side question — "${question.trim()}":\n\n${turn.content
                  .trim()
                  .split('\n')
                  .map((l) => `> ${l}`)
                  .join('\n')}\n\n`,
              )
            }
            className="inline-flex items-center gap-1 rounded-md px-1.5 py-0.5 text-[11.5px] text-muted-foreground transition-colors hover:bg-fg/[0.06] hover:text-foreground"
          >
            <CornerDownLeft className="size-3" />
            Send to agent
          </button>
        </div>
      )}
    </div>
  );
}

/** What's on screen, condensed — the server's fallback when it has no
 *  transcript of its own for this session. Newest kept. */
function clientDigest(entries: Entry[]): string {
  const parts: string[] = [];
  for (const e of entries) {
    if (e.kind === 'msg') {
      const c = (e.msg.content ?? '').trim();
      if (!c) continue;
      if (e.msg.role === 'user') parts.push(`\n## User\n${c.slice(0, 4000)}`);
      else if (e.msg.role === 'assistant') parts.push(`\n## Agent\n${c.slice(0, 4000)}`);
    } else if (e.kind === 'tool') {
      const args = e.call.function.arguments.slice(0, 300);
      const res = e.result?.content?.split('\n').slice(0, 4).join(' ⏎ ').slice(0, 300);
      parts.push(`- tool ${e.call.function.name}(${args})${res ? `\n  → ${res}` : ''}`);
    }
  }
  const all = parts.join('\n');
  return all.length > 80_000 ? all.slice(all.length - 80_000) : all;
}
