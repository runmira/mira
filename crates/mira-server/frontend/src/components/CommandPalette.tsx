/**
 * ⌘K — one box to reach anything: app actions, settings pages and past
 * chats. Actions are supplied by the app (so each one runs exactly what its
 * button or shortcut would); chats are fetched when the palette opens.
 */
import { useEffect, useState, type ComponentType } from 'react';
import { Dialog, DialogContent, DialogTitle } from '@/components/ui/dialog';
import {
  Command,
  CommandEmpty,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandList,
} from '@/components/ui/command';
import { MessageSquare } from 'lucide-react';
import { listSessions } from '../api';
import type { SessionSummary } from '../types';
import { AgentIcon } from './AgentIcon';

export type PaletteAction = {
  id: string;
  label: string;
  group: string;
  icon: ComponentType<{ className?: string }>;
  /** Shortcut to show on the right, already formatted (`⌘J`). */
  shortcut?: string | null;
  /** Extra words to match on. */
  keywords?: string[];
  run: () => void;
};

function timeAgo(ts: number): string {
  // Session timestamps are epoch seconds or ms depending on the store.
  const secs = ts < 1e12 ? ts : ts / 1000;
  const s = Math.max(0, Math.floor(Date.now() / 1000 - secs));
  if (s < 60) return 'now';
  if (s < 3600) return `${Math.floor(s / 60)}m`;
  if (s < 86400) return `${Math.floor(s / 3600)}h`;
  return `${Math.floor(s / 86400)}d`;
}

export function CommandPalette({
  open,
  onOpenChange,
  actions,
  onOpenSession,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  actions: PaletteAction[];
  onOpenSession: (id: string) => void;
}) {
  const [sessions, setSessions] = useState<SessionSummary[]>([]);
  useEffect(() => {
    if (!open) return;
    listSessions()
      .then((all) =>
        setSessions(
          all
            .filter((s) => s.message_count > 0)
            .sort((a, b) => b.updated_at - a.updated_at)
            .slice(0, 40),
        ),
      )
      .catch(() => setSessions([]));
  }, [open]);

  const groups = new Map<string, PaletteAction[]>();
  for (const a of actions) {
    const list = groups.get(a.group) ?? [];
    list.push(a);
    groups.set(a.group, list);
  }

  const run = (fn: () => void) => {
    onOpenChange(false);
    // Let the dialog close (and hand focus back) before the action moves it.
    requestAnimationFrame(fn);
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="top-[18%] max-w-xl translate-y-0 gap-0 overflow-hidden p-0 [&>button:last-child]:hidden">
        <DialogTitle className="sr-only">Command palette</DialogTitle>
        <Command loop className="bg-transparent">
          <CommandInput placeholder="Type a command or search chats…" autoFocus />
          <CommandList className="max-h-[min(26rem,60vh)]">
            <CommandEmpty>Nothing matches.</CommandEmpty>
            {[...groups].map(([group, items]) => (
              <CommandGroup key={group} heading={group}>
                {items.map((a) => (
                  <CommandItem
                    key={a.id}
                    value={`${a.label} ${(a.keywords ?? []).join(' ')}`}
                    onSelect={() => run(a.run)}
                  >
                    <a.icon className="size-4 shrink-0 text-muted-foreground" />
                    <span className="min-w-0 flex-1 truncate">{a.label}</span>
                    {a.shortcut && (
                      <kbd className="shrink-0 rounded border border-border/70 px-1.5 py-px font-sans text-[10.5px] text-muted-foreground">
                        {a.shortcut}
                      </kbd>
                    )}
                  </CommandItem>
                ))}
              </CommandGroup>
            ))}
            {sessions.length > 0 && (
              <CommandGroup heading="Chats">
                {sessions.map((s) => {
                  const title = s.title || s.first_user_message || 'Untitled';
                  return (
                    <CommandItem
                      key={s.id}
                      value={`chat ${title} ${s.id}`}
                      onSelect={() => run(() => onOpenSession(s.id))}
                    >
                      {s.agent_driver ? (
                        <AgentIcon kind={s.agent_driver} name={s.agent_driver} size="sm" tile={false} className="shrink-0" />
                      ) : (
                        <MessageSquare className="size-4 shrink-0 text-muted-foreground" />
                      )}
                      <span className="min-w-0 flex-1 truncate">{title}</span>
                      <span className="shrink-0 text-[11px] tabular-nums text-muted-foreground/60">
                        {timeAgo(s.updated_at)}
                      </span>
                    </CommandItem>
                  );
                })}
              </CommandGroup>
            )}
          </CommandList>
          <div className="flex items-center gap-3 border-t border-border/60 px-3 py-1.5 text-[10.5px] text-muted-foreground/70">
            <span><kbd className="font-sans">↑↓</kbd> move</span>
            <span><kbd className="font-sans">↵</kbd> open</span>
            <span><kbd className="font-sans">esc</kbd> close</span>
          </div>
        </Command>
      </DialogContent>
    </Dialog>
  );
}
