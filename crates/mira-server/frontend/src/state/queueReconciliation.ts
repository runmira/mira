import type { QueuedComposerMessage } from '../components/Composer';
/** Keep unconfirmed writes recoverable. Missing from a snapshot does not prove rejection. */
export function reconcileQueue(previous: QueuedComposerMessage[], server: QueuedComposerMessage[], delivered: ReadonlySet<string> = new Set()) {
  const ids = new Set(server.map(item => item.id));
  return [...server.map(item => ({ ...item, accepted: true })), ...previous.filter(item => !item.accepted && !item.fingerprint && !ids.has(item.id) && !delivered.has(item.id))
    .map(item => ({ ...item, error: 'Queue delivery is unconfirmed. Check the transcript before retrying.' }))];
}
