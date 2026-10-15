import { jsonOrThrow } from './request';

/** An ACP agent's terminal sign-in (OpenCode), run in a Mira terminal. */
export type TerminalAuthView = {
  attempt_id: string | null;
  phase: 'idle' | 'running' | 'refreshing' | 'succeeded' | 'failed' | 'cancelled' | 'unsupported';
  terminal_id: string | null;
  label: string | null;
  command: string | null;
  exit_code: number | null;
  error: string | null;
  models: number | null;
  ready: boolean | null;
};
const url = (instance: string) => `/api/engines/${encodeURIComponent(instance)}/terminal-auth`;
export async function getTerminalAuth(instance: string): Promise<TerminalAuthView> {
  return jsonOrThrow(await fetch(url(instance), { cache: 'no-store' }), 'Check sign-in');
}
export async function startTerminalAuth(instance: string): Promise<TerminalAuthView> {
  return jsonOrThrow(await fetch(url(instance), { method: 'POST', cache: 'no-store' }), 'Start sign-in');
}
export async function cancelTerminalAuth(instance: string): Promise<TerminalAuthView> {
  return jsonOrThrow(await fetch(url(instance), { method: 'DELETE', cache: 'no-store' }), 'Cancel sign-in');
}
