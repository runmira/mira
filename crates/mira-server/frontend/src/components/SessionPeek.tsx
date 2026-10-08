import { useCallback, useEffect, useRef, useState } from 'react';
import { AnimatePresence, motion } from 'framer-motion';
import {
  Bot,
  CircleAlert,
  Clock,
  Folder,
  GitBranch,
  GitMerge,
  GitPullRequest,
  Loader,
  MessagesSquare,
  Sparkles,
  User,
  Wrench,
} from 'lucide-react';
import type { SessionSummary } from '../types';

/**
 * The sidebar's hover card.
 *
 * The row itself is a single truncated line, which is the right shape
 * for scanning and the wrong shape for deciding *which* chat to open.
 * This is what you get for the extra half second of looking: when it was
 * last touched, how the last exchange ended, and what the chat is
 * pointed at.
 *
 * Three things make it feel like one system rather than a bolted-on
 * tooltip:
 *
 *  - **Intent, not reaction.** The card appears after a short dwell and
 *    disappears the instant the pointer leaves a row. Sliding down a
 *    list of twenty rows should feel like turning pages, not like
 *    twenty tooltips.
 *  - **It follows the row, not the pixel.** The card is anchored to the
 *    row it describes and eases to the next one, so a fast slide reads
 *    as one card moving rather than a flicker of twenty.
 *  - **Same materials as everything else.** The surface, radius, shadow
 *    and hairline are the timeline minimap's preview card — same hover
 *    card, different content — so the app has one idea of what a card
 *    looks like.
 */

/** How long the pointer must rest on a row before the card appears. */
const OPEN_DELAY_MS = 260;
/** …and how long it lingers after leaving, so a glance that overshoots
 *  the row edge doesn't flicker. */
const CLOSE_DELAY_MS = 140;

type Preview = {
  lastUser: string | null;
  lastAssistant: string | null;
  lastTool: string | null;
  messageCount: number;
};

export type PeekTarget = {
  session: SessionSummary;
  /** Viewport rect of the row, so the card can sit beside it. */
  rect: DOMRect;
};

/** The PR Mira already knows about, if any. */
export type PeekPr = { number: number; title?: string };

export function SessionPeek({
  target,
  pr,
  onDismiss,
}: {
  target: PeekTarget | null;
  /** Shown only for the row this PR belongs to. */
  pr?: PeekPr | null;
  onDismiss: () => void;
}) {
  const [preview, setPreview] = useState<Preview | null>(null);
  const [shown, setShown] = useState<PeekTarget | null>(null);
  const openTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const closeTimer = useRef<ReturnType<typeof setTimeout> | null>(null);

  const clearTimers = () => {
    if (openTimer.current) clearTimeout(openTimer.current);
    if (closeTimer.current) clearTimeout(closeTimer.current);
    openTimer.current = null;
    closeTimer.current = null;
  };

  // Open dwell, and close on leave. Already open means the pointer is
  // sliding between rows, so re-anchor immediately — a card that waits
  // out the dwell on every row is twenty cards, not one moving card.
  useEffect(() => {
    clearTimers();
    if (target) {
      if (shown) {
        setShown(target);
        return;
      }
      openTimer.current = setTimeout(() => setShown(target), OPEN_DELAY_MS);
    } else {
      closeTimer.current = setTimeout(() => {
        setShown(null);
        setPreview(null);
        onDismiss();
      }, CLOSE_DELAY_MS);
    }
    return clearTimers;
  }, [target, shown, onDismiss]);

  // Fetch the transcript tail once per row. Cached for the life of the
  // component so sliding back over a row you already looked at is
  // instant and costs nothing.
  const cache = useRef(new Map<string, Preview>());
  useEffect(() => {
    if (!shown) return;
    const id = shown.session.id;
    const hit = cache.current.get(id);
    if (hit) {
      setPreview(hit);
      return;
    }
    let live = true;
    void fetch(`/api/sessions/${encodeURIComponent(id)}/preview`, { cache: 'no-store' })
      .then((r) => (r.ok ? r.json() : null))
      .then((v: {
        last_user?: string | null;
        last_assistant?: string | null;
        last_tool?: string | null;
        message_count?: number;
      } | null) => {
        if (!live || !v) return;
        const p: Preview = {
          lastUser: v.last_user ?? null,
          lastAssistant: v.last_assistant ?? null,
          lastTool: v.last_tool ?? null,
          messageCount: v.message_count ?? 0,
        };
        cache.current.set(id, p);
        setPreview(p);
      })
      .catch(() => { /* a chat with no record yet simply shows less */ });
    return () => {
      live = false;
    };
  }, [shown]);

  // Card geometry: beside the row, vertically centred on it, flipped to
  // the other side and clamped so it never leaves the window.
  const place = useCallback((rect: DOMRect) => {
    const CARD_W = 340;
    const CARD_H = 230;
    const gap = 10;
    const left = rect.right + gap + CARD_W <= window.innerWidth
      ? rect.right + gap
      : Math.max(12, rect.left - gap - CARD_W);
    const top = Math.min(
      Math.max(12, rect.top + rect.height / 2 - CARD_H / 2),
      Math.max(12, window.innerHeight - CARD_H - 12),
    );
    return { left, top };
  }, []);

  const session = shown?.session;
  const agent = session?.agent_driver ?? null;
  const engineName = agent ? agentLabel(agent) : prettyModel(session?.model ?? '');

  return (
    <AnimatePresence>
      {shown && session && (
        <motion.div
          key="peek"
          initial={{ opacity: 0, x: -6, scale: 0.98 }}
          animate={{ opacity: 1, x: 0, scale: 1 }}
          exit={{ opacity: 0, x: -4, scale: 0.99 }}
          transition={{ duration: 0.14, ease: [0.22, 0.61, 0.36, 1] }}
          className="pointer-events-none fixed z-50 w-[340px]"
          style={place(shown.rect)}
        >
          <div className="overflow-hidden rounded-xl border border-border bg-popover text-left shadow-xl shadow-shade/25">
            {/* Header: what the chat is, and where it stands. */}
            <div className="px-3.5 pb-2.5 pt-3">
              <div className="line-clamp-2 text-[13px] font-semibold leading-[1.35] text-foreground">
                {sessionLabel(session)}
              </div>
              <div className="mt-1.5 flex flex-wrap items-center gap-x-3 gap-y-1 text-[11px] text-muted-foreground">
                <Meta icon={agent ? Bot : Sparkles} title={agent ? 'External agent' : session.model}>
                  {engineName}
                </Meta>
                {session.updated_at > 0 && (
                  <Meta icon={Clock} title={new Date(session.updated_at * 1000).toLocaleString()}>
                    {timeAgo(session.updated_at)}
                  </Meta>
                )}
                <Meta icon={Folder} title={session.cwd}>{basename(session.cwd)}</Meta>
              </div>
              {(session.needs_attention || session.running) && (
                <div
                  className={
                    session.needs_attention
                      ? 'mt-2 inline-flex items-center gap-1.5 rounded-md bg-amber-500/12 px-2 py-0.5 text-[11px] font-medium text-amber-700 dark:text-amber-400'
                      : 'mt-2 inline-flex items-center gap-1.5 rounded-md bg-mira-blue/10 px-2 py-0.5 text-[11px] font-medium text-mira-blue'
                  }
                >
                  {session.needs_attention ? (
                    <>
                      <CircleAlert className="size-3" /> Waiting on you
                    </>
                  ) : (
                    <>
                      <Loader className="size-3 animate-spin" /> Working
                    </>
                  )}
                </div>
              )}
            </div>

            {/* The last exchange. */}
            <div className="space-y-2 border-t border-border/60 bg-fg/[0.02] px-3.5 py-2.5">
              {preview?.lastUser || preview?.lastAssistant ? (
                <>
                  {preview.lastUser && (
                    <Line icon={User} label="You">
                      <span className="line-clamp-2 text-foreground/85">{preview.lastUser}</span>
                    </Line>
                  )}
                  {preview.lastAssistant && (
                    <Line icon={agent ? Bot : Sparkles} label={engineName}>
                      <span className="line-clamp-3 text-muted-foreground">{preview.lastAssistant}</span>
                    </Line>
                  )}
                </>
              ) : (
                <div className="text-[12px] text-muted-foreground/70">{preview ? 'No messages yet' : 'Loading…'}</div>
              )}
            </div>

            {/* Facts: size, last tool, branch, PR. */}
            {(preview?.messageCount || preview?.lastTool || session.worktree_branch || pr) && (
              <div className="flex items-center gap-3 border-t border-border/60 px-3.5 py-2 text-[11px] text-muted-foreground">
                {preview && preview.messageCount > 0 && (
                  <Meta icon={MessagesSquare} title="Messages">{preview.messageCount}</Meta>
                )}
                {preview?.lastTool && (
                  <Meta icon={Wrench} title="Last tool" mono>{preview.lastTool}</Meta>
                )}
                <span className="flex-1" />
                {session.worktree_branch && (
                  <Meta
                    icon={session.worktree_status === 'merged' ? GitMerge : GitBranch}
                    title={session.worktree_branch}
                  >
                    {shortBranch(session.worktree_branch)}
                  </Meta>
                )}
                {/* A PR chip only for the chat Mira already knows the PR of
                    (the open one): a GitHub call per hovered row would put
                    the network in front of a glance. */}
                {pr && <PrChip pr={pr} />}
              </div>
            )}
          </div>
        </motion.div>
      )}
    </AnimatePresence>
  );
}

type Icon = typeof Clock;

/** One small fact: an icon and a short value. */
function Meta({ icon: I, title, mono, children }: { icon: Icon; title?: string; mono?: boolean; children: React.ReactNode }) {
  return (
    <span className="inline-flex min-w-0 items-center gap-1" title={title}>
      <I className="size-3 shrink-0 opacity-70" aria-hidden />
      <span className={mono ? 'max-w-[9rem] truncate font-mono' : 'max-w-[9rem] truncate'}>{children}</span>
    </span>
  );
}

/** One side of the last exchange. */
function Line({ icon: I, label, children }: { icon: Icon; label: string; children: React.ReactNode }) {
  return (
    <div className="flex gap-2 text-[12px] leading-[1.45]">
      <I className="mt-[2px] size-3.5 shrink-0 text-muted-foreground/70" aria-label={label} />
      {children}
    </div>
  );
}

/** The PR chip. Its data comes from the app's existing branch-PR probe
 *  rather than a fetch of its own. */
function PrChip({ pr }: { pr: PeekPr }) {
  return (
    <span className="inline-flex items-center gap-1 text-mira-purple" title={pr.title}>
      <GitPullRequest className="size-3" aria-hidden />
      #{pr.number}
    </span>
  );
}

function basename(p: string): string {
  const t = p.replace(/\/+$/, '');
  return t.slice(t.lastIndexOf('/') + 1) || t || 'folder';
}

/** `claude-sonnet-4-5-20250929` → `Claude Sonnet 4.5`: readable, not exact. */
function prettyModel(model: string): string {
  const base = model.split('/').pop() ?? model;
  if (!base) return 'Mira';
  const words = base.replace(/-\d{8}$/, '').split(/[-_]/);
  const out: string[] = [];
  for (const w of words) {
    const prev = out[out.length - 1];
    if (/^\d+$/.test(w) && prev && /^\d+$/.test(prev)) out[out.length - 1] = `${prev}.${w}`;
    else out.push(/^[a-z]/.test(w) ? w[0].toUpperCase() + w.slice(1) : w);
  }
  return out.join(' ');
}

/* ---------- small helpers ---------- */

function sessionLabel(s: SessionSummary): string {
  const raw = (s.title ?? s.first_user_message ?? '').trim();
  if (!raw) return 'Untitled';
  return raw.length > 120 ? `${raw.slice(0, 120)}…` : raw;
}

function agentLabel(kind: string): string {
  const known: Record<string, string> = {
    'claude-code': 'Claude Code', codex: 'Codex', cursor: 'Cursor',
    grok: 'Grok', opencode: 'OpenCode', antigravity: 'Antigravity',
  };
  return known[kind] ?? kind;
}

/** `feat/thing` → `thing`: the branch name trimmed to its last segment,
 *  which is the part that identifies the work. */
function shortBranch(branch: string): string {
  const parts = branch.split('/');
  return parts[parts.length - 1] || branch;
}

/** Relative time, matching the sidebar rows' own clock. */
function timeAgo(epochSeconds: number): string {
  const s = Math.max(0, Math.floor(Date.now() / 1000) - epochSeconds);
  if (s < 60) return 'now';
  if (s < 3600) return `${Math.floor(s / 60)}m`;
  if (s < 86400) return `${Math.floor(s / 3600)}h`;
  if (s < 86400 * 7) return `${Math.floor(s / 86400)}d`;
  if (s < 86400 * 30) return `${Math.floor(s / (86400 * 7))}w`;
  return new Date(epochSeconds * 1000).toLocaleDateString();
}
