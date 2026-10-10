import {
  Activity,
  Bot,
  Brain,
  Bug,
  ChartColumn,
  Cog,
  Cpu,
  Download,
  FileDiff,
  FlaskConical,
  FolderOpen,
  Globe2,
  Keyboard,
  MessageCircleQuestion,
  MessageSquarePlus,
  Moon,
  Palette,
  PanelLeftClose,
  Pencil,
  Plug,
  ScanSearch,
  Smartphone,
  Smile,
  Sparkles,
  SquareTerminal,
  Sun,
  Zap,
} from 'lucide-react';
import type { PaletteAction } from '../components/CommandPalette';
import { shortcutLabelForCommand } from '../lib/keybindings';
import { resolveTheme, setThemePref } from '../lib/theme';
import type { WorkspaceRuntime } from './WorkspaceRuntime';
export function useWorkspacePalette(
  context: Pick<
    WorkspaceRuntime,
    | 'acpDriver'
    | 'toggleSidebar'
    | 'setTerminal'
    | 'keybindings'
    | 'openSettings'
    | 'setSettingsSection'
    | 'onNewChat'
    | 'setPickerOpen'
    | 'setMainView'
    | 'setReviewOpen'
    | 'setInspectOpen'
    | 'runReview'
    | 'setReportOpen'
    | 'terminalOpen'
    | 'openToolPane'
    | 'setImportOpen'
  >,
) {
  const {
    acpDriver,
    toggleSidebar,
    setTerminal,
    keybindings,
    openSettings,
    setSettingsSection,
    onNewChat,
    setPickerOpen,
    setMainView,
    setReviewOpen,
    setInspectOpen,
    runReview,
    setReportOpen,
    terminalOpen,
    openToolPane,
    setImportOpen,
  } = context;
  const keyFor = (c: Parameters<typeof shortcutLabelForCommand>[1]) =>
    shortcutLabelForCommand(keybindings, c);
  const goSettings = (id: import('../components/Settings').SettingsSectionId) => () => {
    openSettings();
    setSettingsSection(id);
  };
  const paletteActions: PaletteAction[] = [
    {
      id: 'new',
      group: 'Chat',
      label: 'New chat',
      icon: MessageSquarePlus,
      shortcut: keyFor('chat.new'),
      run: () => void onNewChat(),
    },
    {
      id: 'folder',
      group: 'Chat',
      label: 'Open folder…',
      icon: FolderOpen,
      keywords: ['project', 'cwd', 'directory'],
      run: () => setPickerOpen(true),
    },
    {
      id: 'model',
      group: 'Chat',
      label: acpDriver ? 'Switch model or agent…' : 'Switch model…',
      icon: Cpu,
      keywords: ['provider', 'agent', 'claude', 'codex', 'engine'],
      run: () => {
        setMainView('chat');
        window.dispatchEvent(new Event('mira:open-model-picker'));
      },
    },
    {
      id: 'changes',
      group: 'Chat',
      label: 'Review changes',
      icon: FileDiff,
      shortcut: keyFor('review.toggle'),
      keywords: ['diff', 'git'],
      run: () => setReviewOpen(true),
    },
    {
      id: 'context',
      group: 'Chat',
      label: "What's in the context window",
      icon: Brain,
      keywords: ['tokens', 'compact', 'inspector'],
      run: () => setInspectOpen(true),
    },
    {
      id: 'review',
      group: 'Chat',
      label: 'Ask Iris to review the changes',
      icon: ScanSearch,
      keywords: ['code review', 'second opinion', 'reviewer'],
      run: () => void runReview(''),
    },
    {
      id: 'sidebar',
      group: 'View',
      label: 'Toggle sidebar',
      icon: PanelLeftClose,
      shortcut: keyFor('sidebar.toggle'),
      run: toggleSidebar,
    },
    {
      id: 'terminal',
      group: 'View',
      label: 'Toggle terminal',
      icon: SquareTerminal,
      shortcut: keyFor('terminal.toggle'),
      run: () => setTerminal(!terminalOpen),
    },
    {
      id: 'browser',
      group: 'View',
      label: 'Open browser',
      icon: Globe2,
      keywords: ['chrome', 'web'],
      run: () => {
        setMainView('chat');
        openToolPane('browser');
      },
    },
    {
      id: 'whiteboard',
      group: 'View',
      label: 'Open whiteboard',
      icon: Pencil,
      keywords: ['sketch', 'draw'],
      run: () => {
        setMainView('chat');
        openToolPane('whiteboard');
      },
    },
    {
      id: 'aside',
      group: 'View',
      label: 'Ask aside',
      icon: MessageCircleQuestion,
      keywords: ['side question', 'btw', 'quick question'],
      run: () => {
        setMainView('chat');
        openToolPane('aside');
      },
    },
    {
      id: 'processes',
      group: 'View',
      label: 'Open processes',
      icon: SquareTerminal,
      keywords: ['dev server', 'ports', 'logs', 'background'],
      run: () => {
        setMainView('chat');
        openToolPane('processes');
      },
    },
    {
      id: 'tests',
      group: 'View',
      label: 'Open tests',
      icon: FlaskConical,
      keywords: ['run tests', 'failures', 'test runner'],
      run: () => {
        setMainView('chat');
        openToolPane('tests');
      },
    },
    {
      id: 'activity',
      group: 'View',
      label: 'Open activity',
      icon: Activity,
      keywords: ['timeline', 'history', 'restore', 'changes'],
      run: () => {
        setMainView('chat');
        openToolPane('activity');
      },
    },
    {
      id: 'devices',
      group: 'View',
      label: 'Open device preview',
      icon: Smartphone,
      keywords: ['responsive', 'mobile', 'iphone', 'ipad'],
      run: () => {
        setMainView('chat');
        openToolPane('devices');
      },
    },
    {
      id: 's-general',
      group: 'Settings',
      label: 'General settings',
      icon: Cog,
      shortcut: keyFor('settings.toggle'),
      run: goSettings('general'),
    },
    {
      id: 's-appearance',
      group: 'Settings',
      label: 'Appearance',
      icon: Palette,
      keywords: ['theme', 'light', 'dark', 'motion'],
      run: goSettings('appearance'),
    },
    {
      id: 'theme-toggle',
      group: 'View',
      label: resolveTheme() === 'dark' ? 'Switch to light theme' : 'Switch to dark theme',
      icon: resolveTheme() === 'dark' ? Sun : Moon,
      keywords: ['theme', 'appearance', 'light', 'dark', 'mode'],
      run: () => setThemePref(resolveTheme() === 'dark' ? 'light' : 'dark'),
    },
    {
      id: 's-provider',
      group: 'Settings',
      label: 'Providers & API keys',
      icon: Plug,
      run: goSettings('provider'),
    },
    {
      id: 's-agents',
      group: 'Settings',
      label: 'External agents',
      icon: Bot,
      keywords: ['claude code', 'codex'],
      run: goSettings('agents'),
    },
    {
      id: 'import-chats',
      group: 'Chat',
      label: 'Import chats from Claude Code or Codex',
      icon: Download,
      keywords: ['history', 'migrate', 'bring'],
      run: () => setImportOpen(true),
    },
    {
      id: 'report-problem',
      group: 'Help',
      label: 'Report a problem',
      icon: Bug,
      keywords: ['bug', 'diagnostics', 'logs', 'crash', 'issue', 'feedback'],
      run: () => setReportOpen(true),
    },
    {
      id: 's-subagents',
      group: 'Settings',
      label: 'Subagents',
      icon: Smile,
      keywords: ['scout', 'iris', 'atlas', 'bolt', 'quill', 'sentry', 'faces'],
      run: goSettings('subagents'),
    },
    {
      id: 's-usage',
      group: 'Settings',
      label: 'Usage & cost',
      icon: ChartColumn,
      keywords: ['tokens', 'spend', 'limits'],
      run: goSettings('usage'),
    },
    { id: 's-memory', group: 'Settings', label: 'Memory', icon: Brain, run: goSettings('memory') },
    {
      id: 's-skills',
      group: 'Settings',
      label: 'Skills',
      icon: Sparkles,
      run: goSettings('skills'),
    },
    { id: 's-hooks', group: 'Settings', label: 'Hooks', icon: Zap, run: goSettings('hooks') },
    {
      id: 's-keys',
      group: 'Settings',
      label: 'Keyboard shortcuts',
      icon: Keyboard,
      keywords: ['keybindings'],
      run: goSettings('keybindings'),
    },
  ];
  return { keyFor, goSettings, paletteActions };
}
