/** Local review fixtures. All API mutations are intercepted; no files or PRs change. */
import { useEffect, useState } from 'react';
import { createRoot } from 'react-dom/client';
import { RuntimeProvider, useAppRuntime, useWorkspaceUiField } from './state/RuntimeProvider';
import { openTurnReview } from './lib/openTurnReview';
import { setThemePref } from './lib/theme';
import { TurnChanges } from './components/TurnChanges';
import { ThoughtBlock } from './components/ThoughtBlock';
import { ReviewChanges } from './components/ReviewChanges';
import { DetailHeader } from './components/pullRequests/PullRequestDetail';
import { CodePane } from './components/pullRequests/CodePane';
import { RemoteControlSection } from './components/context/RemoteControlSection';
import { LastTurnChanges } from './components/context/LastTurnChanges';
import type { TurnDiffSummary } from './types';
import type { PullRequestDetailView } from './api';
import './styles.css';
const summary: TurnDiffSummary = { text: 'Review the save flow', occurrence: 0, files: [
  { path: 'src/save.ts', added: 4, removed: 1, binary: false },
  { path: 'src/save.test.ts', added: 12, removed: 0, binary: false },
  { path: 'docs/guide.md', added: 3, removed: 1, binary: false },
  ...Array.from({ length: 29 }, (_, i) => ({ path: `src/helpers/helper-${i}.ts`, added: 1, removed: 0, binary: false })),
] };
const selection = { owner: 'runmira', repo: 'mira', number: 213, project_label: 'mira' };
const detail = { merged: false, mergeable: true, summary: { html_url: 'https://github.com/runmira/mira/pull/213' } } as PullRequestDetailView;
window.fetch = async (input, init) => {
  const url = typeof input === 'string' ? input : input instanceof URL ? input.pathname : input.url;
  if (url.includes('/api/git/session-changes')) return new Response(JSON.stringify({files:[{path:'src/save.ts',status:'modified',added:1,removed:1,binary:false,diff:'diff --git a/src/save.ts b/src/save.ts\n--- a/src/save.ts\n+++ b/src/save.ts\n@@ -1 +1 @@\n-old\n+new\n'}]}));
  if (url.includes('/turn-diff')) return new Response(JSON.stringify({ before: 'export function save() {\n  writeFile();\n}\n', after: 'export async function save() {\n  await writeFile();\n}\n' }));
  if (url.endsWith('/files')) return new Response(JSON.stringify({ files: [
    { filename: 'src/save.ts', status: 'modified', additions: 2, deletions: 2, patch: '@@ -40,3 +40,3 @@\n-export function save() {\n-  writeFile();\n+export async function save() {\n+  await writeFile();\n }', raw_url: null },
    { filename: 'assets/logo.png', status: 'modified', additions: 0, deletions: 0, patch: null, raw_url: null },
  ] }));
  if (url === '/api/remote' && !init?.method) return new Response(JSON.stringify({ enabled: true, supported: true, status: 'connected', hostname: 'hidden-preview.runmira.dev' }));
  return new Response(JSON.stringify({ error: 'Preview only' }), { status: 400 });
};
function Preview() {
  const runtime = useAppRuntime();
  const [reviewOpen, setReviewOpen] = useWorkspaceUiField('reviewOpen');
  const [mode, setMode] = useState('Turn card'), [dark, setDark] = useState(false), [file, setFile] = useState<string | null>(null);
  useEffect(() => { runtime.turnCommands.hydrate('review-preview', false, []); runtime.updateSessionData('turnDiffs', [summary]); runtime.updateSessionData('entries', [{kind:'msg', msg:{role:'user', content:summary.text}}]); openTurnReview(runtime, summary); }, [runtime]);
  useEffect(() => setThemePref(dark ? 'dark' : 'light'), [dark]);
  return <main className="min-h-screen bg-background text-foreground">
    <nav className="flex flex-wrap gap-2 border-b border-border p-3 text-xs">{['Turn card', 'Context card', 'PR header', 'PR code', 'Thinking'].map(label => <button className="rounded border border-border px-2 py-1" key={label} onClick={() => setMode(label)}>{label}</button>)}<button onClick={() => setDark(!dark)}>Toggle theme</button></nav>
    <div className="mx-auto max-w-4xl py-4">
      {mode === 'PR header' ? <DetailHeader selection={selection} detail={detail} pane="summary" onPane={() => setMode('PR code')} expanded={false} onToggleExpand={() => {}} onReload={() => {}} onReviewPr={() => {}} hideExpand /> : mode === 'PR code' ? <CodePane selection={selection} /> : <div className="px-3">
        {mode === 'Turn card' && <TurnChanges summary={summary} onOpenFile={setFile} onReview={() => openTurnReview(runtime, summary)} onUndo={() => {}} />}
        {mode === 'Context card' && <section className="rounded-xl border border-border px-4"><RemoteControlSection /><LastTurnChanges summary={summary} onOpenFile={setFile} onReview={() => openTurnReview(runtime, summary)} /></section>}
        {mode === 'Thinking' && <ThoughtBlock live content={'## Review plan\n\n**Check the save flow** before updating callers.\n\n1. Read `save.ts`\n2. Verify error recovery\n\n```ts\nawait save();\n```\n\nThe file write must finish before we report success.'} />}
        {file && <aside aria-label="File panel" className="mt-4 rounded-lg border border-border p-4 text-sm">Right file panel: <code>{file}</code></aside>}
      </div>}
    </div>
    <div style={{width: 380, minWidth: 280, maxWidth: '95vw', height: 640, resize: 'horizontal', overflow: 'hidden'}} className="ml-auto mr-4 flex flex-col rounded-xl border border-border"><div className="shrink-0 border-b border-border px-3 py-2 text-xs">Diff · drag the bottom corner to resize</div><div className="min-h-0 flex-1">{reviewOpen ? <ReviewChanges embedded open onClose={() => setReviewOpen(false)} onChanged={() => {}} onSendComments={() => {}} /> : <button className="m-4 rounded-full bg-secondary px-3 py-2 text-xs" onClick={() => openTurnReview(runtime, summary)}>Open latest turn changes</button>}</div></div>
  </main>;
}
createRoot(document.getElementById('root')!).render(<RuntimeProvider scope={{ backend: 'review-preview', account: 'fixture' }}><Preview /></RuntimeProvider>);
