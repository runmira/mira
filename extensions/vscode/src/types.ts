/**
 * Minimal mirror of mira-server's WebSocket protocol.
 *
 * We only declare the frames the extension actually cares about — every
 * unknown frame is dropped silently on the client side, which keeps the
 * extension forward-compatible when the server adds new variants.
 *
 * Kept in sync with `crates/mira-server/src/protocol.rs`. If a field
 * name diverges, this file is the wrong one — the server owns the wire.
 */

export type Mode = 'plan' | 'manual' | 'auto' | 'edit' | 'yolo';

/** `session` also allows identical calls for the rest of the session. */
export type ApprovalScope = 'once' | 'session' | 'always';

/** Diff preview attached to edit/write approvals. */
export type DiffLine =
  | { tag: 'ctx' | 'add' | 'del'; text: string }
  | { tag: 'hunkgap' };

export type DiffPreview = {
  path: string;
  kind: 'edit' | 'overwrite' | 'create';
  lines: DiffLine[];
  truncated?: boolean;
};

export type ToolCall = {
  id: string;
  type: 'function';
  function: { name: string; arguments: string };
};

export type ToolResult = {
  call_id: string;
  content: string;
  is_error?: boolean;
};

/** Client → server. Discriminator is `type` with snake_case values. */
export type ClientMsg =
  | { type: 'send'; text: string }
  | { type: 'approve'; call_id: string; allow: boolean; scope?: ApprovalScope }
  | { type: 'set_model'; model: string }
  | { type: 'set_mode'; mode: Mode }
  | { type: 'interrupt' }
  | { type: 'sync' };

/** Server → client. Only the subset the extension renders is spelled
 *  out; anything else lands as `unknown`-typed and gets ignored. */
export type ServerMsg =
  | { type: 'ready'; session_id: string; model: string; mode: Mode; cwd: string }
  | { type: 'token'; text: string }
  | { type: 'tool_start'; call: ToolCall }
  | { type: 'tool_end'; result: ToolResult }
  | { type: 'turn_complete' }
  | { type: 'done' }
  | { type: 'warning'; text: string }
  | { type: 'error'; text: string }
  | { type: 'approval_request'; call: ToolCall; preview?: DiffPreview }
  | { type: string; [k: string]: unknown };
