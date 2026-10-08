import React, { useState } from 'react';
import ReactDOM from 'react-dom/client';
import './styles.css';
import { Composer, type QueuedComposerMessage } from './components/Composer';
import type { Mode } from './types';

function Preview() {
  const [queued, setQueued] = useState<QueuedComposerMessage[]>([
    { id: 'q1', text: 'Shop feedback — tighten the bottom UI and make the card feel calmer.' },
    { id: 'q2', text: 'Refine a bit the UI at the bottom of the shop, it is cramped right now.' },
    { id: 'q3', text: 'We also need a way to inspect the state of my character.' },
  ]);
  const [mode, setMode] = useState<Mode>('manual');
  const [steered, setSteered] = useState<string | null>(null);
  return (
    <main className="flex min-h-screen items-end justify-center bg-background px-8 py-10 text-foreground">
      {steered && (
        <div className="mb-3 rounded-full bg-mira-blue/10 px-3 py-1.5 text-[12px] text-mira-blue">{steered}</div>
      )}
      <Composer
        disabled={false}
        busy
        mode={mode}
        model="gpt-6-astra"
        providerName="OpenAI"
        cwd="/Users/damilola/Desktop/coding_agent/mira"
        usage={null}
        onSend={() => {}}
        onQueueMessage={(text, images) => setQueued((items) => [...items, { id: `${Date.now()}`, text, images }])}
        queuedMessages={queued}
        onRemoveQueuedMessage={(id) => setQueued((items) => items.filter((item) => item.id !== id))}
        onSteerQueuedMessage={(id) => {
          const picked = queued.find((item) => item.id === id);
          setQueued((items) => items.filter((item) => item.id !== id));
          setSteered(picked ? `Interrupted current turn · steering with “${picked.text}”` : 'Interrupted current turn');
        }}
        onSetMode={setMode}
        onSetModel={() => {}}
        onSetModelOption={() => {}}
        onOpenPicker={() => {}}
        onInterrupt={() => setSteered('Interrupted current turn')}
        onNewChat={() => {}}
        onOpenSettings={() => {}}
        onRunReview={() => {}}
        onSetGoal={() => {}}
        onClearGoal={() => {}}
        onCompact={() => {}}
        goal={null}
        onRemember={async () => 'remembered'}
        onUndo={async () => 'nothing to undo'}
        skills={[]}
        commands={[]}
        engine={{ kind: 'provider', instance: 'openai', display_name: 'OpenAI', model: 'gpt-6-astra', status: 'ready' }}
        engines={[]}
        agents={[]}
        onPickProvider={() => {}}
      />
    </main>
  );
}

ReactDOM.createRoot(document.getElementById('root')!).render(
  <React.StrictMode>
    <Preview />
  </React.StrictMode>,
);
