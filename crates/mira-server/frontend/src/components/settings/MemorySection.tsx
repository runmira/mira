import { SectionInput } from '@/components/ui/input';
import { Brain } from 'lucide-react';
import type { MemoryUpdate, MemoryView } from '../../types';
import { TNote, TRow, TSection, TSwitch } from './SettingsFields';
import { Draft } from './types';
/* ---------- section: memory ---------- */

export function MemorySection({
  draft,
  setDraft,
}: {
  draft: Draft;
  setDraft: (u: (d: Draft) => Draft) => void;
}) {
  function update<K extends keyof MemoryView>(key: K, value: MemoryView[K]) {
    setDraft((d) => ({ ...d, memory: { ...d.memory, [key]: value } }));
  }

  return (
    <div className="flex flex-col gap-2.5">
      <TSection
        icon={<Brain className="size-3.5" />}
        title="Memory"
        description="Cross-session memory: user + project MIRA.md, plus the agent-written episodic stream at .mira/episodic.jsonl. Changes apply to new chats — click New chat after saving to try them."
      >
        <TRow
          title="Inject memory into prompt"
          description="Add a live 'memory' section to every model request. Turn off to shrink the system prompt back to the pre-memory baseline — useful for isolating whether the injected content is confusing the model."
          control={
            <TSwitch
              checked={draft.memory.inject_context}
              onChange={(v) => update('inject_context', v)}
              label="Inject memory into prompt"
            />
          }
        />

        <TRow
          title="Enable memory tools"
          description="Registers memory_read / memory_search / memory_append / memory_edit / memory_remember. Turn off to remove them from the model's tool list — useful when the extra tools distract simple questions."
          control={
            <TSwitch
              checked={draft.memory.tools_enabled}
              onChange={(v) => update('tools_enabled', v)}
              label="Enable memory tools"
            />
          }
        />

        <TRow
          title="Auto-extract facts after each turn"
          description="Background pass that mines each finished turn for durable facts and appends them to .mira/episodic.jsonl. Only fires when at least one tool call succeeded."
          control={
            <TSwitch
              checked={draft.memory.auto_extract}
              onChange={(v) => update('auto_extract', v)}
              label="Auto-extract facts after each turn"
            />
          }
        />

        <TRow
          title="Extractor model"
          description="Model id used for the extraction call. Leave blank to reuse the session's active model (works but is expensive). Point at your provider's cheap tier — e.g. claude-haiku-4-5, gpt-5-nano, deepseek-chat — for negligible per-round cost."
          control={
            <SectionInput
              value={draft.memory.extractor_model ?? ''}
              onChange={(e) => update('extractor_model', e.target.value || null)}
              placeholder="(uses session model)"
              spellCheck={false}
              className="h-8 text-[13px] sm:w-64"
            />
          }
        />
      </TSection>

      <TNote>
        <span>
          Settings are saved to <code className="font-mono">mira.yaml</code>. To apply them to the
          running server, <b>restart</b> Mira (Ctrl+C then run the command again). Hot-reload
          without restart is not yet wired.
        </span>
      </TNote>
    </div>
  );
}

/**
 * Diff `next` against `current` and return a `MemoryUpdate` with only the
 * changed fields, or `undefined` if nothing changed. Sending only deltas
 * lets the backend distinguish "user cleared this" (present-with-null)
 * from "user didn't touch it" (absent).
 */
export function memoryPatchFor(current: MemoryView, next: MemoryView): MemoryUpdate | undefined {
  const patch: MemoryUpdate = {};
  let any = false;
  if (current.auto_extract !== next.auto_extract) {
    patch.auto_extract = next.auto_extract;
    any = true;
  }
  if (current.tools_enabled !== next.tools_enabled) {
    patch.tools_enabled = next.tools_enabled;
    any = true;
  }
  if (current.inject_context !== next.inject_context) {
    patch.inject_context = next.inject_context;
    any = true;
  }
  const curModel = current.extractor_model ?? null;
  const nextModel = next.extractor_model ?? null;
  if (curModel !== nextModel) {
    patch.extractor_model = nextModel;
    any = true;
  }
  return any ? patch : undefined;
}
