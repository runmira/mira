import { Button } from '@/components/ui/button';
import { Dialog, DialogContent } from '@/components/ui/dialog';
import { Input } from '@/components/ui/input';
import { Sparkles } from 'lucide-react';
import { useEffect, useState } from 'react';
import type { SessionSummary } from '../../types';
import { sessionLabel } from './sessionRows';
/* ---------- rename dialog ---------- */

export function RenameDialog({
  session,
  onClose,
  onManual,
  onAi,
}: {
  session: SessionSummary | null;
  onClose: () => void;
  onManual: (id: string, title: string) => Promise<void>;
  onAi: (id: string) => Promise<string>;
}) {
  const [value, setValue] = useState('');
  const [aiBusy, setAiBusy] = useState(false);
  const [saveBusy, setSaveBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const [aiApplied, setAiApplied] = useState<string | null>(null);

  // Hydrate the input each time the dialog opens for a different session.
  useEffect(() => {
    if (!session) return;
    // Seed the rename input with the cleaned-up label so the user isn't
    // editing a raw "## Attached files …" markdown blob when their first
    // turn was an attachment.
    setValue(session.title ?? (session.first_user_message ? sessionLabel(session) : ''));
    setAiBusy(false);
    setSaveBusy(false);
    setErr(null);
    setAiApplied(null);
  }, [session]);

  if (!session) return null;

  async function submitManual(e: React.FormEvent) {
    e.preventDefault();
    if (!session) return;
    const trimmed = value.trim();
    if (!trimmed) return;
    setSaveBusy(true);
    setErr(null);
    try {
      await onManual(session.id, trimmed);
    } catch (e2) {
      setErr(String((e2 as Error).message));
    } finally {
      setSaveBusy(false);
    }
  }

  async function submitAi() {
    if (!session) return;
    setAiBusy(true);
    setErr(null);
    setAiApplied(null);
    try {
      const generated = await onAi(session.id);
      // Populate the input with the AI result. Don't auto-close so the
      // user can see what was generated (and re-run / edit / cancel).
      // The rename is already committed server-side by the time we get
      // here; clicking Cancel now would leave the AI title in place.
      setValue(generated);
      setAiApplied(generated);
    } catch (e2) {
      setErr(String((e2 as Error).message));
    } finally {
      setAiBusy(false);
    }
  }

  return (
    <Dialog open={!!session} onOpenChange={(o) => !o && onClose()}>
      <DialogContent className="max-w-md p-0 gap-0 overflow-hidden">
        <form onSubmit={submitManual} className="flex flex-col gap-4 p-5">
          <div>
            <div className="text-[15px] font-semibold text-foreground">Rename session</div>
            <div className="mt-1 text-[12px] text-muted-foreground">
              Type a new name below, or let the model pick one from the conversation.
            </div>
          </div>

          <div className="flex flex-col gap-1.5">
            <label htmlFor="rename-input" className="text-[12.5px] font-medium text-foreground/85">
              Title
            </label>
            <Input
              id="rename-input"
              value={value}
              onChange={(e) => {
                setValue(e.target.value);
                setAiApplied(null);
              }}
              placeholder="Session name"
              spellCheck={false}
              autoFocus
              disabled={saveBusy || aiBusy}
            />
          </div>

          {aiApplied && (
            <div className="rounded-md border border-emerald-500/40 bg-emerald-500/[0.06] px-3 py-2 text-[12.5px] text-emerald-400">
              AI applied: <span className="font-mono">{aiApplied}</span> — click Done to close, or
              edit + Save.
            </div>
          )}
          {err && (
            <div className="rounded-md border border-destructive/40 bg-destructive/10 px-3 py-2 text-[12.5px] text-destructive">
              {err}
            </div>
          )}

          <div className="flex items-center justify-between gap-2">
            <Button
              type="button"
              variant="outline"
              onClick={submitAi}
              disabled={saveBusy || aiBusy}
              className="gap-1.5"
            >
              <Sparkles className="size-3.5" />
              {aiBusy ? 'Generating…' : aiApplied ? 'Regenerate' : 'Rename with AI'}
            </Button>
            <div className="flex items-center gap-2">
              <Button
                type="button"
                variant="outline"
                onClick={onClose}
                disabled={saveBusy || aiBusy}
              >
                {aiApplied ? 'Done' : 'Cancel'}
              </Button>
              <Button type="submit" disabled={saveBusy || aiBusy || !value.trim()}>
                {saveBusy ? 'Saving…' : 'Save'}
              </Button>
            </div>
          </div>
        </form>
      </DialogContent>
    </Dialog>
  );
}
