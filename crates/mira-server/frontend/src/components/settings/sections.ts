import {
  Bot,
  Brain,
  ChartColumn,
  Cog,
  Info,
  Keyboard,
  Palette,
  Plug,
  Search,
  Smile,
  Sparkle,
  Zap,
} from 'lucide-react';

export type SettingsSectionId = 'general' | 'appearance' | 'provider' | 'agents' | 'subagents' | 'usage' | 'memory' | 'skills' | 'hooks' | 'keybindings' | 'search' | 'integrations' | 'about';

/** Section metadata, shared with the Sidebar so it can render the same nav
 *  in its "settings mode" (the settings surface is inline in the main pane,
 *  not a dialog — the sidebar drives section selection). Kept out of
 *  Settings.tsx so the Sidebar doesn't pull the lazily loaded settings
 *  surface into the main chunk. */
export const SETTINGS_SECTIONS: {
  id: SettingsSectionId;
  label: string;
  icon: React.ComponentType<{ className?: string }>;
}[] = [
  { id: 'general',     label: 'General',     icon: Cog },
  { id: 'appearance',  label: 'Appearance',  icon: Palette },
  { id: 'provider',    label: 'Provider',    icon: Plug },
  // External coding agents. A sibling of Provider, not a child: they
  // authenticate and bill separately, so a Provider key does not apply.
  { id: 'agents',      label: 'External agents', icon: Bot },
  // Mira's own helpers — a different thing from external agents, hence
  // a different word everywhere.
  { id: 'subagents',   label: 'Subagents',   icon: Smile },
  { id: 'usage',       label: 'Usage',       icon: ChartColumn },
  { id: 'memory',      label: 'Memory',      icon: Brain },
  { id: 'skills',      label: 'Skills',      icon: Sparkle },
  { id: 'hooks',       label: 'Hooks',       icon: Zap },
  { id: 'keybindings', label: 'Keyboard shortcuts', icon: Keyboard },
  { id: 'search',      label: 'Search & keys', icon: Search },
  { id: 'integrations', label: 'Integrations', icon: Plug },
  { id: 'about',       label: 'About',       icon: Info },
];
