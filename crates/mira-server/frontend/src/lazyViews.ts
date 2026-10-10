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

// Right-panel panes (rendered inside the lazy SubagentPanel).
export const AsidePane = lazyNamed(() => import('./components/panes/AsidePane'), 'AsidePane');
export const ActivityPane = lazyNamed(() => import('./components/panes/ActivityPane'), 'ActivityPane');
export const DevicesPane = lazyNamed(() => import('./components/panes/DevicesPane'), 'DevicesPane');
export const ProcessesPane = lazyNamed(() => import('./components/panes/ProcessesPane'), 'ProcessesPane');
export const TestsPane = lazyNamed(() => import('./components/panes/TestsPane'), 'TestsPane');

// Dialogs and one-off surfaces, mounted on first open.
export const GetStarted = lazyNamed(() => import('./components/onboarding/GetStarted'), 'GetStarted');
export const ImportChats = lazyNamed(() => import('./components/ImportChats'), 'ImportChats');
export const ContextInspector = lazyNamed(() => import('./components/ContextInspector'), 'ContextInspector');
export const CommandPalette = lazyNamed(() => import('./components/CommandPalette'), 'CommandPalette');
export const FolderPicker = lazyNamed(() => import('./components/FolderPicker'), 'FolderPicker');
export const GoalPanel = lazyNamed(() => import('./components/GoalPanel'), 'GoalPanel');
export const ReportProblem = lazyNamed(() => import('./components/ReportProblem'), 'ReportProblem');
export const SessionPeek = lazyNamed(() => import('./components/SessionPeek'), 'SessionPeek');
