/** Dev-only preview of the compaction card's states. Open
 *  /compaction-preview.html on the Vite dev server. */
import React from 'react';
import ReactDOM from 'react-dom/client';
import './styles.css';
import { applyTheme } from './lib/theme';
import { CompactionCard } from './components/CompactionCard';

applyTheme();
const now = Date.now();

ReactDOM.createRoot(document.getElementById('root')!).render(
  <React.StrictMode>
    <main className="mx-auto flex min-h-screen max-w-2xl flex-col gap-6 bg-background px-6 py-10 text-foreground">
      <p className="text-[13px] text-muted-foreground">Running (manual):</p>
      <CompactionCard entry={{ kind: 'compact', state: 'running', trigger: 'manual', startedAt: now - 7000, summarized: null, summary: null, tokensBefore: 182_400 }} />
      <p className="text-[13px] text-muted-foreground">Running (auto, mid-turn):</p>
      <CompactionCard entry={{ kind: 'compact', state: 'running', trigger: 'auto', startedAt: now - 2000, summarized: null, summary: null, tokensBefore: 251_000 }} />
      <p className="text-[13px] text-muted-foreground">Done:</p>
      <CompactionCard entry={{ kind: 'compact', state: 'done', trigger: 'manual', startedAt: now - 12_000, endedAt: now, summarized: 34, summary: 'The user is building a sidebar redesign…\n\n- Needs-you badges\n- Search\n- Nested threads', tokensBefore: 182_400, tokensAfter: 18_900 }} />
      <p className="text-[13px] text-muted-foreground">Failed:</p>
      <CompactionCard entry={{ kind: 'compact', state: 'failed', trigger: 'auto', startedAt: now - 3000, endedAt: now, summarized: null, summary: null, error: 'The summary model returned an error (429)' }} />
    </main>
  </React.StrictMode>,
);
