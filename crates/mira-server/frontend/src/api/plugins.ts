import { jsonOrThrow, post } from './request';
/** A server entry in `.mcp.json` shape. */
export type McpServerConfig =
  | {
      type?: 'stdio';
      command: string;
      args?: string[];
      env?: Record<string, string>;
      cwd?: string | null;
    }
  | {
      type?: 'http' | 'sse';
      url: string;
      headers?: Record<string, string>;
      auth?: string | null;
      oauth?: { clientId?: string; clientSecret?: string; scopes?: string[] } | null;
    };

export type McpScope =
  { kind: 'user' } | { kind: 'project' } | { kind: 'local' } | { kind: 'plugin'; plugin: string };

export type McpStatus =
  | { state: 'connecting' }
  | { state: 'connected' }
  | { state: 'needs_auth' }
  | { state: 'needs_approval' }
  | { state: 'rejected' }
  | { state: 'disabled' }
  | { state: 'needs_setup'; variables: string[] }
  | { state: 'failed'; message: string };

export type McpToolView = {
  name: string;
  remote_name: string;
  description: string;
  read_only: boolean;
  enabled: boolean;
};

/** How MCP tools reach the model. */
export type ToolLoading = 'all' | 'on_demand' | 'auto';

export type McpPromptView = {
  name: string;
  description: string | null;
  arguments: { name: string; description: string | null; required: boolean }[];
};

export type McpResourceView = {
  uri: string;
  name: string;
  description: string | null;
  mime_type: string | null;
};

export type McpServerView = {
  name: string;
  scope: McpScope;
  transport: 'stdio' | 'http' | 'sse';
  target: string;
  source: string | null;
  status: McpStatus;
  tools: McpToolView[];
  resources: McpResourceView[];
  prompts: McpPromptView[];
  server_name: string | null;
  server_version: string | null;
  instructions: string | null;
  can_sign_in: boolean;
  signed_in: boolean;
  /** `${VAR}`s in the definition with no value. */
  missing_vars: string[];
  log_path: string | null;
  config: McpServerConfig;
};

export type McpProblem = { source: string; server: string | null; message: string };

export type McpListView = {
  servers: McpServerView[];
  problems: McpProblem[];
  user_config_path: string;
  project: string | null;
  /** Names of saved `${VAR}` values (never the values). */
  saved_variables: string[];
  tool_loading: ToolLoading;
  /** Whether tools go through search_mcp_tools / call_mcp_tool right now. */
  tools_on_demand: boolean;
};

export type WriteScope = 'user' | 'project' | 'local';

export async function listMcp(): Promise<McpListView> {
  return jsonOrThrow(await fetch('/api/mcp', { cache: 'no-store' }), 'Loading MCP servers');
}

export function saveMcpServer(req: {
  name: string;
  scope: WriteScope;
  config: McpServerConfig;
  replaces?: { name: string; scope: WriteScope } | null;
}): Promise<McpListView> {
  return post('/api/mcp/servers', req, 'Saving the server');
}

export async function deleteMcpServer(name: string, scope: WriteScope): Promise<McpListView> {
  const r = await fetch(`/api/mcp/servers/${encodeURIComponent(name)}?scope=${scope}`, {
    method: 'DELETE',
  });
  return jsonOrThrow(r, 'Removing the server');
}

export const mcpUrl = (name: string, action: string) =>
  `/api/mcp/servers/${encodeURIComponent(name)}/${action}`;

export const reconnectMcp = (name: string) =>
  post<McpListView>(mcpUrl(name, 'reconnect'), {}, 'Reconnecting');

export const setMcpEnabled = (name: string, enabled: boolean) =>
  post<McpListView>(mcpUrl(name, 'enabled'), { enabled }, enabled ? 'Enabling' : 'Disabling');

export const setMcpApproval = (name: string, approve: boolean) =>
  post<McpListView>(mcpUrl(name, 'approval'), { approve }, approve ? 'Approving' : 'Rejecting');

export const signInMcp = (name: string) =>
  post<{ url: string }>(mcpUrl(name, 'sign-in'), {}, 'Starting sign-in');

/** Save (or with an empty value, remove) a `${VAR}` value used by server
 *  definitions, like a plugin's `GITHUB_PERSONAL_ACCESS_TOKEN`. */
export const setMcpVariable = (name: string, value: string | null) =>
  post<McpListView>('/api/mcp/variables', { name, value }, 'Saving the token');

export const setMcpToolEnabled = (tool: string, enabled: boolean) =>
  post<McpListView>(
    '/api/mcp/tools/enabled',
    { tool, enabled },
    enabled ? 'Enabling the tool' : 'Disabling the tool',
  );

export const setMcpToolLoading = (mode: ToolLoading) =>
  post<McpListView>('/api/mcp/tool-loading', { mode }, 'Changing tool loading');

export const signOutMcp = (name: string) =>
  post<McpListView>(mcpUrl(name, 'sign-out'), {}, 'Signing out');

/** Where a command or skill comes from, for grouping it in palettes. */
export type Origin = {
  kind: 'plugin' | 'mcp' | 'user' | 'project';
  /** Groups items from the same place (`plugin:notion`, `mcp:linear`). */
  key: string;
  /** "Notion", "Commit commands". */
  label: string;
  icon_url: string | null;
  homepage: string | null;
};

export type CommandInfo = {
  name: string;
  description: string;
  argument_hint: string | null;
  /** `user`, `project`, `plugin:<name>` or `mcp:<server>`. */
  source: string;
  /** A Markdown command, or an MCP server's prompt. */
  kind: 'command' | 'prompt';
  origin: Origin;
};

export async function listCommands(): Promise<CommandInfo[]> {
  return jsonOrThrow(await fetch('/api/commands', { cache: 'no-store' }), 'Loading commands');
}

export type MarketplaceView = {
  name: string;
  source: string;
  kind: 'github' | 'git' | 'directory' | 'url';
  description: string | null;
  owner: string | null;
  plugin_count: number;
  path: string;
  updated_at: number;
  error: string | null;
};

export type CatalogEntry = {
  id: string;
  name: string;
  display_name: string | null;
  icon_url: string | null;
  marketplace: string;
  description: string | null;
  version: string | null;
  author: string | null;
  category: string | null;
  tags: string[];
  keywords: string[];
  homepage: string | null;
  source: string;
  installable: boolean;
  installed: boolean;
  enabled: boolean;
  installed_version: string | null;
};

export type InstalledPluginView = {
  id: string;
  name: string;
  display_name: string | null;
  marketplace: string;
  version: string;
  description: string | null;
  author: string | null;
  enabled: boolean;
  path: string;
  updated_at: number;
  commands: string[];
  agents: string[];
  skills: string[];
  mcp_servers: string[];
  hooks: string[];
  lsp_servers: string[];
  problems: string[];
};

export type PluginsOverview = {
  marketplaces: MarketplaceView[];
  catalog: CatalogEntry[];
  installed: InstalledPluginView[];
  suggested_marketplaces: { source: string; name: string; description: string }[];
};

export type PluginComponents = {
  commands: string[];
  agents: string[];
  skill_dirs: string[];
  skills: string[];
  hooks: string[];
  mcp_server_names: string[];
  lsp_servers: string[];
  problems: string[];
};

export type PluginDetail = CatalogEntry & {
  license: string | null;
  repository: string | null;
  components: PluginComponents | null;
  command_names: string[];
  agent_names: string[];
  readme: string | null;
  path: string | null;
};

export async function getPlugins(): Promise<PluginsOverview> {
  return jsonOrThrow(await fetch('/api/plugins', { cache: 'no-store' }), 'Loading plugins');
}

export async function getPluginDetail(id: string): Promise<PluginDetail> {
  return jsonOrThrow(
    await fetch(`/api/plugins/detail/${encodeURIComponent(id)}`, { cache: 'no-store' }),
    'Loading the plugin',
  );
}

export const installPlugin = (id: string) =>
  post<PluginsOverview>('/api/plugins/install', { id }, 'Installing');

export const uninstallPlugin = (id: string) =>
  post<PluginsOverview>(`/api/plugins/${encodeURIComponent(id)}/uninstall`, {}, 'Uninstalling');

export const setPluginEnabled = (id: string, enabled: boolean) =>
  post<PluginsOverview>(
    `/api/plugins/${encodeURIComponent(id)}/enabled`,
    { enabled },
    enabled ? 'Enabling' : 'Disabling',
  );

export const addMarketplace = (source: string) =>
  post<PluginsOverview>('/api/plugins/marketplaces', { source }, 'Adding the marketplace');

export const updateMarketplace = (name: string) =>
  post<PluginsOverview>(
    `/api/plugins/marketplaces/${encodeURIComponent(name)}/update`,
    {},
    'Updating the marketplace',
  );

export async function removeMarketplace(name: string): Promise<PluginsOverview> {
  const r = await fetch(`/api/plugins/marketplaces/${encodeURIComponent(name)}`, {
    method: 'DELETE',
  });
  return jsonOrThrow(r, 'Removing the marketplace');
}
