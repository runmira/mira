/**
 * Dev-only preview of the annotation flow on a fake conversation: select
 * text in a reply for Quote / Ask in side chat / Copy; quotes become cards
 * in the composer, and clicking a card jumps back to the passage.
 * Open /annotation-preview.html on the Vite dev server.
 */
import React, { useRef, useState } from 'react';
import ReactDOM from 'react-dom/client';
import './styles.css';
import { Composer } from './components/Composer';
import { AssistantSelectionToolbar } from './components/AssistantSelectionToolbar';
import { SourceCitationNavigator } from './components/SourceCitationNavigator';
import { Markdown } from './components/Markdown';
import { composeQuote, getAsidePassage, setAsidePassage, subscribeAsidePassage } from './lib/attachBridge';
import type { Mode } from './types';
import { applyTheme } from './lib/theme';
import { installInputModality } from './lib/inputModality';

// The app's own startup: theme tokens and focus-ring behaviour.
applyTheme();
installInputModality();

const REPLIES = [
  'I traced the archived-list bug to `list_sessions`. After loading saved chats it appends every chat that is open in memory but missing from the list, assuming it is new and unsaved. For the archived view that list only has archived chats, so every open chat landed there as Untitled.',
  'The fix skips that step for the archived view, only adds chats that were never saved, and keeps them to the current folder. Unsaved chats now get their real title and the current time instead of zero, which is why they showed as 2961 weeks old.',
  'Filler so the page scrolls. '.repeat(40),
  'Last reply: the sidebar now nests launched threads under the chat that started them, shows a branch icon for worktree chats, and puts an amber badge on anything waiting on you.',
];

function Preview() {
  const pane = useRef<HTMLDivElement>(null);
  const [mode, setMode] = useState<Mode>('manual');
  const [sent, setSent] = useState<string | null>(null);
  const aside = React.useSyncExternalStore(subscribeAsidePassage, getAsidePassage);
  return (
    <main className="flex h-screen flex-col bg-background text-foreground">
      <div ref={pane} className="min-h-0 flex-1 overflow-y-auto px-8 py-6">
        <div className="mx-auto flex max-w-2xl flex-col gap-6">
          {REPLIES.map((text, i) => (
            <div key={i} data-minimap-id={`turn-${i}`} className="flex flex-col gap-2">
              <div className="self-end rounded-2xl bg-fg/[0.07] px-3 py-2 text-[13px]">Question {i + 1}</div>
              <div data-assistant-message className="text-[14px] leading-relaxed"><Markdown text={text} /></div>
            </div>
          ))}
          {sent && (
            <div className="self-end max-w-[85%] rounded-2xl bg-fg/[0.07] px-3 py-2 text-[13px]">
              <div className="mb-1 text-[10.5px] uppercase tracking-wide text-muted-foreground">Sent (rendered)</div>
              <Markdown text={sent} />
            </div>
          )}
        </div>
      </div>
      {aside && (
        <div className="mx-auto mb-2 w-full max-w-2xl rounded-lg border border-border px-3 py-2 text-[12px] text-muted-foreground">
          Side chat would open with: <span className="text-foreground">{aside.slice(0, 120)}…</span>{' '}
          <button className="underline" onClick={() => setAsidePassage(null)}>clear</button>
        </div>
      )}
      <AssistantSelectionToolbar
        pane={pane}
        sessionId="preview"
        onQuote={(text, turn, href) => composeQuote({ text, turn, href })}
        onAskAside={(text) => setAsidePassage(text)}
      />
      <div className="mx-auto w-full max-w-2xl px-4 pb-6">
        <SourceCitationNavigator pane={pane} sessionId="preview" hasOlder={false} loading={false} loadOlder={() => {}} historyError={null} onOpenSession={() => {}} />
      <Composer
        disabled={false}
        busy={false}
        mode={mode}
        model="gpt-6-astra"
        providerName="OpenAI"
        cwd="/Users/damilola/Desktop/coding_agent/mira"
        usage={null}
        onSend={(text) => setSent(text)}
        onQueueMessage={() => {}}
        queuedMessages={[]}
        onRemoveQueuedMessage={() => {}}
        onSteerQueuedMessage={() => {}}
        onSetMode={setMode}
        onSetModel={() => {}}
        onSetModelOption={() => {}}
        onOpenPicker={() => {}}
        onInterrupt={() => {}}
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
      </div>
    </main>
  );
}

ReactDOM.createRoot(document.getElementById('root')!).render(
  <React.StrictMode>
    <Preview />
  </React.StrictMode>,
);
