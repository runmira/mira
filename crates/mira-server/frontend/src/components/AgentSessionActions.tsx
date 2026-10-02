/**
 * Actions on the agent session itself — as opposed to the agent's
 * configuration, which lives in Settings → Agents. They sit with the
 * session (the composer's engine picker), because that is where the user
 * is when they want them.
 */
import { useState } from 'react';
import { listAgentTurns, revertAgentTurn, type AgentTurn } from '../api';

/**
 * Revert an agent session to before a turn ran.
 *
 * Two steps, both explicit: pick the turn, then confirm with the
 * consequences stated (files restored, transcript dropped from there, agent
 * restarted fresh with no memory of the reverted turns, untracked files
 * untouched). A one-click revert would be a footgun next to Stop.
 */
export function RevertButton({
  sessionId,
  onReverted,
}: {
  sessionId: string | null | undefined;
  onReverted?: () => void;
}) {
  const [open, setOpen] = useState(false);
  const [turns, setTurns] = useState<AgentTurn[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [confirm, setConfirm] = useState<AgentTurn | null>(null);
  const [busy, setBusy] = useState(false);

  function load() {
    if (!sessionId) return;
    setError(null);
    listAgentTurns(sessionId)
      .then(setTurns)
      .catch((e) => setError(String((e as Error).message)));
  }

  async function revert(turn: number) {
    if (!sessionId) return;
    setBusy(true);
    setError(null);
    try {
      await revertAgentTurn(sessionId, turn);
      setOpen(false);
      setConfirm(null);
      onReverted?.();
    } catch (e) {
      setError(String((e as Error).message));
    } finally {
      setBusy(false);
    }
  }

  if (!sessionId) return null;
  return (
    <div className="relative">
      <button
        type="button"
        onClick={() => {
          setOpen((v) => !v);
          setConfirm(null);
          if (turns === null) load();
        }}
        title="Restore files and transcript to before a turn"
        className="flex items-center gap-1 rounded-lg border border-border/80 px-2 py-1
                   text-[11.5px] text-muted-foreground transition-colors hover:bg-muted/50 hover:text-foreground"
      >
        Revert
      </button>
      {open && (
        <div className="absolute bottom-full right-0 z-50 mb-1.5 w-72 rounded-xl border border-border/70 bg-popover p-1.5 shadow-xl">
          {error && <div className="px-2 py-1.5 text-[11.5px] text-destructive">{error}</div>}
          {turns === null ? (
            <div className="px-2 py-2 text-[12px] text-muted-foreground">Loading turns…</div>
          ) : turns.length === 0 ? (
            <div className="px-2 py-2 text-[12px] text-muted-foreground">
              No recorded turns yet — send the agent a prompt first.
            </div>
          ) : confirm ? (
            <div className="px-2 py-1.5">
              <div className="text-[12.5px] font-medium text-foreground">
                Revert to before turn {confirm.turn}?
              </div>
              <div className="mt-1 text-[11.5px] leading-snug text-muted-foreground">
                {confirm.snapshot
                  ? 'Tracked files go back, later transcript is dropped, and the agent restarts fresh with no memory of those turns.'
                  : 'No file snapshot for this turn — transcript only. Later turns are dropped.'}{' '}
                Untracked files are always left alone.
              </div>
              <div className="mt-2 flex justify-end gap-1.5">
                <button
                  type="button"
                  disabled={busy}
                  onClick={() => setConfirm(null)}
                  className="rounded-md px-2 py-1 text-[12px] text-muted-foreground hover:bg-fg/[0.05]"
                >
                  Cancel
                </button>
                <button
                  type="button"
                  disabled={busy}
                  onClick={() => void revert(confirm.turn)}
                  className="rounded-md bg-destructive px-2.5 py-1 text-[12px] font-medium text-white hover:opacity-90 disabled:opacity-50"
                >
                  {busy ? 'Reverting…' : `Revert to turn ${confirm.turn}`}
                </button>
              </div>
            </div>
          ) : (
            <div className="max-h-56 overflow-y-auto">
              {turns.map((t) => (
                <button
                  key={t.turn}
                  type="button"
                  onClick={() => setConfirm(t)}
                  className="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left transition-colors hover:bg-fg/[0.05]"
                >
                  <span className="shrink-0 rounded bg-muted/70 px-1.5 py-px font-mono text-[10.5px] text-muted-foreground">
                    {t.turn}
                  </span>
                  <span className="min-w-0 flex-1 truncate text-[12px] text-foreground/90">
                    {t.first_text || `Turn ${t.turn}`}
                  </span>
                  {!t.snapshot && (
                    <span className="shrink-0 text-[10px] text-muted-foreground/60" title="No file snapshot — transcript only">
                      no files
                    </span>
                  )}
                </button>
              ))}
            </div>
          )}
        </div>
      )}
    </div>
  );
}

