import { Tip } from '../ui/Tip';
import { useSelector } from '@tanstack/react-store';
import { Columns2, List, WrapText, Pilcrow } from 'lucide-react';
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
  const owner = useSelector(runtime.sessionState, state => state.active ?? state.selected);
  const selected = scope ? summaries.findIndex(turn => turn.text === scope.summary.text && turn.occurrence === scope.summary.occurrence) : -1;
  return <select aria-label="Changes source" className="h-7 min-w-0 max-w-48 rounded-full border border-border/60 bg-secondary px-2 text-xs text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring" value={scope ? selected < 0 ? 'snapshot' : String(selected) : 'changes'} onChange={event => {
    if (event.target.value === 'changes') setScope(null);
    else if (owner && summaries[Number(event.target.value)]) setScope({ session: owner, summary: summaries[Number(event.target.value)] });
  }}>
    <option value="changes">Changes · uncommitted</option>
    {scope && selected < 0 && <option value="snapshot">Selected turn</option>}
    {summaries.map((turn, index) => ({ turn, index })).reverse().map(({turn,index}) => <option key={`${turn.text}:${turn.occurrence}`} value={String(index)}>{turn.text === latest?.text && turn.occurrence === latest.occurrence ? 'Latest turn' : `Turn edits · ${turn.text.slice(0, 48)}`}</option>)}
  </select>;
}

export function ReviewDisplayActions({ narrow = false }: { narrow?: boolean }) {
  const [layout, setLayout] = useStringPref(PREF_KEYS.diffLayout, 'unified');
  const [wrap, setWrap] = useBoolPref(PREF_KEYS.diffWrap, false);
  const [numbers, setNumbers] = useBoolPref(PREF_KEYS.diffLineNumbers, true);
  const action = 'flex size-7 items-center justify-center rounded-full text-muted-foreground transition-colors disabled:opacity-40 hover:bg-accent hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring';
  return <div className="flex shrink-0 items-center gap-1" role="group" aria-label="Diff display">
    <div className="flex rounded-full bg-secondary p-0.5">
      <Tip label="Unified diff" side="bottom" align="start"><button type="button" aria-label="Unified diff" aria-pressed={layout !== 'split' || narrow} onClick={() => setLayout('unified')} className={cn(action,(layout !== 'split' || narrow) && 'bg-accent text-foreground')}><List className="size-3.5" /></button></Tip>
      <Tip label="Side by side diff" hint={narrow ? "Widen the panel to at least 600px to compare both sides." : undefined} side="bottom" align="end"><button type="button" disabled={narrow} aria-label="Side by side diff" aria-pressed={layout === 'split' && !narrow} onClick={() => setLayout('split')} className={cn(action,layout === 'split' && !narrow && 'bg-accent text-foreground')}><Columns2 className="size-3.5" /></button></Tip>
    </div>
    <Tip label={wrap ? "Stop wrapping lines" : "Wrap lines"} side="bottom" align="end"><button type="button" aria-label="Wrap lines" aria-pressed={wrap} onClick={() => setWrap(!wrap)} className={cn(action,'bg-secondary',wrap && 'bg-accent text-foreground')}><WrapText className="size-3.5" /></button></Tip>
    <Tip label={numbers ? "Hide line numbers" : "Show line numbers"} side="bottom" align="end"><button type="button" aria-label="Line numbers" aria-pressed={numbers} onClick={() => setNumbers(!numbers)} className={cn(action,'bg-secondary',numbers && 'bg-accent text-foreground')}><Pilcrow className="size-3.5" /></button></Tip>
  </div>;
}
