import { useEffect, useMemo, useRef, useState } from 'react';
import { walkthroughCall } from '../api';
import { useOptionalRuntime } from '../state/RuntimeProvider';
import { buildFileDiffContentVersion, buildFileDiffIdentityKey, getRenderablePatch, resolveFileDiffPath } from '../lib/diffRender';
import { canRevertWalkthroughHunk, parseWalkthroughPr, restoreApprovals, reviewStorageKey, splitWalkthroughHunks, type WalkthroughPlan, type WalkthroughSource } from '../lib/walkthrough';
import { ReviewDiffView, type ReviewDiffFile } from './diffs/ReviewDiffView';
import { useDiffOptions } from '../lib/diffPrefs';

const button = 'rounded-md border border-border px-3 py-1.5 text-xs hover:bg-accent disabled:opacity-40';
const labels = { core: 'Core change', dependent: 'Depends on it', tests: 'Tests', incidental: 'Incidental edits' };

export function GuidedWalkthrough({ initialSource = { kind: 'session' }, onChanged, onClose }: {
  initialSource?: WalkthroughSource; onChanged: () => void; onClose: () => void;
}) {
  const runtime = useOptionalRuntime();
  const sessionId = runtime?.sessionState.get().active ?? runtime?.sessionState.get().selected ?? undefined;
  // Remount all state when the owning chat changes. Async work stays pinned to
  // the original session; responses cannot appear in the next chat's review.
  return <Walkthrough key={sessionId ?? 'standalone'} sessionId={sessionId} initialSource={initialSource} onChanged={onChanged} onClose={onClose} />;
}

function Walkthrough({ sessionId, initialSource, onChanged, onClose }: {
  sessionId?: string; initialSource: WalkthroughSource; onChanged: () => void; onClose: () => void;
}) {
  const [kind, setKind] = useState<WalkthroughSource['kind']>(initialSource.kind);
  const [input, setInput] = useState(initialSource.kind === 'branch' ? initialSource.range : initialSource.kind === 'pr' ? `${initialSource.owner}/${initialSource.repo}/${initialSource.number}` : 'main...HEAD');
  const [source, setSource] = useState<WalkthroughSource>(initialSource);
  const [plan, setPlan] = useState<WalkthroughPlan | null>(null);
  const [index, setIndex] = useState(0);
  const [hunk, setHunk] = useState(0);
  const [approved, setApproved] = useState<Set<string>>(new Set());
  const [reverted, setReverted] = useState<Set<string>>(new Set());
  const [notice, setNotice] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [question, setQuestion] = useState('');
  const [answers, setAnswers] = useState<Record<number, string>>({});
  const [asking, setAsking] = useState(false);
  const [confirm, setConfirm] = useState(false);
  const [incidentalOpen, setIncidentalOpen] = useState(false);
  const generation = useRef(0);
  const diffOptions = useDiffOptions();
  useEffect(() => () => { generation.current++; }, []);

  async function load() {
    const request = ++generation.current;
    setBusy(true); setError(null); setNotice(null); setReverted(new Set()); setPlan(null); setAnswers({}); setQuestion(''); setConfirm(false);
    try {
      const next: WalkthroughSource = kind === 'session' ? { kind } : kind === 'branch' ? { kind, range: input.trim() } : parseWalkthroughPr(input);
      const result = await walkthroughCall<WalkthroughPlan>('', { source: next }, sessionId);
      if (request !== generation.current) return;
      setSource(next); setPlan(result); setIndex(0); setHunk(0);
      let stored: string | null = null;
      try { stored = localStorage.getItem(reviewStorageKey(sessionId ?? 'standalone', next)); } catch { /* storage may be unavailable */ }
      setApproved(new Set(restoreApprovals(stored, result.files)));
    } catch (e) { if (request === generation.current) setError((e as Error).message); }
    finally { if (request === generation.current) setBusy(false); }
  }

  const stop = plan?.stops[index];
  const fileIndex = stop?.file_index ?? 0;
  const patch = plan?.files[fileIndex] ?? '';
  const hunks = useMemo(() => splitWalkthroughHunks(patch), [patch]);
  const rendered = useMemo<ReviewDiffFile[]>(() => {
    const parsed = getRenderablePatch(hunks[hunk] ?? patch, 'mira-walkthrough');
    return parsed?.kind === 'files' ? parsed.files.map(fileDiff => ({ fileDiff,
      filePath: resolveFileDiffPath(fileDiff), fileKey: buildFileDiffIdentityKey(fileDiff),
      fileVersion: buildFileDiffContentVersion(fileDiff), collapsed: false })) : [];
  }, [patch, hunks, hunk]);
  const path = rendered[0]?.filePath ?? patch.split('\n')[0]?.replace(/^diff --git /, '') ?? 'Changed file';
  const key = `${fileIndex}:${hunk}`;
  const total = plan?.files.reduce((n, p) => n + Math.max(1, splitWalkthroughHunks(p).length), 0) ?? 0;
  function go(next: number) { setIndex(next); setHunk(0); setQuestion(''); setConfirm(false); setError(null); setNotice(null); }
  function approve() {
    if (!plan) return;
    const next = new Set(approved);
    if (next.has(key)) next.delete(key); else next.add(key);
    setApproved(next);
    try { localStorage.setItem(reviewStorageKey(sessionId ?? 'standalone', source), JSON.stringify({ files: plan.files, approved: [...next] })); } catch { /* current review remains usable */ }
  }
  async function revert() {
    if (!plan) return;
    const request = generation.current;
    setBusy(true); setError(null); setConfirm(false);
    let applied = false;
    try {
      await walkthroughCall('/revert', { source, file_index: fileIndex, expected_patch: patch, hunk_index: hunk }, sessionId);
      applied = true;
      onChanged();
      if (request !== generation.current) return;
      if (source.kind === 'session') {
        // Rebuild the plan after a mutation, carrying forward approvals only
        // for files whose full patch is still identical.
        const result = await walkthroughCall<WalkthroughPlan>('', { source }, sessionId);
        if (request !== generation.current) return;
        const nextApprovals = new Set<string>();
        for (const entry of approved) {
          const [oldFile, oldHunk] = entry.split(':').map(Number);
          const nextFile = result.files.indexOf(plan.files[oldFile]);
          if (nextFile >= 0) nextApprovals.add(`${nextFile}:${oldHunk}`);
        }
        setPlan(result); setApproved(nextApprovals); setAnswers({});
        setIndex(Math.min(index, Math.max(0, result.stops.length - 1))); setHunk(0);
        try { localStorage.setItem(reviewStorageKey(sessionId ?? 'standalone', source), JSON.stringify({ files: result.files, approved: [...nextApprovals] })); } catch { /* optional */ }
      } else {
        // A committed range or remote PR remains unchanged when its local
        // working file changes. Keep its review order and disable this action.
        setReverted(prev => new Set([...prev, key]));
      }
      setNotice('Hunk reverted in the local working file.');
    } catch (e) {
      if (request === generation.current) {
        if (applied) setPlan(null);
        setError(`${applied ? 'Hunk reverted, but refreshing failed. Reload the walkthrough. ' : ''}${(e as Error).message}`);
      }
    }
    finally { setBusy(false); }
  }
  async function ask() {
    const request = generation.current;
    const target = fileIndex;
    setAsking(true); setError(null);
    try {
      const result = await walkthroughCall<{ answer: string }>('/question', { patch, question }, sessionId);
      if (request === generation.current) { setAnswers(prev => ({ ...prev, [target]: `Question: ${question}\n\n${result.answer}` })); setQuestion(''); }
    } catch (e) { if (request === generation.current) setError((e as Error).message); }
    finally { setAsking(false); }
  }
  const pending = busy || asking;
  return <section className="flex h-full min-h-0 flex-col" aria-label="Guided walkthrough">
    <div className="flex flex-wrap items-end gap-2 border-b border-border p-3">
      <label className="flex flex-col gap-1 text-xs">Review source
        <select aria-label="Review source" className="rounded border border-border bg-background p-1.5" disabled={pending} value={kind} onChange={e => { setKind(e.target.value as WalkthroughSource['kind']); setPlan(null); setError(null); setInput(e.target.value === 'branch' ? 'main...HEAD' : ''); }}>
          <option value="session">Agent’s changes</option><option value="branch">Branch / git range</option><option value="pr">Pull request</option>
        </select>
      </label>
      {kind !== 'session' && <label className="flex min-w-40 flex-1 flex-col gap-1 text-xs">{kind === 'branch' ? 'Git range' : 'GitHub PR URL or owner/repo/number'}
        <input className="rounded border border-border bg-background p-1.5" disabled={pending} value={input} onChange={e => { setInput(e.target.value); setPlan(null); }} />
      </label>}
      <button className={button} disabled={pending} onClick={() => void load()}>{busy ? 'Preparing walkthrough…' : plan ? 'Reload walkthrough' : 'Start walkthrough'}</button>
      <button className={button} disabled={pending} onClick={onClose}>Back to files</button>
    </div>
    {notice && <p role="status" className="px-4 py-2 text-sm text-muted-foreground">{notice}</p>}
    {error && <p role="alert" className="px-4 py-2 text-sm text-destructive">{error}</p>}
    {!plan && !busy && !error && <p className="p-5 text-sm text-muted-foreground">Review the core change, its dependents, tests, then incidental edits. Mira explains each stop and flags changes to checks, errors, and I/O.</p>}
    {busy && <p role="status" className="p-4 text-sm text-muted-foreground">Reading the diff and preparing the review…</p>}
    {plan?.stops.length === 0 && <p className="p-5 text-sm">No changes in this source.</p>}
    {plan && stop && <>
      <div className="flex min-h-0 flex-1">
        <nav aria-label="Walkthrough stops" className="hidden w-56 shrink-0 overflow-auto border-r border-border p-2 md:block">
          {plan.stops.map((s, i) => s.category !== 'incidental' && <button key={s.file_index} className={`mb-1 block w-full rounded p-2 text-left text-xs ${i === index ? 'bg-accent' : 'hover:bg-accent/50'}`} aria-current={i === index ? 'step' : undefined} disabled={pending} onClick={() => go(i)}>
            <span className="block text-muted-foreground">{i + 1}. {labels[s.category]}</span>
            <span className="break-all">{fileName(plan.files[s.file_index])}</span>
            {s.risks.length > 0 && <span className="block text-amber-600 dark:text-amber-400">{s.risks.length} risk marker{s.risks.length === 1 ? '' : 's'}</span>}
          </button>)}
          {plan.stops.some(s => s.category === 'incidental') && <details open={incidentalOpen || stop.category === 'incidental'} onToggle={e => setIncidentalOpen(e.currentTarget.open)}>
            <summary className="cursor-pointer p-2 text-xs">Incidental edits ({plan.stops.filter(s => s.category === 'incidental').length})</summary>
            {plan.stops.map((s, i) => s.category === 'incidental' && <button className={`${button} mb-1 w-full break-all text-left`} key={i} disabled={pending} onClick={() => go(i)} aria-current={i === index ? 'step' : undefined}>{i + 1}. {fileName(plan.files[s.file_index])}</button>)}
          </details>}
        </nav>
        <div className="flex min-h-0 min-w-0 flex-1 flex-col">
          <div className="max-h-[45%] shrink-0 overflow-auto border-b border-border p-4">
            <p className="text-xs text-muted-foreground">Stop {index + 1} of {plan.stops.length} · {labels[stop.category]}</p>
            <h3 className="my-1 break-all font-mono text-sm">{path}</h3>
            <p className="text-sm">{stop.summary}</p>
            {stop.risks.length > 0 && <div className="mt-2 rounded border border-amber-500/40 bg-amber-500/10 p-2 text-xs"><p className="font-medium">Risk markers · AI suggestions to verify</p><ul className="mt-1 list-disc pl-4">{stop.risks.map((risk, i) => <li key={i}>{risk}</li>)}</ul></div>}
            <form className="mt-3 flex gap-2" onSubmit={e => { e.preventDefault(); void ask(); }}>
              <input aria-label="Question about this stop" className="min-w-0 flex-1 rounded border border-border bg-background px-2 py-1 text-sm" placeholder="Ask about this stop…" value={question} disabled={pending} onChange={e => setQuestion(e.target.value)} maxLength={8000} />
              <button className={button} disabled={pending || !question.trim()}>{asking ? 'Answering…' : 'Ask'}</button>
            </form>
            {answers[fileIndex] && <p role="status" className="mt-2 whitespace-pre-wrap text-sm text-muted-foreground">{answers[fileIndex]}</p>}
          </div>
          <div className="min-h-0 flex-1 overflow-auto">
            {rendered.length ? <ReviewDiffView enableComments={false} codeViewKey={`walkthrough:${fileIndex}:${hunk}:${rendered[0].fileVersion}`} files={rendered} comments={[]} onAddComment={() => {}} onUpdateCommentText={() => {}} onRemoveComment={() => {}}
              options={{ ...diffOptions, enableLineSelection: false, enableGutterUtility: false }} className="h-full min-h-48" renderHeaderFilenameSuffix={() => null} renderHeaderPrefix={() => null} /> : <pre className="overflow-auto p-4 text-xs">{patch}</pre>}
          </div>
          <div className="flex flex-wrap items-center gap-2 border-t border-border p-3">
            {hunks.length > 0 && <><button className={button} disabled={pending || hunk === 0} onClick={() => { setHunk(hunk - 1); setConfirm(false); }}>Previous hunk</button><span className="text-xs">Hunk {hunk + 1}/{hunks.length}</span><button className={button} disabled={pending || hunk === hunks.length - 1} onClick={() => { setHunk(hunk + 1); setConfirm(false); }}>Next hunk</button></>}
            <button className={button} disabled={pending} aria-pressed={approved.has(key)} onClick={approve}>{approved.has(key) ? 'Approved ✓' : hunks.length ? 'Approve hunk' : 'Mark file reviewed'}</button>
            {!canRevertWalkthroughHunk(patch) && <span className="text-xs text-muted-foreground">Structural change · use file-level revert</span>}
            {canRevertWalkthroughHunk(patch) && (confirm ? <><span className="text-xs">Discard this hunk locally?</span><button className={button} disabled={pending} onClick={() => void revert()}>Confirm revert</button><button className={button} disabled={pending} onClick={() => setConfirm(false)}>Cancel</button></> : <button className={button} disabled={pending || reverted.has(key)} onClick={() => setConfirm(true)}>{reverted.has(key) ? 'Reverted locally' : 'Revert hunk'}</button>)}
          </div>
        </div>
      </div>
      <footer className="flex flex-wrap items-center gap-2 border-t border-border p-3">
        <span className="mr-auto text-xs text-muted-foreground">{approved.size}/{total} reviewed · approvals saved locally{approved.size === total ? ' · Review complete' : ''}</span>
        <button className={button} disabled={pending || index === 0} onClick={() => go(index - 1)}>Previous stop</button>
        <button className={button} disabled={pending || index === plan.stops.length - 1} onClick={() => go(index + 1)}>Next stop</button>
      </footer>
    </>}
  </section>;
}
function fileName(patch: string): string {
  const match = patch.match(/^\+\+\+ b\/(.+)$/m) ?? patch.match(/^--- a\/(.+)$/m);
  return match?.[1] ?? patch.split('\n')[0].replace(/^diff --git /, '');
}
