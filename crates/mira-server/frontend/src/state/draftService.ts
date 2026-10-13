import { createStore } from '@tanstack/react-store';
import { cachedDraft,changeRichDraft,clearRichDraft,flushRichDrafts,restoreRichDraft,type RichDraft } from '../lib/richComposerDrafts';
import type { RuntimeScope } from './createAppRuntime';
export function createDraftService(scope: RuntimeScope) {
  const store = createStore(new Map<string, RichDraft>());
  const initial = new Map<string, RichDraft>();
  const revisions = new Map<string, number>();
  let disposed = false;
  const deliveries = new Map<string, { key: string; draft: RichDraft }>();
  const key = (session: string | null | undefined) => JSON.stringify([scope.backend, scope.account, session || '__unsaved__']);
  function get(id: string): RichDraft {
    const saved = store.get().get(id); if (saved) return saved;
    if (!initial.has(id)) initial.set(id, cachedDraft(id));
    while (initial.size > 40) initial.delete(initial.keys().next().value!);
    return initial.get(id)!;
  }
  function remember(id: string, draft: RichDraft) {
    const next = new Map(store.get()); next.delete(id); next.set(id, draft);
    // Rich storage handles dirty attachment retention. Keep recent empty/text-only projections bounded here.
    for (const [candidate, entry] of next) {
      if (next.size <= 40) break;
      if (candidate !== id && !entry.attachments.length && !entry.images.length) next.delete(candidate);
    }
    store.setState(() => next);
  }
  return {
    key, get,
    awaiting(id: string) { const current = get(id); return [...deliveries.values()].some(pending => pending.key === id && pending.draft === current); },
    markDelivery(id: string, input: string) { deliveries.set(input, { key: id, draft: get(id) }); },
    confirmDelivery(input: string) {
      const pending = deliveries.get(input); deliveries.delete(input);
      if (!pending || disposed || get(pending.key) !== pending.draft) return;
      remember(pending.key, { text: '', attachments: [], images: [] }); clearRichDraft(pending.key);
      if (typeof window !== 'undefined') window.dispatchEvent(new CustomEvent('mira:draft-delivered', { detail: pending.key }));
    },
    source: { get: () => store.get(), subscribe: store.subscribe.bind(store) },
    update(id: string, draft: RichDraft, immediate = false) {
      if (disposed) return;
      revisions.set(id, (revisions.get(id) ?? 0) + 1); remember(id, draft);
      changeRichDraft(id, draft, immediate);
    },
    async restore(id: string) {
      const revision = revisions.get(id) ?? 0;
      const stored = await restoreRichDraft(id);
      if (!disposed && revision === (revisions.get(id) ?? 0)) remember(id, stored);
      return get(id);
    },
    clear(id: string) {
      if (disposed) return;
      revisions.set(id, (revisions.get(id) ?? 0) + 1);
      remember(id, { text: '', attachments: [], images: [] }); clearRichDraft(id);
    },
    dispose() { disposed = true; void flushRichDrafts(); initial.clear(); revisions.clear(); deliveries.clear(); },
  };
}
