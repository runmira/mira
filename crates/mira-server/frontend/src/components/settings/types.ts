import type { AcpAgentStatus, MemoryView, Mode, SettingsView } from '../../types';
import { type GithubReturn } from '../Integrations';
import { type SettingsSectionId } from './sections';
export type SurfaceProps = {
  /** Which section to render — driven by the sidebar. */
  section: SettingsSectionId;
  onSectionChange: (id: SettingsSectionId) => void;
  onSaved: (v: SettingsView) => void;
  /** Called when the user hits "Back to App" or otherwise leaves
   *  settings — the caller pops back to the previous main view. */
  onExit: () => void;
  /** Bumps whenever the backend broadcasts `skills_reloaded` (fs
   *  watcher detected a `SKILL.md` change). Threaded into the Skills
   *  panel so its list re-fetches automatically without the user
   *  clicking Reload. */
  skillsVersion?: number;
  /** Set after GitHub sends the user back from installing the app. */
  githubReturn?: GithubReturn;

  // -------- external ACP agents --------
  //
  // Kept out of `SettingsView` deliberately: a provider is server-side
  // configuration, while an agent's binary path and launch args describe how
  // to run a third-party binary on *this* machine. Persisted client-side.
  acpAgents?: AcpAgentStatus[];
  acpRefreshing?: boolean;
  acpDriver?: string | null;
  acpError?: string | null;
  onAcpRefresh?: () => void;
  onAcpStart?: (kind: string, resume?: string | null) => void;
};

export type Draft = {
  providerName: string;
  baseUrl: string;
  apiKey: string;
  showReplaceKey: boolean;
  model: string;
  smallModel: string;
  mode: Mode;
  maxTokens: string;
  keyValues: Record<string, string>; // key name → new pending value
  keyEditing: Record<string, boolean>; // which keys are in edit mode
  memory: MemoryView;
};

export const EMPTY_MEMORY: MemoryView = {
  auto_extract: true,
  tools_enabled: true,
  inject_context: true,
  extractor_model: null,
};

export const EMPTY_DRAFT: Draft = {
  providerName: 'openrouter',
  baseUrl: '',
  apiKey: '',
  showReplaceKey: false,
  model: '',
  smallModel: '',
  mode: 'manual',
  maxTokens: '',
  keyValues: {},
  keyEditing: {},
  memory: EMPTY_MEMORY,
};
