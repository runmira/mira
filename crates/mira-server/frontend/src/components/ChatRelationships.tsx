import { useEffect, useState } from 'react';
import { ArrowUpRight, ChevronRight, GitBranch, LoaderCircle } from 'lucide-react';
import { getChatRelationships, type ChatRelationshipView } from '../api';
import type { SessionSummary } from '../types';
import { cn } from '../lib/utils';
export function ChatRelationships({ sessionId, refreshKey, onOpen }: { sessionId: string; refreshKey: number; onOpen: (id: string) => void }) {
  const [view, setView] = useState<ChatRelationshipView>({ current: null, parent: null, children: [], has_more: false });
  const [limit, setLimit] = useState(20);
  const [open, setOpen] = useState(false);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    let live = true; const controller = new AbortController();
    const refresh = () => { void getChatRelationships(sessionId, limit, controller.signal).then(rows => { if (live) { setView(rows); setError(null); } }).catch(error => { if (live) setError(error.message); }); };
    refresh(); const timer = open ? setInterval(refresh, 10000) : undefined;
    return () => { live = false; controller.abort(); if (timer) clearInterval(timer); };
  }, [sessionId, refreshKey, open, limit]);
  const current = view.current;
  const parentId = current?.parent_id ?? current?.forked_from;
  const parent = view.parent;
  const children = view.children;
  if (!parentId && !children.length) return null;
  const label = (session: SessionSummary) => session.title || session.first_user_message || 'Untitled chat';
  return <div className="py-1 text-xs text-muted-foreground">
    <button className="flex items-center gap-2 py-1" aria-expanded={open} onClick={() => setOpen(value => !value)}><ChevronRight className={cn('size-3.5', open && 'rotate-90')} /><GitBranch className="size-3.5" />Related chats <span>{children.length + (parentId ? 1 : 0)}</span></button>
    {open && <div className="ml-7 flex flex-col gap-1">
      {parentId && <button className="flex items-center gap-2 py-1 text-left hover:text-foreground" onClick={() => onOpen(parentId)}><ArrowUpRight className="size-3.5" /><span className="truncate">{current?.parent_id ? 'Parent' : 'Forked from'} · {parent ? label(parent) : parentId}</span><span className="ml-auto">{parent?.agent_driver ?? parent?.model ?? ''}</span></button>}
      {current?.forked_at && <p className="truncate opacity-70" title={current.forked_at}>At: {current.forked_at}</p>}
      {children.map(child => <button key={child.id} className="flex items-center gap-2 py-1 text-left hover:text-foreground" onClick={() => onOpen(child.id)}><GitBranch className="size-3.5" /><span className="min-w-0 flex-1 truncate">{child.parent_id ? 'Agent' : 'Fork'} · {label(child)}</span><span className="max-w-32 truncate">{child.agent_driver ?? child.model}</span>{child.running && <LoaderCircle className="size-3 animate-spin" />}<span>{child.archived ? 'Archived' : child.running ? 'Running' : 'Idle'}</span></button>)}
      {view.has_more && <button className="py-1 text-left" onClick={() => setLimit(value => Math.min(200, value + 20))} disabled={limit >= 200}>{limit >= 200 ? 'Showing the latest 200 related chats' : 'Show more related chats'}</button>}
      {error && <p role="alert">Could not refresh related chats: {error}</p>}
    </div>}
  </div>;
}
