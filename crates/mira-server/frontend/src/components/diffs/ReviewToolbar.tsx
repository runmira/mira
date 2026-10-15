import { reviewTurnSources } from '../../lib/reviewSources';
import { openTurnReview } from '../../lib/openTurnReview';
import { useState } from 'react';
import { Menu, MenuItem } from '../ui/Menu';
import { Tip } from '../ui/Tip';
import { useSelector } from '@tanstack/react-store';
import { AlignLeft, ChevronDown, ChevronLeft, ChevronRight, Columns2, List, ListOrdered, Text, WrapText } from 'lucide-react';
import { useAppRuntime, useWorkspaceUiField } from '../../state/RuntimeProvider';
import { selectActiveSession } from '../../state/runtimeStore';
import { PREF_KEYS, useBoolPref, useStringPref } from '../../lib/prefs';
import { latestEditedTurn } from '../context/LastTurnChanges';
import { cn } from '@/lib/utils';

export function ReviewSourceSelector() {
  const runtime = useAppRuntime();
  const summaries = useSelector(runtime.sessionState, state => selectActiveSession(state).turnDiffs);
  const entries = useSelector(runtime.sessionState, state => selectActiveSession(state).entries);
  const latest = latestEditedTurn(entries, summaries);
  const [scope, setScope] = useWorkspaceUiField('reviewTurn');
  const [open, setOpen] = useState(false);
  const [turnMenu, setTurnMenu] = useState(false);
  const [search, setSearch] = useState('');
  const available = reviewTurnSources(entries,summaries).map(source => ({...source,turn:source.summary}));
  const filtered = available.filter(source => `${source.label} ${source.turn.text} ${source.turn.files.map(file=>file.path).join(' ')}`.toLowerCase().includes(search.toLowerCase()));
  const isSelected = (turn: typeof summaries[number]) => !!scope && (scope.turnKey ? available.find(source => source.turn === turn)?.key === scope.turnKey : scope.summary.text === turn.text && scope.summary.occurrence === turn.occurrence);
  const isLatest = !!latest && isSelected(latest);
  const label = scope ? isLatest ? 'Latest turn' : available.find(item => isSelected(item.turn))?.label ?? 'Selected turn' : 'Changes';
  const choose = (turn: typeof summaries[number]) => openTurnReview(runtime,turn);
  const closeMenu = (value: boolean) => { setOpen(value); if (!value) { setTurnMenu(false); setSearch(''); } };
  return <div className="relative min-w-0 max-w-48">
    <button type="button" aria-label="Changes source" aria-haspopup="menu" aria-expanded={open} onClick={() => setOpen(value => !value)} onKeyDown={event => {
      if (event.key === 'ArrowDown' || event.key === 'ArrowUp') { event.preventDefault(); setOpen(true); }
    }} className="flex h-7 max-w-full items-center gap-3 rounded-full border border-border/60 bg-secondary pl-3 pr-3 text-xs text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring">
      <span className="truncate">{label}</span><ChevronDown aria-hidden="true" className="size-3 shrink-0 text-muted-foreground" />
    </button>
    <Menu open={open} onOpenChange={closeMenu} label={turnMenu ? 'Turn changes menu' : 'Changes source menu'} align="start" minWidth={220} className="max-h-80 max-w-[calc(100vw-24px)] overflow-y-auto">
      {turnMenu ? <>
        <button type="button" role="menuitem" data-menu-item="Back" onClick={() => setTurnMenu(false)} onKeyDown={event => { if (event.key === 'ArrowLeft') { event.preventDefault(); setTurnMenu(false); } }} className="flex items-center gap-2 rounded-md px-2 py-2 text-xs text-muted-foreground hover:bg-accent focus-visible:bg-accent"><ChevronLeft className="size-3.5" />Changes source</button>
        <input aria-label="Search changed turns" placeholder="Search turns or files…" value={search} onChange={event => setSearch(event.target.value)} onKeyDown={event => { if (!['ArrowDown','ArrowUp','Escape'].includes(event.key)) event.stopPropagation(); }} className="mx-1 mb-1 rounded-md border border-border bg-background px-2 py-2 text-xs" />
        {filtered.length === 0 && <p role="status" className="px-2 py-3 text-xs text-muted-foreground">No matching turns.</p>}
        {filtered.map(({turn,label,timestamp}) => <MenuItem key={`${turn.text}:${turn.occurrence}`} label={label} hint={`${turn.files.length} files${timestamp ? ` · ${new Date(timestamp * 1000).toLocaleString(undefined,{month:'short',day:'numeric',hour:'numeric',minute:'2-digit'})}` : ''} · ${turn.text}`} active={isSelected(turn)} onSelect={() => choose(turn)} />)}
      </> : <>
        <MenuItem label="Changes" hint="Uncommitted session changes" active={!scope} onSelect={() => setScope(null)} />
        <MenuItem label="Latest turn" disabled={!latest} hint="Most recent turn with file changes" active={isLatest} onSelect={() => { if (latest) choose(latest); }} />
        <button type="button" role="menuitem" data-menu-item="Turns" aria-haspopup="menu" aria-expanded={turnMenu} disabled={!available.length} onClick={() => setTurnMenu(true)} onKeyDown={event => { if (event.key === 'ArrowRight') { event.preventDefault(); setTurnMenu(true); } }} className="flex items-center justify-between gap-3 rounded-md px-2 py-2 text-left text-xs font-medium hover:bg-accent focus-visible:bg-accent disabled:opacity-40"><span>Turns</span><ChevronRight className="size-3.5" /></button>
      </>}

    </Menu>
  </div>;
}

export function ReviewDisplayActions({ narrow = false }: { narrow?: boolean }) {
  const [layout, setLayout] = useStringPref(PREF_KEYS.diffLayout, 'unified');
  const [wrap, setWrap] = useBoolPref(PREF_KEYS.diffWrap, false);
  const [numbers, setNumbers] = useBoolPref(PREF_KEYS.diffLineNumbers, true);
  const action = 'flex size-7 items-center justify-center rounded-full text-muted-foreground transition-colors disabled:opacity-40 hover:bg-accent hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring';
  const active = 'bg-mira-blue/15 text-mira-blue ring-1 ring-inset ring-mira-blue/30 hover:bg-mira-blue/20 hover:text-mira-blue';
  return <div className="flex shrink-0 items-center gap-1" role="group" aria-label="Diff display">
    <div className="flex rounded-full bg-secondary p-0.5">
      <Tip label="Unified diff" side="bottom" align="start"><button type="button" aria-label="Unified diff" aria-pressed={layout !== 'split' || narrow} onClick={() => setLayout('unified')} className={cn(action,(layout !== 'split' || narrow) && active)}><List className="size-3.5" /></button></Tip>
      <Tip label="Side by side diff" hint={narrow ? "Widen the panel to at least 600px to compare both sides." : undefined} side="bottom" align="end"><button type="button" disabled={narrow} aria-label="Side by side diff" aria-pressed={layout === 'split' && !narrow} onClick={() => setLayout('split')} className={cn(action,layout === 'split' && !narrow && active)}><Columns2 className="size-3.5" /></button></Tip>
    </div>
    <Tip label={wrap ? "Stop wrapping lines" : "Wrap lines"} side="bottom" align="end"><button type="button" aria-label="Wrap lines" aria-pressed={wrap} onClick={() => setWrap(!wrap)} className={cn(action,'bg-secondary',wrap && active)}>{wrap ? <WrapText className="size-3.5" /> : <AlignLeft className="size-3.5" />}</button></Tip>
    <Tip label={numbers ? "Hide line numbers" : "Show line numbers"} side="bottom" align="end"><button type="button" aria-label="Line numbers" aria-pressed={numbers} onClick={() => setNumbers(!numbers)} className={cn(action,'bg-secondary',numbers && active)}>{numbers ? <ListOrdered className="size-3.5" /> : <Text className="size-3.5" />}</button></Tip>
  </div>;
}
