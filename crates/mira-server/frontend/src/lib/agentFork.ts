import type { SessionEngine } from '../types';

/** Which fork actions an engine offers. Mira's own providers fork from any
 *  message. External agents fork natively, as negotiated by their runtime:
 *  `turn` can stop at a chosen completed turn, `session` copies the whole
 *  conversation only, and an unknown or older agent offers neither. */
export function forkActions(engine: SessionEngine | null | undefined): { fromMessage: boolean; wholeChat: boolean } {
  if (!engine || engine.kind !== 'agent') return { fromMessage: true, wholeChat: false };
  const scope = engine.capabilities?.fork ?? 'unavailable';
  return { fromMessage: scope === 'turn', wholeChat: scope !== 'unavailable' };
}
