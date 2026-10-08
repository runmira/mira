/**
 * Dismissible info notices above the composer (agent updates and the
 * like), and the helpers that build them. Split out of App.tsx.
 */
import { updateExternalAgent } from '../api';
import { useState } from 'react';
import { AnimatePresence, m } from 'framer-motion';
import {
  Check,
  Bell,
  ShieldAlert,
  X,
} from 'lucide-react';
import type {
  AcpAgentStatus,
} from '../types';

export type InfoNotice = {
  id: string;
  kind: 'short' | 'persistent';
  title: string;
  body?: string | null;
  meta?: string | null;
  tone?: 'info' | 'success' | 'warning' | 'danger';
  data?: unknown;
  updateAgent?: string;
};

export function noticeId(prefix = 'notice') {
  return `${prefix}-${Date.now()}-${Math.random().toString(36).slice(2, 7)}`;
}

export function agentUpdateNotice(prev: AcpAgentStatus[], next: AcpAgentStatus[]): InfoNotice | null {
  const byKind = new Map(prev.map((a) => [a.kind, a]));
  for (const agent of next) {
    const old = byKind.get(agent.kind);
    if (!old) continue;
    if (agent.cli_version && old.cli_version && agent.cli_version !== old.cli_version) {
      return {
        id: noticeId(`agent-${agent.kind}`),
        kind: 'persistent',
        tone: 'info',
        title: `New ${agent.display_name} version available`,
        body: `${old.cli_version} → ${agent.cli_version}`,
        meta: 'External agent update',
        updateAgent: ['codex', 'claude-code', 'opencode'].includes(agent.kind) ? agent.kind : undefined,
        data: { agent },
      };
    }
    if (old.state.state !== 'ready' && agent.state.state === 'ready') {
      return {
        id: noticeId(`agent-${agent.kind}`),
        kind: 'short',
        tone: 'success',
        title: `${agent.display_name} is ready`,
        meta: agent.auth ?? agent.transport ?? null,
        data: { agent },
      };
    }
  }
  return null;
}

export function InfoNoticeHost({ notices, onDismiss, onAgentUpdated, inline = false }: { inline?: boolean; notices: InfoNotice[]; onDismiss: (id: string) => void; onAgentUpdated: (kind: string) => void }) {
  const [updating, setUpdating] = useState<string | null>(null);
  const [updateErrors, setUpdateErrors] = useState<Record<string, string>>({});
  async function update(notice: InfoNotice) {
    if (!notice.updateAgent || updating) return;
    setUpdating(notice.id);
    setUpdateErrors(previous => ({ ...previous, [notice.id]: '' }));
    try {
      await updateExternalAgent(notice.updateAgent);
      onAgentUpdated(notice.updateAgent);
      onDismiss(notice.id);
    } catch (error) {
      setUpdateErrors(previous => ({ ...previous, [notice.id]: error instanceof Error ? error.message : 'Update failed. Try again.' }));
    } finally { setUpdating(null); }
  }
  const short = notices.filter((n) => n.kind === 'short').slice(-1);
  const persistent = notices.filter((n) => n.kind === 'persistent');
  return (
    <>
      <div className="pointer-events-none fixed left-1/2 top-4 z-[80] flex -translate-x-1/2 flex-col items-center gap-2">
        <AnimatePresence initial={false}>
          {short.map((notice) => (
            <m.div
              key={notice.id}
              layout
              initial={{ opacity: 0, y: -18, scale: 0.96 }}
              animate={{ opacity: 1, y: 0, scale: 1 }}
              exit={{ opacity: 0, y: -16, scale: 0.98 }}
              transition={{ type: 'spring', stiffness: 520, damping: 36, mass: 0.8 }}
              className="pointer-events-auto flex max-w-[min(34rem,calc(100vw-2rem))] items-center gap-2 rounded-full border border-border/65 bg-white/95 px-3.5 py-2 text-[13px] text-foreground shadow-[0_16px_45px_-28px_rgba(15,23,42,0.55)] backdrop-blur-xl dark:border-fg/[0.08] dark:bg-secondary/95"
            >
              <NoticeIcon tone={notice.tone} />
              <span className="min-w-0 truncate font-medium">{notice.title}</span>
              {notice.meta && <span className="hidden text-muted-foreground sm:inline">· {notice.meta}</span>}
              <button type="button" onClick={() => onDismiss(notice.id)} className="ml-1 rounded-full p-0.5 text-muted-foreground transition-colors hover:bg-fg/[0.06] hover:text-foreground" aria-label="Dismiss notification">
                <X className="size-3.5" />
              </button>
            </m.div>
          ))}
        </AnimatePresence>
      </div>
      <div className={inline ? "mx-1 mb-1 flex flex-col gap-2" : "pointer-events-none fixed right-4 top-4 z-[79] flex w-[min(24rem,calc(100vw-2rem))] flex-col gap-2"}>
        <AnimatePresence initial={false}>
          {persistent.map((notice) => (
            <m.div
              key={notice.id}
              layout
              initial={{ opacity: 0, x: 28, scale: 0.98 }}
              animate={{ opacity: 1, x: 0, scale: 1 }}
              exit={{ opacity: 0, x: 28, scale: 0.98 }}
              transition={{ type: 'spring', stiffness: 430, damping: 34, mass: 0.9 }}
              className="pointer-events-auto overflow-hidden rounded-2xl border border-border/65 bg-white/96 p-3.5 text-foreground shadow-[0_18px_55px_-30px_rgba(15,23,42,0.55)] backdrop-blur-xl dark:border-fg/[0.08] dark:bg-secondary/95"
            >
              <div className="flex items-start gap-3">
                <span className="mt-0.5 flex size-8 shrink-0 items-center justify-center rounded-xl bg-fg/[0.05] text-muted-foreground ring-1 ring-border/60 dark:bg-fg/[0.06] dark:ring-fg/[0.08]">
                  <NoticeIcon tone={notice.tone} />
                </span>
                <div className="min-w-0 flex-1">
                  {notice.meta && <div className="mb-0.5 text-[11px] font-semibold uppercase tracking-[0.16em] text-muted-foreground/65">{notice.meta}</div>}
                  <div className="text-[13.5px] font-semibold leading-5">{notice.title}</div>
                  {notice.body && <div className="mt-1 text-[12.5px] leading-5 text-muted-foreground">{notice.body}</div>}
                  {notice.updateAgent && <button type="button" disabled={!!updating} onClick={() => void update(notice)} className="mt-2 rounded-md bg-foreground px-3 py-1.5 text-xs font-medium text-background hover:opacity-90 disabled:opacity-50">{updating === notice.id ? 'Updating…' : 'Update now'}</button>}
                  {updateErrors[notice.id] && <p role="alert" className="mt-2 text-xs text-red-400">{updateErrors[notice.id]}</p>}
                </div>
                <button type="button" onClick={() => onDismiss(notice.id)} className="rounded-md p-1 text-muted-foreground/70 transition-colors hover:bg-fg/[0.06] hover:text-foreground" aria-label="Dismiss notification">
                  <X className="size-3.5" />
                </button>
              </div>
            </m.div>
          ))}
        </AnimatePresence>
      </div>
    </>
  );
}

export function NoticeIcon({ tone = 'info' }: { tone?: InfoNotice['tone'] }) {
  if (tone === 'success') return <Check className="size-4 text-emerald-500" />;
  if (tone === 'warning') return <ShieldAlert className="size-4 text-amber-500" />;
  if (tone === 'danger') return <ShieldAlert className="size-4 text-destructive" />;
  return <Bell className="size-4 text-mira-blue" />;
}
