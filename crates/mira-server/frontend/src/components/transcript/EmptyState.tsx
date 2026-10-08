/**
 * The new-chat empty state (starter prompts, recent chats) and the
 * placeholder for unbuilt views. Split out of App.tsx.
 */
import { useEffect, useState } from 'react';
import { listSessions } from '../../api';
import { ProjectSwitcher } from '../ProjectSwitcher';
import { basename } from '../../lib/utils';
import miraLogo from '../../assets/mira-logo.png';
import type {
  SessionSummary,
} from '../../types';

export const STARTER_PROMPTS = [
  'Give me a tour of this codebase',
  'Review my uncommitted changes',
  'Find likely bugs in the recent commits',
  'Write tests for the least-covered module',
];

export function EmptyState({
  cwd,
  onPrompt,
  onOpenSession,
  onSwitchProject,
  onNewProject,
}: {
  cwd: string;
  onPrompt: (text: string) => void;
  onOpenSession: (id: string) => void;
  /** Point this chat at another folder. Starts a fresh session there. */
  onSwitchProject: (path: string) => void;
  /** "New project" — the platform folder panel on the desktop build. */
  onNewProject: () => void;
}) {
  const [recent, setRecent] = useState<SessionSummary[]>([]);
  const [switcherOpen, setSwitcherOpen] = useState(false);
  const project = basename(cwd) || 'this folder';
  useEffect(() => {
    listSessions()
      .then((all) =>
        setRecent(
          all
            .filter((s) => s.cwd === cwd && s.message_count > 0 && !s.active)
            .sort((a, b) => b.updated_at - a.updated_at)
            .slice(0, 4),
        ),
      )
      .catch(() => setRecent([]));
  }, [cwd]);
  return (
    <div className="flex min-h-full flex-col items-center justify-center px-6 py-10">
      <img
        src={miraLogo}
        alt=""
        className="size-11 rounded-full object-contain opacity-90"
        draggable={false}
      />
      {/* The project name is part of the sentence, and clicking it is how
          you change it: a dotted underline says "this is a control"
          without needing a control to look like. */}
      <h2 className="mt-4 text-[19px] font-medium tracking-tight text-foreground">
        What should we build in{' '}
        <button
          type="button"
          onClick={() => setSwitcherOpen(true)}
          title={`${cwd} — switch project`}
          className="underline decoration-dotted decoration-1 underline-offset-[5px] transition-colors hover:text-mira-blue"
        >
          {project}
        </button>
        ?
      </h2>
      <p className="mt-1.5 max-w-[46ch] text-center text-[13px] leading-relaxed text-muted-foreground">
        Ask Mira anything, or pick up where you left off in this folder.
      </p>

      {/* Suggestions as inline chips rather than a grid of bordered boxes.
          Four identical outlined rectangles read as a form, not as
          suggestions; chips wrap, scale down to one column on a narrow pane,
          and don't draw a grid of lines across the middle of the screen. */}
      <div className="mt-7 flex max-w-2xl flex-wrap items-center justify-center gap-1.5">
        {STARTER_PROMPTS.map((p) => (
          <button
            key={p}
            type="button"
            onClick={() => onPrompt(p)}
            className="rounded-full border border-border/70 elev-card dark:bg-fg/[0.03] px-3 py-1.5 text-[12.5px] text-muted-foreground transition-colors hover:border-border dark:hover:bg-fg/[0.07] hover:text-foreground"
          >
            {p}
          </button>
        ))}
      </div>

      {recent.length > 0 && (
        <div className="mt-8 w-full max-w-lg">
          <div className="mb-1.5 text-[11px] font-semibold uppercase tracking-widest text-muted-foreground/50">
            Recent in this folder
          </div>
          <div className="flex flex-col">
            {recent.map((r) => (
              <button
                key={r.id}
                type="button"
                onClick={() => onOpenSession(r.id)}
                className="group flex items-center gap-3 rounded-md px-2 py-1.5 text-left text-[13px] transition-colors hover:bg-fg/[0.04]"
              >
                <span className="min-w-0 flex-1 truncate text-foreground/75 group-hover:text-foreground">
                  {r.title || r.first_user_message || 'Untitled'}
                </span>
                <span className="shrink-0 text-[11.5px] tabular-nums text-muted-foreground/50">
                  {timeAgo(r.updated_at)}
                </span>
              </button>
            ))}
          </div>
        </div>
      )}

      <ProjectSwitcher
        open={switcherOpen}
        onClose={() => setSwitcherOpen(false)}
        cwd={cwd}
        onPicked={onSwitchProject}
        onNewProject={onNewProject}
      />
    </div>
  );
}

export function timeAgo(ts: number): string {
  // Session timestamps are epoch seconds or ms depending on the store.
  const ms = ts < 1e12 ? ts * 1000 : ts;
  const mins = Math.round((Date.now() - ms) / 60000);
  if (mins < 1) return 'just now';
  if (mins < 60) return `${mins}m ago`;
  const hrs = Math.round(mins / 60);
  if (hrs < 24) return `${hrs}h ago`;
  return `${Math.round(hrs / 24)}d ago`;
}

/** Placeholder view rendered for sidebar entries that don't have a real
 *  panel yet (Pull request, Scheduled). Keeps the shell responsive while
 *  we build out the actual features. */
export function ComingSoon({ label }: { label: string }) {
  return (
    <div className="flex flex-1 min-h-0 flex-col items-center justify-center gap-2 text-muted-foreground">
      <div className="text-[18px] font-medium text-foreground/80">{label}</div>
      <div className="text-[13px]">Coming soon.</div>
    </div>
  );
}
