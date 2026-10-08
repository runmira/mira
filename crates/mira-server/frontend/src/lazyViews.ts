import { lazyNamed } from './lib/lazy';

// Views that aren't on screen at first load get their own chunks (issue #72).
// Defined here, not in App, so the controls that open them (Sidebar,
// UserCard, toolbar) can preload them on hover without importing App.
export const SettingsSurface = lazyNamed(() => import('./components/Settings'), 'SettingsSurface');
export const PluginsPanel = lazyNamed(() => import('./components/Plugins'), 'PluginsPanel');
export const PullRequestPanel = lazyNamed(() => import('./components/PullRequestPanel'), 'PullRequestPanel');
export const ReviewChanges = lazyNamed(() => import('./components/ReviewChanges'), 'ReviewChanges');
export const SubagentPanel = lazyNamed(() => import('./components/SubagentPanel'), 'SubagentPanel');
export const TerminalPanel = lazyNamed(() => import('./components/TerminalPanel'), 'TerminalPanel');
