import { useState } from 'react';
import { ImportChats } from '../ImportChats';
import { Collapse } from '../ui/Collapse';
/** Settings → External agents: bring Claude Code and Codex chats over.
 *  Scans only when opened — a scan reads every transcript. */
export function ImportChatsCard() {
  const [open, setOpen] = useState(false);
  return (
    <div className="mt-8 rounded-2xl border border-border/70 bg-fg/[0.02] p-4">
      <div className="flex items-start gap-3">
        <div className="min-w-0 flex-1">
          <div className="text-[14px] font-semibold text-foreground">
            Bring chats from other agents
          </div>
          <p className="mt-1 max-w-[60ch] text-[12.5px] leading-relaxed text-muted-foreground">
            Import your Claude Code and Codex conversations, grouped by project. Each opens on the
            same agent and continues where it left off.
          </p>
        </div>
        {!open && (
          <button
            type="button"
            onClick={() => setOpen(true)}
            className="shrink-0 rounded-lg bg-mira-blue px-3 py-1.5 text-[12.5px] font-medium text-white"
          >
            Find chats
          </button>
        )}
      </div>
      <Collapse open={open}>
        <div className="mt-4">
          <ImportChats
            onDone={() => {
              setOpen(false);
              window.dispatchEvent(new Event('mira:sessions-changed'));
            }}
          />
        </div>
      </Collapse>
    </div>
  );
}
