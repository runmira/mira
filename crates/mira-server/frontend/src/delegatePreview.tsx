// TEMPORARY visual harness for the delegation card. Not shipped.
import { createRoot } from 'react-dom/client';
import { applyTheme, watchSystemTheme } from './lib/theme';
import { applyAppearance } from './lib/appearance';
import { DelegateCard } from './components/DelegateCard';
import type { ToolCall } from './types';
import './styles.css';

applyTheme();
watchSystemTheme();
applyAppearance();

const call = (name: string, args: unknown): ToolCall => ({
  id: name,
  type: 'function',
  function: { name, arguments: JSON.stringify(args) },
});

const AGENTS = ['mira', 'claude-code', 'codex', 'opencode', 'grok', 'cursor'];

function Row({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="flex flex-col gap-1">
      <div className="text-[11px] font-semibold uppercase tracking-wider text-muted-foreground/60">{label}</div>
      <div className="flex justify-start">{children}</div>
    </div>
  );
}

function Demo() {
  const args = {
    prompt:
      'Review crates/mira-auth/src/lib.rs for bugs. One short paragraph. Focus on the loopback bind and the PKCE verifier.',
  };
  return (
    <div className="min-h-screen bg-background p-6 text-foreground">
      <div className="mx-auto flex max-w-2xl flex-col gap-6">
        <Row label="Pending approval">
          <DelegateCard call={call('delegate_task', { ...args, engine: 'codex' })} status="pending" result={null} startedAt={Date.now() - 3000} />
        </Row>
        <Row label="Working — Mira">
          <DelegateCard
            call={call('delegate_task', args)}
            status="running"
            result={null}
            startedAt={Date.now() - 42_000}
            steps={[
              { kind: 'search', text: 'Searched for "bind_any"' },
              { kind: 'read', text: 'Read …/mira-auth/src/loopback.rs' },
              { kind: 'read', text: 'Read …/mira-auth/src/pkce.rs' },
              { kind: 'search', text: 'Found references to code_challenge_s256' },
              { kind: 'read', text: 'Read …/mira-auth/src/store.rs' },
              { kind: 'note', text: 'Comparing against RFC 7636 requirements' },
            ]}
          />
        </Row>
        <Row label="Working — external agent">
          <DelegateCard
            call={call('delegate_task', { ...args, engine: 'claude-code' })}
            status="running"
            result={null}
            startedAt={Date.now() - 8_000}
            steps={[
              { kind: 'read', text: 'Read crates/mira-auth/src/lib.rs' },
              { kind: 'search', text: 'Grep for "loopback" in crates' },
            ]}
          />
        </Row>
        <Row label="Answered">
          <DelegateCard
            call={call('delegate_task', { ...args, engine: 'claude-code' })}
            status="complete"
            startedAt={Date.now() - 96_000}
            result={{
              call_id: 'delegate_task',
              content:
                'The crate is coherent. The one real risk is in `loopback.rs`: `bind_any()` reports every bind failure as "another sign-in is in progress", which hides port exhaustion and permission errors. `is_noise()` is also stricter than OAuth requires — it drops any callback without both `code` and `error`, which is fine for the standard flow but would swallow a provider that signals differently. PKCE generation itself is correct: URL-safe base64, 64 bytes of randomness, S256 challenge.',
            }}
          />
        </Row>
        <Row label="Failed">
          <DelegateCard
            call={call('delegate_task', { ...args, engine: 'grok' })}
            status="complete"
            startedAt={Date.now() - 5_000}
            result={{ call_id: 'delegate_task', content: "grok finished without an answer", is_error: true }}
          />
        </Row>
        <div className="text-[11px] text-muted-foreground/50">
          engines: {AGENTS.join(', ')}
        </div>
      </div>
    </div>
  );
}

createRoot(document.getElementById('root')!).render(<Demo />);
