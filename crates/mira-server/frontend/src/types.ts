// Wire types — mirror crates/mira-server/src/protocol.rs.

export type Mode = 'plan' | 'manual' | 'auto' | 'edit' | 'yolo';

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

export type Role = 'system' | 'user' | 'assistant' | 'tool';

export type Message = {
  role: Role;
  content?: string | null;
  tool_calls?: ToolCall[];
  tool_call_id?: string | null;
  name?: string | null;
};

export type DiffKind = 'edit' | 'overwrite' | 'create';

export type DiffLine =
  | { tag: 'ctx'; text: string }
  | { tag: 'add'; text: string }
  | { tag: 'del'; text: string }
  | { tag: 'hunkgap' };

export type DiffPreview = {
  path: string;
  kind: DiffKind;
  lines: DiffLine[];
};

export type ServerMsg =
  | { type: 'ready'; session_id: string; model: string; mode: Mode; cwd: string; history: Message[] }
  | { type: 'token'; text: string }
  | { type: 'tool_start'; call: ToolCall }
  | { type: 'tool_end'; result: ToolResult }
  | { type: 'turn_complete' }
  | { type: 'done' }
  | { type: 'approval_request'; call: ToolCall; preview?: DiffPreview | null }
  | { type: 'warning'; text: string }
  | { type: 'model_changed'; model: string }
  | { type: 'mode_changed'; mode: Mode }
  | { type: 'error'; text: string };

export type ClientMsg =
  | { type: 'send'; text: string }
  | { type: 'approve'; call_id: string; allow: boolean }
  | { type: 'set_model'; model: string }
  | { type: 'set_mode'; mode: Mode }
  | { type: 'interrupt' }
  | { type: 'sync' };

export type ProviderView = {
  name: string;
  base_url?: string | null;
  api_key_masked?: string | null;
  has_api_key: boolean;
  api_key_env?: string | null;
};

export type SettingsView = {
  default_provider?: string | null;
  default_model?: string | null;
  default_mode?: string | null;
  max_tokens?: number | null;
  temperature?: number | null;
  providers: ProviderView[];
  configured: boolean;
  config_path: string;
};

export type ProviderUpdate = {
  name: string;
  base_url?: string | null;
  /** Omit to keep existing key. Empty string clears. Any value sets. */
  api_key?: string;
  api_key_env?: string | null;
};

// The server distinguishes "key missing" (leave alone) from "key present
// and null" (clear). Fields you always send stay as `string | null`;
// fields you sometimes want to leave alone use the optional `?`.
export type SettingsUpdate = {
  default_provider?: string | null;
  default_model?: string | null;
  default_mode?: string | null;
  max_tokens?: number | null;
  temperature?: number | null;
  providers?: ProviderUpdate[];
};

export type SessionSummary = {
  id: string;
  model: string;
  cwd: string;
  created_at: number;
  updated_at: number;
  message_count: number;
  first_user_message?: string | null;
  active: boolean;
};
