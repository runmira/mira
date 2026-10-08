import type { ClientMsg, ServerMsg } from '../types';

type Mutation = Extract<ClientMsg, { type: 'edit_queued_input' | 'reorder_queued_input' | 'update_limit_recovery' }>;
type Request = Mutation extends infer T ? T extends Mutation ? Omit<T, 'request_id'> : never : never;

/** Queue edits require an acknowledgement; never replay them after a disconnect. */
export class QueueMutations {
  private connected = false;
  private pending = new Map<string, { session: string; resolve: () => void; reject: (error: Error) => void; timer: ReturnType<typeof setTimeout> }>();

  setConnected(connected: boolean) {
    this.connected = connected;
    if (!connected) this.clear('Connection lost. Your edit is still here; reconnect and check the queue before saving again.');
  }

  clear(message: string) {
    for (const request of this.pending.values()) {
      clearTimeout(request.timer);
      request.reject(new Error(message));
    }
    this.pending.clear();
  }

  request(input: Request, send: (message: ClientMsg) => void): Promise<void> {
    if (!this.connected) return Promise.reject(new Error('Reconnect before changing queued messages.'));
    if (this.pending.size >= 32) return Promise.reject(new Error('Wait for the current queue changes to finish.'));
    const id = crypto.randomUUID();
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        this.pending.delete(id);
        reject(new Error('Save was not confirmed. Your edit is still here; check the queue before trying again.'));
      }, 15000);
      this.pending.set(id, { session: input.session_id, resolve, reject, timer });
      try { send({ ...input, request_id: id } as Mutation); }
      catch (error) {
        clearTimeout(timer);
        this.pending.delete(id);
        reject(error);
      }
    });
  }

  receive(message: Extract<ServerMsg, { type: 'queue_mutation_result' }>) {
    const request = this.pending.get(message.request_id);
    if (!request || request.session !== message.session_id) return;
    clearTimeout(request.timer);
    this.pending.delete(message.request_id);
    if (message.error) request.reject(new Error(message.error));
    else request.resolve();
  }
}
