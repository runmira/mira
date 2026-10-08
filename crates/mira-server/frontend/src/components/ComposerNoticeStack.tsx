import { useId, useRef, useState, type ReactNode } from 'react';
import { AnimatePresence, m, useReducedMotion } from 'framer-motion';
import { ChevronDown, CircleHelp, Download, ShieldCheck, WifiOff, ClipboardList, X } from 'lucide-react';
import { cn } from '../lib/utils';
export type ComposerNotice = { id: string; kind: 'question' | 'plan' | 'approval' | 'connection' | 'update'; title: string; detail?: string; content?: ReactNode };
const icons = { question: CircleHelp, plan: ClipboardList, approval: ShieldCheck, connection: WifiOff, update: Download };
export function ComposerNoticeStack({ items, selected, onSelect }: { items: ComposerNotice[]; selected: string; onSelect: (id: string) => void }) {
  const [expanded, setExpanded] = useState(false);
  const trigger = useRef<HTMLButtonElement>(null);
  const panel = useRef<HTMLDivElement>(null);
  const id = useId(); const reduced = useReducedMotion();
  const active = items.find(item => item.id === selected) ?? items[0];
  if (!active) return null;
  const Icon = icons[active.kind];
  function close() { setExpanded(false); trigger.current?.focus(); }
  return <div className="relative mx-1 mb-1" onKeyDown={event => { if (expanded && event.key === 'Escape') { event.preventDefault(); event.stopPropagation(); close(); } }}>
    <button ref={trigger} type="button" aria-expanded={expanded} aria-controls={id} disabled={items.length < 2} onClick={() => { setExpanded(value => !value); if (!expanded) requestAnimationFrame(() => panel.current?.querySelector<HTMLButtonElement>('button')?.focus()); }} className="flex w-full items-center gap-2 rounded-xl border border-border bg-background px-3 py-2 text-left text-[12px] text-foreground disabled:opacity-100">
      <Icon className={cn('size-3.5 shrink-0', active.kind === 'connection' ? 'text-amber-700 dark:text-amber-400' : 'text-muted-foreground')} />
      <span className="min-w-0 flex-1 truncate font-medium">{active.title}</span>
      {items.length > 1 && <><span className="text-[11px] text-muted-foreground">{items.length - 1} more</span><ChevronDown className={cn('size-3.5 transition-transform', expanded && 'rotate-180')} /></>}
    </button>
    <AnimatePresence initial={false}>
      {expanded && <m.div ref={panel} id={id} initial={reduced ? false : { opacity: 0, height: 0 }} animate={{ opacity: 1, height: 'auto' }} exit={{ opacity: 0, height: 0 }} transition={{ duration: reduced ? 0 : .18 }} className="overflow-hidden">
        <div className="mt-1 max-h-52 space-y-1 overflow-y-auto rounded-xl border border-border bg-background p-1.5">
          <div className="flex items-center justify-between px-2 py-1 text-[11px] text-muted-foreground"><span>Waiting for your attention</span><button type="button" aria-label="Close notices" onClick={close}><X className="size-3.5" /></button></div>
          {items.map(item => { const ItemIcon = icons[item.kind]; return <button type="button" key={item.id} aria-pressed={item.id === selected} onClick={() => { onSelect(item.id); close(); }} className={cn('flex w-full items-start gap-2 rounded-lg px-2 py-2 text-left hover:bg-secondary focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-mira-blue', item.id === selected && 'bg-secondary/60')}><ItemIcon className="mt-0.5 size-3.5 shrink-0 text-muted-foreground" /><span className="min-w-0"><span className="block text-[12px] font-medium">{item.title}</span>{item.detail && <span className="mt-0.5 block truncate text-[11px] text-muted-foreground">{item.detail}</span>}</span></button>; })}
        </div>
      </m.div>}
    </AnimatePresence>
  </div>;
}
