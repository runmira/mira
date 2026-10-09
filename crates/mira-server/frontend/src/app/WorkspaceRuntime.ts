import type { OptionDescriptor } from '../api';
import { type MessageRef, type Restored } from '../api';
import { type AskUserDecision } from '../components/AskUserCard';
import type { PaletteAction } from '../components/CommandPalette';
import { type PendingApproval, type QueuedComposerMessage } from '../components/Composer';
import { type InfoNotice } from '../components/InfoNoticeHost';
import type { ActivityTurn } from '../components/panes/ActivityPane';
import { type ToolPaneKind } from '../components/panes/toolPanes';
import type { SubagentTab } from '../components/SubagentPanel';
import { type MinimapItem } from '../components/TimelineMinimap';
import { type MessageActions } from '../components/transcript/EntryView';
import { type RecoveryAction } from '../components/UsageRecoveryCard';
import type { UsageRingData } from '../components/UsageRing';
import type { PostureOption } from '../lib/agentPostures';
import type { BackgroundProcess } from '../lib/backgroundProcesses';
import { shortcutLabelForCommand, type ShortcutMatchContext } from '../lib/keybindings';
import { type SessionActivityUpdate } from '../lib/sessionActivity';
import type { Turn } from '../lib/turnActivity';
import { type Entry } from '../transcript/entries';
import type {
  AcpConfigOption,
  ApprovalScope,
  AskUserProposal,
  DiffPreview,
  Mode,
  PlanProposal,
  PlanStep,
  ServerMsg,
  SessionEngine,
  SettingsView,
} from '../types';
import type { useWorkspaceState } from './useWorkspaceState';

import type React from 'react';

/** Typed contract between workspace state, commands, events, and views. */
type WorkspaceState = ReturnType<typeof useWorkspaceState>;
export interface WorkspaceRuntime extends WorkspaceState {
  playPing: () => void;
  loadOlderHistory: () => void;
  acpDriver: string | null;
  applyEngine: (next: SessionEngine | null) => void;
  acpModelOption: AcpConfigOption | null;
  acpDescriptors: OptionDescriptor[] | null;
  pushInfoNotice: (notice: InfoNotice) => void;
  dismissInfoNotice: (id: string) => void;
  acpDriverName: string;
  loadEngines: () => void;
  requestAcpStatus: () => void;
  startAcpAgent: (
    kind: string,
    resume?: string | null,
    model?: string | null,
    instance?: string | null,
  ) => void;
  attachSession: (id: string) => void;
  switchCwd: (path: string) => Promise<void>;
  openProjectPicker: () => Promise<void>;
  forkAcpAgent: () => void;
  compactAcpAgent: (focus?: string) => void;
  clearThinkingIdle: () => void;
  scheduleThinkingIdle: () => void;
  inspectOpenMounted: boolean;
  paletteOpenMounted: boolean;
  pickerOpenMounted: boolean;
  openAgentSettings: () => void;
  toggleSidebar: () => void;
  closeDrawer: () => void;
  onCtxOpenChange: (v: boolean) => void;
  reviewMounted: boolean;
  setTerminal: (v: boolean) => void;
  shortcutContext: () => ShortcutMatchContext;
  backgroundProcesses: {
    processes: BackgroundProcess[];
    error: string | null;
    stop: (id: number) => Promise<void>;
    stopping: ReadonlySet<string>;
  };
  chatColRef: (el: HTMLDivElement | null) => void;
  refreshRepo: () => void;
  noteScrollInput: () => void;
  jumpToLatest: () => void;
  onPaneScroll: () => void;
  updateSessionActivity: (update: SessionActivityUpdate) => void;
  drainTokens: () => void;
  flushTokens: () => void;
  startTurnUsage: (turn: number) => void;
  flushNativeFrames: () => void;
  onMessage: (msg: ServerMsg) => void;
  handleMessage: (msg: ServerMsg) => void;
  replyToSecret: (promptId: string, value: string | null) => void;
  replyToPlan: (callId: string, approved: boolean, steps?: PlanStep[], note?: string) => void;
  replyToAskUser: (callId: string, decision: AskUserDecision) => void;
  replyToSubagentReview: (
    parentCallId: string,
    promptId: string,
    approved: boolean,
    note?: string,
  ) => void;
  runReview: (args: string) => Promise<void>;
  runPrReview: (owner: string, repo: string, number: number) => Promise<void>;
  decideApproval: (callId: string, allow: boolean, scope?: ApprovalScope, rules?: string[]) => void;
  pendingApprovals: PendingApproval[];
  agentPostures: PostureOption[];
  usageRing: UsageRingData;
  pendingPlans: { callId: string; proposal: PlanProposal }[];
  pendingQuestions: { callId: string; proposal: AskUserProposal }[];
  firstPendingCallId: string;
  queuedFor: (id?: string) => QueuedComposerMessage[];
  setQueuedForSession: (
    id: string,
    update: (items: QueuedComposerMessage[]) => QueuedComposerMessage[],
  ) => void;
  queueMessage: (text: string, images?: { media_type: string; data: string }[]) => void;
  removeQueuedMessage: (id: string) => void;
  editQueuedMessage: (item: QueuedComposerMessage, text: string) => Promise<void>;
  changeRecovery: (id: string, action: RecoveryAction) => Promise<void>;
  reorderQueuedMessage: (id: string, beforeId: string | null) => Promise<void>;
  steerQueuedMessage: (id: string) => void;
  sendNow: (
    text: string,
    images?: { media_type: string; data: string }[],
    transmit?: boolean,
    inputId?: string,
  ) => void;
  onSend: (text: string, images?: { media_type: string; data: string }[]) => void;
  drainQueuedMessage: () => void;
  messageRefAt: (userIdx: number) => MessageRef | null;
  askRestore: (userIdx: number) => Promise<void>;
  forkAt: (userIdx: number) => Promise<void>;
  doRestore: (run: () => Promise<Restored>, verb: string) => Promise<void>;
  onResend: (userIdx: number, text: string) => void;
  stableToggleTurn: (idx: number) => void;
  stableDecide: (
    callId: string,
    allow: boolean,
    scope?: ApprovalScope | undefined,
    rules?: string[] | undefined,
  ) => void;
  stablePlanReply: (
    callId: string,
    approved: boolean,
    steps?: PlanStep[] | undefined,
    note?: string | undefined,
  ) => void;
  stableAskUserReply: (callId: string, decision: AskUserDecision) => void;
  stableOpenAgent: (callId: string) => void;
  stableOpenFile: (
    ref: string,
    diff: DiffPreview | null,
    atLine?: number | null | undefined,
  ) => void;
  stableSetMode: (m: Mode) => void;
  stableFailureSettings: () => void;
  editMessage: (entry: Entry, text: string) => void;
  restoreMessage: (entry: Entry) => undefined;
  forkMessage: (entry: Entry) => undefined;
  retryMessage: (entry: Entry) => void;
  messageActions: MessageActions;
  onSetMode: (m: Mode) => void;
  onSetAcpMode: (modeId: string, acknowledgePrivileged?: boolean) => void;
  onSetModel: (m: string, instance?: string | null) => void;
  onSetModelOption: (id: string, value: string) => void;
  onSetGoal: (condition: string, maxIterations?: number) => void;
  onClearGoal: () => void;
  onCompact: (focus: string) => void;
  onNewChat: () => Promise<void>;
  keyFor: (c: Parameters<typeof shortcutLabelForCommand>[1]) => string | null;
  goSettings: (id: import('../components/Settings').SettingsSectionId) => () => void;
  paletteActions: PaletteAction[];
  settingsHandler: (v: SettingsView) => void;
  isEmpty: boolean;
  turns: Turn[];
  lastTurnIdx: number;
  secondOpinionKey: string;
  lastTurnEdited: boolean;
  showSecondOpinion: boolean;
  retireSecondOpinion: () => void;
  minimapItems: MinimapItem[];
  activityTurns: ActivityTurn[];
  jumpToMinimapTurn: (id: string) => void;
  ctxHasContent: boolean;
  ctxRoomy: boolean;
  ctxReserve: boolean;
  ctxPill: boolean;
  subagentTabs: SubagentTab[];
  openAgentTab: (callId: string) => void;
  hydrateSubagentIfNeeded: (callId: string) => Promise<void>;
  closeAgentTab: (callId: string) => void;
  closeSubagentPanel: () => void;
  openFileTab: (ref: string, diff: DiffPreview | null, atLine?: number | null) => void;
  closeFileTab: (tabId: string) => void;
  closeToolTab: (tabId: string) => void;
  openToolPane: (kind: ToolPaneKind) => void;
  closeAnyTab: (id: string) => void;
  sendWhiteboardToChat: (pngDataUrl: string) => Promise<void>;
  handlePanelResizeStart: (e: React.MouseEvent) => void;
  panelOpen: boolean;
  hiddenTitleBar: boolean;
  toggleTurn: (idx: number) => void;
  sidebarCol: string;
  rightCol: string;
}
