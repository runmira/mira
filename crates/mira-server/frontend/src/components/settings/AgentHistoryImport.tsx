import { useState } from 'react';
import type { ExternalAgentSession } from '../../types';
import { listExternalAgentSessions } from '../../api';

/**
 * Resume an agent's own past sessions inside Mira.
 *
 * Deliberately self-contained — no settings state, no panel chrome, one
 * callback out. It renders today under the agent's row in Settings; the
 * onboarding wizard will render this same component as a step, because
 * "pick up where your terminal left off" is a first-run question, not a
 * settings page. Nothing here may assume either host.
 */
export function AgentHistoryImport({
  onResume,
}: {
  /** Resume this history session id in Mira. The host starts the agent. */
  onResume: (sessionId: string) => void;
}) {
  const [history, setHistory] = useState<ExternalAgentSession[] | null>(null);
  const [historyError, setHistoryError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);

  function toggle() {
    if (history !== null) {
      setHistory(null);
      return;
    }
    setHistoryError(null);
    setLoading(true);
    listExternalAgentSessions()
      .then(setHistory)
      .catch((e) => setHistoryError(String((e as Error).message)))
      .finally(() => setLoading(false));
  }

  return (
    <div>
      <button
        type="button"
        onClick={toggle}
        className="text-[12px] font-medium text-muted-foreground transition-colors hover:text-foreground"
      >
        {history === null
          ? loading
            ? 'Looking for previous sessions…'
            : 'Resume a previous session'
          : 'Hide previous sessions'}
      </button>
      {historyError && (
        <div className="mt-1 text-[12px] text-destructive/85">{historyError}</div>
      )}
      {history !== null && history.length === 0 && (
        <div className="mt-1 text-[12px] text-muted-foreground/70">
          No previous sessions found.
        </div>
      )}
      {history !== null && history.length > 0 && (
        <div className="mt-1.5 grid gap-1.5">
          {history.slice(0, 20).map((h) => (
            <div
              key={h.id}
              className="flex items-center gap-2 rounded-lg border border-border/60 px-2.5 py-1.5"
            >
              <div className="min-w-0 flex-1">
                <div className="truncate text-[12.5px] text-foreground/90">
                  {h.first_text || h.id.slice(0, 8)}
                </div>
                <div className="truncate font-mono text-[10.5px] text-muted-foreground/70">
                  {h.model ? `${h.model} · ` : ''}
                  {h.messages} messages · {h.cwd}
                </div>
              </div>
              <button
                type="button"
                onClick={() => onResume(h.id)}
                className="shrink-0 rounded-md bg-mira-blue/90 px-2.5 py-1 text-[11.5px] text-white transition-colors hover:bg-mira-blue"
              >
                Resume
              </button>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
