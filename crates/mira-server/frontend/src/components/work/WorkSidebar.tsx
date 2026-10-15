import { cn } from '@/lib/utils';
import {
  CheckCheck,
  CircleAlert,
  CircleCheck,
  CircleDashed,
  CircleDot,
  CircleX,
  Eye,
  GitBranch,
  GitMerge,
  Loader,
  Plus,
} from 'lucide-react';
import { PREF_KEYS, useStringPref } from '../../lib/prefs';
import { workShelves, type CiState, type WorkItem, type WorkState } from '../../lib/workItems';
import { AgentIcon, ModelIcon } from '../AgentIcon';
import { basename } from '../sidebar/groups';
import { Empty, NavItem, ShelfHeader, SidebarFolderIcon } from '../sidebar/navigation';
import { agentName, PR_MARK, SessionStatus, timeAgo } from '../sidebar/sessionRows';
import { Collapse } from '../ui/Collapse';

const SHELF: Record<WorkState, { label: string; icon: React.ReactNode }> = {
  'needs-you': { label: 'Needs you', icon: <CircleAlert className="size-3 shrink-0 text-amber-500" /> },
  'in-progress': { label: 'In progress', icon: <Loader className="size-3 shrink-0" /> },
  review: { label: 'In review', icon: <Eye className="size-3 shrink-0" /> },
  ready: { label: 'Ready to merge', icon: <GitMerge className="size-3 shrink-0" /> },
  done: { label: 'Done', icon: <CheckCheck className="size-3 shrink-0" /> },
};

/** The sidebar's Work tab: one row per deliverable, shelved by where it
 *  stands. Done starts collapsed, like Settled on the Chat side. */
export function WorkSidebar({
  items,
  activeKey,
  onPick,
  onStartWork,
}: {
  items: WorkItem[];
  activeKey: string | null;
  onPick: (key: string) => void;
  onStartWork: () => void;
}) {
  const [closedPref, setClosedPref] = useStringPref(PREF_KEYS.workShelvesClosed, 'done');
  const closed = new Set(closedPref.split(',').filter(Boolean));
  const toggle = (state: WorkState) => {
    const next = new Set(closed);
    if (next.has(state)) next.delete(state);
    else next.add(state);
    setClosedPref([...next].join(','));
  };
  return (
    <div className="min-h-0 flex-1 overflow-y-auto px-1.5 pb-2">
      <nav className="flex flex-col gap-0.5 px-0.5">
        <NavItem icon={<Plus className="size-3.5" />} onClick={onStartWork}>
          Start work
        </NavItem>
      </nav>
      {items.length === 0 && <Empty>No work yet. Start from an issue.</Empty>}
      {workShelves(items).map(([state, list]) =>
        list.length === 0 ? null : (
          <div key={state} className="mt-4 flex flex-col gap-0.5 px-0.5">
            <ShelfHeader
              icon={SHELF[state].icon}
              label={SHELF[state].label}
              count={list.length}
              open={!closed.has(state)}
              onToggle={() => toggle(state)}
            />
            <Collapse open={!closed.has(state)}>
              <div className="flex flex-col gap-0.5 pl-1">
                {list.map((item) => (
                  <WorkRow
                    key={item.key}
                    item={item}
                    active={item.key === activeKey}
                    onPick={() => onPick(item.key)}
                  />
                ))}
              </div>
            </Collapse>
          </div>
        ),
      )}
    </div>
  );
}

/** One deliverable, laid out like a chat row: what it is on top, where it
 *  stands below, and who's on it plus its diff and PR at the right. */
export function WorkRow({
  item,
  active,
  onPick,
}: {
  item: WorkItem;
  active: boolean;
  onPick: () => void;
}) {
  const running = item.sessions.some((s) => s.running);
  const diff = item.meta.diff;
  const project = basename(item.cwd);
  return (
    <div
      role="button"
      tabIndex={0}
      onClick={onPick}
      onKeyDown={(e) => {
        if (e.key === 'Enter' || e.key === ' ') {
          e.preventDefault();
          onPick();
        }
      }}
      className={cn(
        'group grid w-full cursor-pointer grid-cols-[1fr_auto] items-center gap-1.5 rounded-lg px-2 py-1.5 transition-colors',
        active ? 'sidebar-active bg-fg/[0.1] text-foreground' : 'text-foreground/90 hover:bg-fg/[0.05] hover:text-foreground',
      )}
    >
      <span className="grid min-w-0 grid-cols-[16px_minmax(0,1fr)_auto] items-center gap-2 text-left">
        <WorkKindIcon item={item} />
        <span className="min-w-0">
          <span className="flex min-w-0 items-center gap-1.5">
            <span className={cn('min-w-0 truncate text-[13.5px] leading-4', active ? 'font-semibold text-foreground' : 'font-medium text-foreground/90')}>
              {item.title}
            </span>
          </span>
          <span className="mt-0.5 flex min-w-0 items-center gap-1 text-[10.5px] leading-[15px]">
            {item.reason ? (
              <span className="min-w-0 truncate text-amber-700 dark:text-amber-400">{item.reason}</span>
            ) : running ? (
              <span className="shrink-0 text-mira-blue">Working</span>
            ) : null}
            {(item.reason || running) && <span className="shrink-0 text-muted-foreground/35">·</span>}
            <span className="flex shrink-0 items-center gap-1 text-muted-foreground/65">
              <SidebarFolderIcon name={project} open={false} className="size-3" />
              {project}
            </span>
            {!item.reason && item.branch && (
              <>
                <span className="shrink-0 text-muted-foreground/35">·</span>
                <span className="truncate text-muted-foreground/60">{item.branch}</span>
              </>
            )}
          </span>
        </span>
        <span className="flex flex-col items-end gap-0.5 self-start">
          <span className="flex items-center gap-1">
            <AgentStack agents={item.agents} model={item.sessions[0]?.model} />
            <PrMark item={item} />
          </span>
          {diff ? (
            <span className="text-[10.5px] leading-[15px] tabular-nums">
              <span className="text-emerald-600 dark:text-emerald-400">+{diff.added}</span>{' '}
              <span className="text-red-600/80 dark:text-red-400/80">−{diff.removed}</span>
            </span>
          ) : (
            <span className="text-[10.5px] leading-[15px] tabular-nums text-muted-foreground/50">{timeAgo(item.updated_at)}</span>
          )}
        </span>
      </span>
      <span className="flex size-5 items-center justify-center">
        <SessionStatus running={running} unread={false} waiting={item.state === 'needs-you'} />
      </span>
    </div>
  );
}

/** Issue-started work shows the issue mark; the rest show their branch. */
function WorkKindIcon({ item }: { item: WorkItem }) {
  if (item.meta.issue) {
    return (
      <span title={`Issue #${item.meta.issue.number}`} className="flex justify-self-center">
        <CircleDot className={cn('size-3.5', item.state === 'done' ? 'text-violet-500 dark:text-violet-400' : 'text-emerald-600 dark:text-emerald-400')} />
      </span>
    );
  }
  return <GitBranch className="size-3.5 justify-self-center text-muted-foreground/60" aria-label="Branch" />;
}

/** The agents on it, overlapped; Mira's own chats show the model's mark. */
export function AgentStack({ agents, model, max = 3 }: { agents: (string | null)[]; model?: string; max?: number }) {
  const shown = agents.slice(0, max);
  return (
    <span className="flex items-center -space-x-1" title={agents.map((a) => (a ? agentName(a) : 'Mira')).join(', ')}>
      {shown.map((a) => (
        <span key={a ?? 'mira'} className="flex rounded-[5px] bg-panel ring-1 ring-panel">
          {a ? <AgentIcon kind={a} name={agentName(a)} size="xs" tile={false} /> : <ModelIcon model={model} size="xs" />}
        </span>
      ))}
      {agents.length > max && <span className="pl-1.5 text-[10px] text-muted-foreground/70">+{agents.length - max}</span>}
    </span>
  );
}

function PrMark({ item }: { item: WorkItem }) {
  if (!item.pr) return null;
  const { Icon, tone, label } = PR_MARK[item.pr.state];
  return (
    <span className="relative flex" title={`PR #${item.pr.number} · ${label}${item.meta.ci ? ` · checks ${item.meta.ci}` : ''}`}>
      <Icon className={cn('size-3.5', tone)} />
      {item.meta.ci && item.pr.state === 'open' && <CiDot state={item.meta.ci} className="absolute -bottom-0.5 -right-0.5" />}
    </span>
  );
}

export function CiDot({ state, className }: { state: CiState; className?: string }) {
  return (
    <span
      className={cn(
        'size-1.5 rounded-full ring-1 ring-panel',
        state === 'passing' ? 'bg-emerald-500' : state === 'failing' ? 'bg-red-500' : 'bg-amber-400',
        className,
      )}
    />
  );
}

export function CiIcon({ state, className }: { state: CiState; className?: string }) {
  if (state === 'passing') return <CircleCheck className={cn('text-emerald-600 dark:text-emerald-400', className)} />;
  if (state === 'failing') return <CircleX className={cn('text-red-600 dark:text-red-400', className)} />;
  return <CircleDashed className={cn('animate-spin text-amber-500 [animation-duration:3s]', className)} />;
}
