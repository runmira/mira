/**
 * Feed for the terminal panel's read-only "Agent" tab: the agent's shell
 * commands (`bash`, `run_background`) and their live output, written as
 * terminal text so they read like a session you're watching.
 *
 * App's websocket handler pushes events in; the Agent tab subscribes.
 * Recent output is buffered so a tab opened mid-session can replay it.
 */

const SHELL_TOOLS = new Set(['bash', 'run_background']);
const MAX_BUFFER = 256 * 1024;

type Listener = (chunk: string) => void;

const listeners = new Set<Listener>();
/** Live shell calls → their tool name. */
const shellCalls = new Map<string, string>();
const streamed = new Set<string>();
let buffer = '';

/** Terminals want CRLF; tool output uses LF. */
const crlf = (s: string) => s.replace(/\r?\n/g, '\r\n');

function write(chunk: string) {
  buffer = (buffer + chunk).slice(-MAX_BUFFER);
  for (const l of listeners) l(chunk);
}

export function subscribe(fn: Listener): () => void {
  listeners.add(fn);
  return () => listeners.delete(fn);
}

export function snapshot(): string {
  return buffer;
}

/** A new session was loaded: start the Agent tab over. */
export function reset() {
  shellCalls.clear();
  streamed.clear();
  buffer = '';
  for (const l of listeners) l('\x1bc');
}

export function commandStart(callId: string, tool: string, argsJson: string) {
  if (!SHELL_TOOLS.has(tool)) return;
  let command = argsJson;
  try {
    command = (JSON.parse(argsJson) as { command?: string }).command ?? argsJson;
  } catch { /* keep raw args */ }
  shellCalls.set(callId, tool);
  const bg = tool === 'run_background' ? ' \x1b[2m(background)\x1b[0m' : '';
  // A blank line separates commands, but not before the first one.
  write(`${buffer ? '\r\n' : ''}\x1b[38;5;110m❯\x1b[0m \x1b[1m${crlf(command)}\x1b[0m${bg}\r\n`);
}

export function outputLine(callId: string, line: string) {
  if (!shellCalls.has(callId)) return;
  streamed.add(callId);
  write(`${crlf(line)}\r\n`);
}

export function commandEnd(callId: string, content: string, isError: boolean) {
  if (!shellCalls.has(callId)) return;
  // Tools that didn't stream progress still show their output once.
  if (!streamed.has(callId) && content.trim()) write(`${crlf(content.trimEnd())}\r\n`);
  if (isError) write('\x1b[31m✗ command failed\x1b[0m\r\n');
  // A background process keeps printing after its tool result lands.
  if (shellCalls.get(callId) === 'run_background') return;
  shellCalls.delete(callId);
  streamed.delete(callId);
}
