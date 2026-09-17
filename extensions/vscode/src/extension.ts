/**
 * Mira VS Code extension — entry point.
 *
 * Registers `@mira` as a chat participant in VS Code's built-in Chat
 * sidebar (the same surface Copilot Chat / Continue use). Each request
 * is streamed to a user-run `mira serve` over WebSocket; tokens land as
 * `stream.markdown`, tool calls render as fenced code blocks so the
 * user can see what the model actually did.
 *
 * v0.1 explicit non-goals — the extension does NOT:
 *   - spawn `mira serve` itself (the user runs it; a follow-up will
 *     autostart when the binary is on PATH and the port is free)
 *   - own an approval flow (approvals silently error out today; a
 *     follow-up will surface them as `vscode.window.showQuickPick`)
 *   - render the subagent / review / PR / plugins panels (those live
 *     in the browser UI — open with `Mira: Open Web UI in Browser`)
 */

import * as vscode from 'vscode';
import { MiraClient } from './miraClient';
import type { AbortSignalLike } from './miraClient';
import type { Mode, ServerMsg, ToolCall } from './types';

const PARTICIPANT_ID = 'mira.mira';

export function activate(context: vscode.ExtensionContext) {
  const client = new MiraClient(configBaseUrl());

  // Re-read the base URL on config change so the user can point at a
  // different `mira serve` without reloading the window. Any active
  // socket is dropped so the next send opens against the new URL.
  context.subscriptions.push(
    vscode.workspace.onDidChangeConfiguration((e) => {
      if (e.affectsConfiguration('mira.baseUrl')) {
        client.reset();
      }
    }),
  );

  const participant = vscode.chat.createChatParticipant(
    PARTICIPANT_ID,
    async (request, chatContext, stream, token) => handleRequest(
      client, request, chatContext, stream, token,
    ),
  );
  participant.iconPath = vscode.Uri.joinPath(context.extensionUri, 'icon.png');
  context.subscriptions.push(participant);

  context.subscriptions.push(
    vscode.commands.registerCommand('mira.openWebUI', async () => {
      const url = configBaseUrl();
      await vscode.env.openExternal(vscode.Uri.parse(url));
    }),
    vscode.commands.registerCommand('mira.showStatus', async () => {
      const url = configBaseUrl();
      const connected = client.isConnected();
      vscode.window.showInformationMessage(
        `Mira: ${connected ? 'connected' : 'not connected'} — ${url}`,
      );
    }),
  );
}

export function deactivate() { /* participant + subscriptions torn down by the framework */ }

/* ---------- chat participant handler ---------- */

async function handleRequest(
  client: MiraClient,
  request: vscode.ChatRequest,
  _context: vscode.ChatContext,
  stream: vscode.ChatResponseStream,
  token: vscode.CancellationToken,
): Promise<vscode.ChatResult> {
  // Slash commands go through `request.command` — one-shots that don't
  // send a user turn to the backend (except `/reset` which forces a
  // fresh WS on the next send).
  if (request.command) {
    return handleSlashCommand(client, request, stream);
  }

  // Follow the active workspace folder before every send when the
  // config knob is on. Cheap PUT; no-op on the server when the folder
  // hasn't changed.
  await maybeSyncCwd(client);

  const prompt = buildPrompt(request);
  if (!prompt.trim()) {
    stream.markdown('_Empty prompt._');
    return {};
  }

  const showTools = vscode.workspace.getConfiguration('mira').get<boolean>('showToolCalls', true);
  const abort = fromCancellationToken(token);

  try {
    for await (const msg of client.send(prompt, abort)) {
      renderFrame(msg, stream, showTools);
      if (msg.type === 'done') break;
    }
  } catch (e) {
    const err = e instanceof Error ? e.message : String(e);
    stream.markdown(`\n\n---\n**Mira: connection error** — \`${escapeInline(err)}\`\n\n` +
      `Is \`mira serve\` running? The extension talks to \`${configBaseUrl()}\` — ` +
      `change it in Settings under **Mira**.`);
    return { errorDetails: { message: err } };
  }

  return {};
}

async function handleSlashCommand(
  client: MiraClient,
  request: vscode.ChatRequest,
  stream: vscode.ChatResponseStream,
): Promise<vscode.ChatResult> {
  switch (request.command) {
    case 'reset':
      await client.reset();
      stream.markdown('_Session reset. Next message opens a fresh chat._');
      return {};
    case 'mode': {
      const mode = (request.prompt || '').trim().toLowerCase();
      if (!isMode(mode)) {
        stream.markdown('_Usage: `/mode plan|manual|auto|edit|yolo`_');
        return {};
      }
      try {
        await client.control({ type: 'set_mode', mode });
        stream.markdown(`_Mode set to \`${mode}\`._`);
      } catch (e) {
        stream.markdown(`_Failed to set mode: ${escapeInline(String(e))}_`);
      }
      return {};
    }
    case 'model': {
      const model = (request.prompt || '').trim();
      if (!model) {
        stream.markdown('_Usage: `/model <name>` — e.g. `google/gemini-2.5-flash`._');
        return {};
      }
      try {
        await client.control({ type: 'set_model', model });
        stream.markdown(`_Model set to \`${model}\` for the next turn._`);
      } catch (e) {
        stream.markdown(`_Failed to set model: ${escapeInline(String(e))}_`);
      }
      return {};
    }
    default:
      stream.markdown(`_Unknown command: \`/${request.command}\`_`);
      return {};
  }
}

/* ---------- frame renderer ---------- */

/** Translate one server frame into chat-stream calls. Silent for
 *  frames the extension doesn't currently surface (turn_complete,
 *  usage, subagent_*, goal_*, memory_learned, …) — the browser UI
 *  owns those. */
function renderFrame(
  msg: ServerMsg,
  stream: vscode.ChatResponseStream,
  showTools: boolean,
): void {
  switch (msg.type) {
    case 'token':
      stream.markdown((msg as { text: string }).text);
      break;
    case 'tool_start': {
      if (!showTools) return;
      const call = (msg as { call: ToolCall }).call;
      const argSummary = summarizeArgs(call.function.arguments);
      stream.markdown(`\n\n\`\`\`\n▸ ${call.function.name}${argSummary ? ' ' + argSummary : ''}\n\`\`\`\n`);
      break;
    }
    case 'tool_end': {
      if (!showTools) return;
      const r = (msg as { result: { content: string; is_error?: boolean } }).result;
      const tag = r.is_error ? 'error' : 'result';
      const body = truncate(r.content, 800);
      stream.markdown(`\`\`\`\n[${tag}] ${body}\n\`\`\`\n`);
      break;
    }
    case 'warning':
      stream.markdown(`\n> ⚠ ${(msg as { text: string }).text}\n`);
      break;
    case 'error':
      stream.markdown(`\n> ⚠ **error:** ${(msg as { text: string }).text}\n`);
      break;
    case 'approval_request':
      // v0.1: surface it as an inline note. A follow-up will use
      // `vscode.window.showQuickPick` (Allow / Deny) and send
      // `ClientMsg::Approve` — for now the user can jump to the
      // browser UI to click the modal.
      stream.markdown(
        `\n> Mira is asking to run something in \`manual\` mode. ` +
        `Open the web UI to approve — or set \`/mode auto\` to skip prompts.\n`,
      );
      break;
    // ready / turn_complete / done / usage / *_progress / *_done → no UI here.
    default:
      break;
  }
}

/** One-line arg summary for a tool-start card. Truncated; strips
 *  newlines so multi-line prompts don't overflow. */
function summarizeArgs(raw: string): string {
  if (!raw || raw === '{}') return '';
  let obj: unknown;
  try { obj = JSON.parse(raw); } catch { return truncate(raw, 80); }
  if (obj && typeof obj === 'object') {
    const entries = Object.entries(obj as Record<string, unknown>);
    if (entries.length === 0) return '';
    // Prefer a `path` or `command` key up front — matches what the
    // policy engine reads and what the user cares about at a glance.
    const preferred = ['path', 'command', 'target', 'query', 'pattern', 'name'];
    entries.sort(([a], [b]) => {
      const ai = preferred.indexOf(a);
      const bi = preferred.indexOf(b);
      if (ai === -1 && bi === -1) return 0;
      if (ai === -1) return 1;
      if (bi === -1) return -1;
      return ai - bi;
    });
    const [k, v] = entries[0];
    const flat = typeof v === 'string' ? v : JSON.stringify(v);
    return truncate(`${k}=${flat.replace(/\s+/g, ' ')}`, 80);
  }
  return truncate(raw, 80);
}

/* ---------- utilities ---------- */

function configBaseUrl(): string {
  return vscode.workspace.getConfiguration('mira').get<string>('baseUrl', 'http://127.0.0.1:8787');
}

function isMode(s: string): s is Mode {
  return s === 'plan' || s === 'manual' || s === 'auto' || s === 'edit' || s === 'yolo';
}

/** Sync the server's cwd to the currently active workspace folder,
 *  when the follow-active-folder knob is on and there's exactly one
 *  candidate. Multi-root workspaces are ignored — the user should
 *  pick one via the browser UI. */
async function maybeSyncCwd(_client: MiraClient): Promise<void> {
  const cfg = vscode.workspace.getConfiguration('mira');
  if (!cfg.get<boolean>('followActiveFolder', true)) return;
  const folders = vscode.workspace.workspaceFolders;
  if (!folders || folders.length !== 1) return;
  const cwd = folders[0].uri.fsPath;
  try {
    const url = new URL('/api/cwd', configBaseUrl());
    await fetch(url.toString(), {
      method: 'PUT',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ cwd }),
    });
  } catch {
    // Non-fatal — the server may just be starting or unreachable.
    // The chat send will surface the connection error clearly.
  }
}

function buildPrompt(request: vscode.ChatRequest): string {
  const parts: string[] = [];
  // File / selection references (`#file:foo.ts`, `#selection`) show up
  // in `request.references`. We flatten them into a small header so
  // Mira sees them, even though the backend doesn't yet natively
  // consume them as attachments.
  const refs = request.references ?? [];
  if (refs.length > 0) {
    parts.push('## References');
    for (const ref of refs) {
      const val = ref.value as unknown;
      if (val instanceof vscode.Uri) {
        parts.push(`- ${vscode.workspace.asRelativePath(val)}`);
      } else if (val && typeof val === 'object' && 'uri' in val && 'range' in val) {
        // Location-like: a range within a file.
        const loc = val as { uri: vscode.Uri; range: vscode.Range };
        parts.push(`- ${vscode.workspace.asRelativePath(loc.uri)}:${loc.range.start.line + 1}`);
      } else if (typeof val === 'string') {
        parts.push(`- ${val}`);
      }
    }
    parts.push('');
  }
  parts.push(request.prompt);
  return parts.join('\n');
}

function fromCancellationToken(token: vscode.CancellationToken): AbortSignalLike {
  const listeners: Array<() => void> = [];
  const signal: AbortSignalLike = {
    get aborted() { return token.isCancellationRequested; },
    onAbort(cb) { listeners.push(cb); },
  };
  token.onCancellationRequested(() => {
    for (const l of listeners) l();
  });
  return signal;
}

function truncate(s: string, max: number): string {
  if (s.length <= max) return s;
  return s.slice(0, max) + '…';
}

function escapeInline(s: string): string {
  return s.replace(/`/g, '\\`');
}
