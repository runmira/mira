/**
 * A Mira session mentioned in a transcript, as a chip with a hover
 * card — the way PR/issue (`GithubRef`) and link (`LinkRef`) mentions
 * show them. Works anywhere Markdown renders: anything rendered as
 * Markdown.
 *
 * The chip shows the session title once the preview lands (short id
 * until then); the hover card mirrors the sidebar's `SessionPeek`
 * card — title, engine, recency, folder, status, last exchange, and
 * facts — so a mention reads the same as the row it points at.
 *
 * Click opens the session in the main chat via the existing
 * `mira:open-session` event (see `useWorkspaceEffects`), so no
 * prop-drilling through every `Markdown` call site — same pattern as
 * `prPanel.ts`.
 */
import { useEffect, useState } from 'react';
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
import { Popover, PopoverContent, PopoverTrigger } from '@/components/ui/popover';
import { shortSessionId } from '../lib/sessionRefs';
import type { SessionSummary } from '../types';

type Preview = {
  title: string | null;
  last_user: string | null;
  last_assistant: string | null;
  last_tool: string | null;
  message_count: number;
};

const previewCache = new Map<string, Promise<Preview | null>>();
function loadPreview(id: string): Promise<Preview | null> {
  let p = previewCache.get(id);
  if (!p) {
    p = fetch(`/api/sessions/${encodeURIComponent(id)}/preview`, { cache: 'no-store' })
      .then((r) => (r.ok ? (r.json() as Promise<Preview>) : null))
      .catch(() => null);
    previewCache.set(id, p);
  }
  return p;
}

// One shared list fetch no matter how many chips are on screen; the
// card picks its own row out of it. `all` covers other folders;
// archived chats need their own flag, tried only on a miss.
let listCache: Promise<SessionSummary[] | null> | null = null;
async function loadList(archived: boolean): Promise<SessionSummary[] | null> {
  try {
    const params = new URLSearchParams({ _ts: String(Date.now()) });
    if (archived) params.set('archived', 'true');
    else params.set('all', 'true');
    const r = await fetch(`/api/sessions?${params.toString()}`, { cache: 'no-store' });
    if (!r.ok) return null;
    return (await r.json()) as SessionSummary[];
  } catch {
    return null;
  }
}
async function loadSummary(id: string): Promise<SessionSummary | null> {
  if (!listCache) listCache = loadList(false);
  const list = await listCache;
  const hit = list?.find((s) => s.id === id);
  if (hit || !list) return hit ?? null;
  const archived = await loadList(true);
  return archived?.find((s) => s.id === id) ?? null;
}

export function openSession(id: string): void {
  window.dispatchEvent(new CustomEvent<string>('mira:open-session', { detail: id }));
}

export function SessionRef({ id }: { id: string; children?: React.ReactNode }) {
  const [preview, setPreview] = useState<Preview | null | undefined>(undefined);
  const [summary, setSummary] = useState<SessionSummary | null | undefined>(undefined);
  const [open, setOpen] = useState(false);
  // The id alone reads as opaque hex, so the chip resolves to the
  // session title once data lands — the way the hover card already
  // does. Short id until then (and when untitled / missing), so the
  // chip never renders empty mid-stream.
  const title = summary?.title?.trim() || preview?.title?.trim() || null;
  const label = title ?? shortSessionId(id);

  useEffect(() => {
    let live = true;
    void loadPreview(id).then((p) => live && setPreview(p ?? null));
    void loadSummary(id).then((s) => live && setSummary(s ?? null));
    return () => {
      live = false;
    };
  }, [id]);

  return (
    <Popover open={open && preview !== undefined} onOpenChange={setOpen}>
      <PopoverTrigger asChild>
        <button
          type="button"
          title={title ? `${title} (${id})` : id}
          onMouseEnter={() => setOpen(true)}
          onMouseLeave={() => setOpen(false)}
          onClick={() => {
            setOpen(false);
            openSession(id);
          }}
          className="not-prose inline-flex max-w-[28ch] items-baseline gap-1 rounded-md bg-fg/[0.06] px-1.5 py-px align-baseline text-[0.92em] font-medium text-mira-blue no-underline transition-colors hover:bg-fg/[0.1]"
        >
          <MessagesSquare className="size-3.5 shrink-0 translate-y-[2px] text-mira-blue" />
          <span className="min-w-0 truncate">{label}</span>
        </button>
      </PopoverTrigger>
      {summary ? (
        <PeekCard session={summary} preview={preview} onHover={setOpen} />
      ) : preview ? (
        <SimpleCard id={id} preview={preview} onHover={setOpen} />
      ) : preview === null ? (
        <PopoverContent
          side="top"
          align="start"
          sideOffset={6}
          onOpenAutoFocus={(e) => e.preventDefault()}
          className="w-64 rounded-xl border border-border/60 bg-popover/95 p-3 text-[12px] text-muted-foreground backdrop-blur"
        >
          Session not found.
        </PopoverContent>
      ) : null}
    </Popover>
  );
}

/** The full card: same facts as the sidebar's `SessionPeek`. */
function PeekCard({
  session,
  preview,
  onHover,
}: {
  session: SessionSummary;
  preview: Preview | null | undefined;
  onHover: (v: boolean) => void;
}) {
  const agent = session.agent_driver ?? null;
  const engineName = agent ? agentLabel(agent) : prettyModel(session.model ?? '');
  return (
    <PopoverContent
      side="top"
      align="start"
      sideOffset={6}
      onMouseEnter={() => onHover(true)}
      onMouseLeave={() => onHover(false)}
      onOpenAutoFocus={(e) => e.preventDefault()}
      className="w-[340px] max-w-[calc(100vw-24px)] overflow-hidden rounded-xl border border-border bg-popover p-0 text-left shadow-xl shadow-shade/25 backdrop-blur"
    >
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
        {(session.needs_attention || session.running || session.failure_reason) && (
          <div
            className={
              session.needs_attention || session.failure_reason
                ? 'mt-2 inline-flex items-center gap-1.5 rounded-md bg-amber-500/12 px-2 py-0.5 text-[11px] font-medium text-amber-700 dark:text-amber-400'
                : 'mt-2 inline-flex items-center gap-1.5 rounded-md bg-mira-blue/10 px-2 py-0.5 text-[11px] font-medium text-mira-blue'
            }
          >
            {session.needs_attention || session.failure_reason ? (
              <>
                <CircleAlert className="size-3" />{' '}
                {session.failure_reason ?? session.attention_reason ?? 'Needs attention'}
              </>
            ) : (
              <>
                <Loader className="size-3 animate-spin" /> Working
              </>
            )}
          </div>
        )}
      </div>
      {session.failure_reason && (
        <div className="line-clamp-2 px-3.5 pb-2 text-[11px] leading-4 text-red-600 dark:text-red-400">
          {session.failure_reason}
        </div>
      )}
      <div className="space-y-2 border-t border-border/60 bg-fg/[0.02] px-3.5 py-2.5">
        {preview?.last_user || preview?.last_assistant ? (
          <>
            {preview.last_user && (
              <Line icon={User} label="You">
                <span className="line-clamp-2 text-foreground/85">{preview.last_user}</span>
              </Line>
            )}
            {preview.last_assistant && (
              <Line icon={agent ? Bot : Sparkles} label={engineName}>
                <span className="line-clamp-3 text-muted-foreground">{preview.last_assistant}</span>
              </Line>
            )}
          </>
        ) : (
          <div className="text-[12px] text-muted-foreground/70">
            {preview ? 'No preview available yet' : 'Loading…'}
          </div>
        )}
      </div>
      {(preview?.message_count || preview?.last_tool || session.worktree_branch || session.pr) && (
        <div className="flex items-center gap-3 border-t border-border/60 px-3.5 py-2 text-[11px] text-muted-foreground">
          {preview && preview.message_count > 0 && (
            <Meta icon={MessagesSquare} title="Messages">{preview.message_count}</Meta>
          )}
          {preview?.last_tool && (
            <Meta icon={Wrench} title="Last tool" mono>{preview.last_tool}</Meta>
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
          {session.pr && (
            <span className="inline-flex items-center gap-1 text-muted-foreground" title={session.pr.title}>
              <GitPullRequest className="size-3" aria-hidden />
              #{session.pr.number}
            </span>
          )}
        </div>
      )}
      <div className="border-t border-border/60 px-3.5 py-1.5 text-[11px] text-muted-foreground/70">
        Click to open this session
      </div>
    </PopoverContent>
  );
}

/** Fallback when the sessions list doesn't have this chat: the
 *  preview tail on its own. */
function SimpleCard({
  id,
  preview,
  onHover,
}: {
  id: string;
  preview: Preview;
  onHover: (v: boolean) => void;
}) {
  return (
    <PopoverContent
      side="top"
      align="start"
      sideOffset={6}
      onMouseEnter={() => onHover(true)}
      onMouseLeave={() => onHover(false)}
      onOpenAutoFocus={(e) => e.preventDefault()}
      className="w-80 rounded-xl border border-border/60 bg-popover/95 p-3 backdrop-blur"
    >
      <div className="line-clamp-2 text-[13px] font-semibold leading-[1.35] text-foreground">
        {preview.title?.trim() || 'Untitled'}
      </div>
      <div className="mt-1 flex items-center gap-2 text-[12px] text-muted-foreground">
        <span className="min-w-0 flex-1 truncate font-mono">{id}</span>
        {preview.message_count > 0 && (
          <span className="shrink-0">
            {preview.message_count} message{preview.message_count === 1 ? '' : 's'}
          </span>
        )}
      </div>
      {preview.last_user || preview.last_assistant ? (
        <div className="mt-2 space-y-1.5 border-t border-border/50 pt-2 text-[12px] leading-relaxed">
          {preview.last_user && (
            <div className="line-clamp-2 text-foreground/85">{preview.last_user}</div>
          )}
          {preview.last_assistant && (
            <div className="line-clamp-3 text-muted-foreground">{preview.last_assistant}</div>
          )}
        </div>
      ) : (
        <div className="mt-2 border-t border-border/50 pt-2 text-[12px] text-muted-foreground/70">
          No preview available yet
        </div>
      )}
      <div className="mt-2 text-[11px] text-muted-foreground/70">Click to open this session</div>
    </PopoverContent>
  );
}

type Icon = typeof Clock;

function Meta({ icon: I, title, mono, children }: { icon: Icon; title?: string; mono?: boolean; children: React.ReactNode }) {
  return (
    <span className="inline-flex min-w-0 items-center gap-1" title={title}>
      <I className="size-3 shrink-0 opacity-70" aria-hidden />
      <span className={mono ? 'max-w-[9rem] truncate font-mono' : 'max-w-[9rem] truncate'}>{children}</span>
    </span>
  );
}

function Line({ icon: I, label, children }: { icon: Icon; label: string; children: React.ReactNode }) {
  return (
    <div className="flex gap-2 text-[12px] leading-[1.45]">
      <I className="mt-[2px] size-3.5 shrink-0 text-muted-foreground/70" aria-label={label} />
      {children}
    </div>
  );
}

/* ---------- small helpers, kept in sync with SessionPeek ---------- */

function sessionLabel(s: SessionSummary): string {
  const raw = (s.title?.trim() || s.first_user_message || '').trim();
  if (!raw) return 'Untitled';
  return raw.length > 120 ? `${raw.slice(0, 120)}…` : raw;
}

function basename(p: string): string {
  const t = p.replace(/\/+$/, '');
  return t.slice(t.lastIndexOf('/') + 1) || t || 'folder';
}

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

function agentLabel(kind: string): string {
  const known: Record<string, string> = {
    'claude-code': 'Claude Code', codex: 'Codex', cursor: 'Cursor',
    grok: 'Grok', opencode: 'OpenCode', antigravity: 'Antigravity',
  };
  return known[kind] ?? kind;
}

function shortBranch(branch: string): string {
  const parts = branch.split('/');
  return parts[parts.length - 1] || branch;
}

function timeAgo(epochSeconds: number): string {
  const s = Math.max(0, Math.floor(Date.now() / 1000) - epochSeconds);
  if (s < 60) return 'now';
  if (s < 3600) return `${Math.floor(s / 60)}m`;
  if (s < 86400) return `${Math.floor(s / 3600)}h`;
  if (s < 86400 * 7) return `${Math.floor(s / 86400)}d`;
  if (s < 86400 * 30) return `${Math.floor(s / (86400 * 7))}w`;
  return new Date(epochSeconds * 1000).toLocaleDateString();
}
