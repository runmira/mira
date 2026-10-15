import { Dialog, DialogContent, DialogTitle } from '@/components/ui/dialog';
import { cn } from '@/lib/utils';
import { CircleDot, GitBranch, Search } from 'lucide-react';
import { useState } from 'react';
import { issueTone, type IssueOption } from '../../lib/issueMentions';
import { AgentIcon } from '../AgentIcon';
import { agentName } from '../sidebar/sessionRows';

/** Start work from an issue: pick one (same look as the composer's `#`
 *  picker), see the branch it will get, and choose who works on it. */
export function StartWork({
  open,
  onOpenChange,
  repo,
  issues,
  agents,
  onStart,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  repo: string;
  issues: IssueOption[];
  agents: string[];
  onStart: (issue: IssueOption, agent: string) => void;
}) {
  const [query, setQuery] = useState('');
  const [activeIdx, setActiveIdx] = useState(0);
  const [agent, setAgent] = useState(agents[0] ?? 'claude-code');
  const q = query.trim().toLowerCase().replace(/^#/, '');
  const items = issues.filter((i) => i.kind === 'issue' && (!q || String(i.number).startsWith(q) || i.title.toLowerCase().includes(q)));
  const picked = items[Math.min(activeIdx, items.length - 1)];
  const start = (issue: IssueOption | undefined) => {
    if (!issue) return;
    onStart(issue, agent);
    onOpenChange(false);
    setQuery('');
  };
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent showClose={false} className="max-w-xl gap-0 overflow-hidden p-0">
        <DialogTitle className="sr-only">Start work from an issue</DialogTitle>
        <label className="flex items-center gap-2 border-b border-fg/[0.06] px-4 py-3">
          <Search className="size-4 shrink-0 text-muted-foreground/70" />
          <input
            autoFocus
            value={query}
            onChange={(e) => {
              setQuery(e.target.value);
              setActiveIdx(0);
            }}
            onKeyDown={(e) => {
              if (e.key === 'ArrowDown') {
                e.preventDefault();
                setActiveIdx((i) => Math.min(i + 1, items.length - 1));
              } else if (e.key === 'ArrowUp') {
                e.preventDefault();
                setActiveIdx((i) => Math.max(i - 1, 0));
              } else if (e.key === 'Enter') {
                e.preventDefault();
                start(picked);
              }
            }}
            placeholder="Start work from an issue…"
            className="min-w-0 flex-1 bg-transparent text-[14px] text-foreground outline-none placeholder:text-muted-foreground/60"
          />
        </label>
        <div className="flex items-center gap-2 px-4 pb-1 pt-2.5 text-[12px]">
          <CircleDot className="size-3.5 shrink-0 text-muted-foreground/70" />
          <span className="truncate text-foreground/80">{repo}</span>
          <span className="truncate text-muted-foreground/50">{q ? (/^\d+$/.test(q) ? `#${q}` : `matching “${query.trim()}”`) : 'Open issues'}</span>
        </div>
        <div className="max-h-[18rem] overflow-y-auto pb-1.5" role="listbox" aria-label="Issues">
          {items.length === 0 && <div className="px-4 py-3 text-[12.5px] text-muted-foreground/80">No open issues match.</div>}
          {items.map((option, i) => {
            const active = option === picked;
            return (
              <button
                key={option.number}
                type="button"
                role="option"
                aria-selected={active}
                onMouseEnter={() => setActiveIdx(i)}
                onClick={() => start(option)}
                className={cn('flex w-full items-center gap-2.5 px-4 py-1.5 text-left transition-colors', active ? 'bg-fg/[0.06]' : 'hover:bg-fg/[0.035]')}
              >
                <CircleDot className={cn('size-[15px] shrink-0', issueTone(option.state))} />
                <span className="w-11 shrink-0 font-mono text-[12px] tabular-nums text-muted-foreground/80">#{option.number}</span>
                <span className={cn('min-w-0 flex-1 truncate text-[13.5px] tracking-tight', active ? 'text-foreground' : 'text-foreground/90')}>{option.title}</span>
                {option.labels.slice(0, 2).map((label) => (
                  <span key={label.name} className="hidden max-w-[7rem] shrink-0 items-center gap-1 truncate rounded-full border border-fg/[0.08] px-1.5 text-[10.5px] text-muted-foreground sm:inline-flex">
                    <span className="size-1.5 shrink-0 rounded-full" style={{ backgroundColor: `#${label.color}` }} />
                    <span className="truncate">{label.name}</span>
                  </span>
                ))}
              </button>
            );
          })}
        </div>
        <div className="flex items-center gap-3 border-t border-fg/[0.06] px-4 py-2.5 text-[12px]">
          {picked ? (
            <span className="flex min-w-0 flex-1 items-center gap-1.5 text-muted-foreground">
              <GitBranch className="size-3.5 shrink-0" />
              <span className="truncate font-mono text-[11.5px] text-foreground/80">{branchFor(picked)}</span>
            </span>
          ) : (
            <span className="flex-1" />
          )}
          <span className="flex items-center gap-0.5 rounded-full border border-border/70 p-0.5">
            {agents.map((a) => (
              <button
                key={a}
                type="button"
                title={agentName(a)}
                aria-pressed={agent === a}
                onClick={() => setAgent(a)}
                className={cn('flex items-center gap-1 rounded-full px-2 py-0.5 transition-colors', agent === a ? 'bg-fg/[0.1] text-foreground' : 'text-muted-foreground hover:text-foreground')}
              >
                <AgentIcon kind={a} name={agentName(a)} size="xs" tile={false} />
                {agent === a && <span className="text-[11.5px]">{agentName(a)}</span>}
              </button>
            ))}
          </span>
          <button
            type="button"
            disabled={!picked}
            onClick={() => start(picked)}
            className="rounded-full bg-foreground px-3 py-1 text-[12px] font-medium text-background transition-opacity hover:opacity-90 disabled:opacity-40"
          >
            Start
          </button>
        </div>
      </DialogContent>
    </Dialog>
  );
}

/** The worktree branch an issue's work gets: `issue-N-short-title`. */
export function branchFor(issue: Pick<IssueOption, 'number' | 'title'>): string {
  const slug = issue.title.toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-|-$/g, '').split('-').slice(0, 4).join('-');
  return `issue-${issue.number}-${slug}`;
}
