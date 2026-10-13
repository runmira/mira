export type TransitionMetadata = { type: string; session: string | null; generation: number; at: number };
/** Deliberately copies only metadata. Content and tool arguments cannot enter this trace. */
export function createDiagnostics(enabled = false) {
  let disposed = false;
  const events: Readonly<TransitionMetadata>[] = [];
  const counts: Record<string, number> = {};
  return {
    record(input: TransitionMetadata) {
      if (!enabled || disposed) return;
      const type = input.type.slice(0, 64);
      counts[type] = (counts[type] ?? 0) + 1;
      events.push(Object.freeze({ type, session: input.session, generation: input.generation, at: input.at }));
      if (events.length > 128) events.shift();
    },
    read() { return { events: [...events], counts: { ...counts } }; },
    dispose() { disposed = true; events.length = 0; for (const type of Object.keys(counts)) delete counts[type]; },
    clear() { events.length = 0; for (const type of Object.keys(counts)) delete counts[type]; },
  };
}
