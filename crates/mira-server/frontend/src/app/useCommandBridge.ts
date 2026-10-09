import { useMemo, useRef } from 'react';
import type { WorkspaceRuntime } from './WorkspaceRuntime';
export type WorkspaceCommands = Pick<
  WorkspaceRuntime,
  | 'loadOlderHistory'
  | 'applyEngine'
  | 'pushInfoNotice'
  | 'dismissInfoNotice'
  | 'attachSession'
  | 'switchCwd'
  | 'openProjectPicker'
  | 'forkAcpAgent'
  | 'compactAcpAgent'
  | 'clearThinkingIdle'
  | 'scheduleThinkingIdle'
  | 'openAgentSettings'
  | 'shortcutContext'
  | 'updateSessionActivity'
  | 'drainTokens'
  | 'flushTokens'
  | 'startTurnUsage'
  | 'flushNativeFrames'
  | 'onMessage'
  | 'handleMessage'
  | 'replyToSecret'
  | 'replyToPlan'
  | 'replyToAskUser'
  | 'replyToSubagentReview'
  | 'runReview'
  | 'runPrReview'
  | 'decideApproval'
  | 'queuedFor'
  | 'setQueuedForSession'
  | 'queueMessage'
  | 'removeQueuedMessage'
  | 'editQueuedMessage'
  | 'changeRecovery'
  | 'reorderQueuedMessage'
  | 'steerQueuedMessage'
  | 'sendNow'
  | 'onSend'
  | 'drainQueuedMessage'
  | 'messageRefAt'
  | 'askRestore'
  | 'forkAt'
  | 'doRestore'
  | 'onResend'
  | 'onSetMode'
  | 'onSetAcpMode'
  | 'onSetModel'
  | 'onSetModelOption'
  | 'onSetGoal'
  | 'onClearGoal'
  | 'onCompact'
  | 'onNewChat'
  | 'openAgentTab'
  | 'hydrateSubagentIfNeeded'
  | 'closeAgentTab'
  | 'closeSubagentPanel'
  | 'openFileTab'
  | 'closeFileTab'
  | 'closeToolTab'
  | 'openToolPane'
  | 'closeAnyTab'
  | 'sendWhiteboardToChat'
  | 'handlePanelResizeStart'
  | 'toggleTurn'
>;
const COMMAND_NAMES = [
  'loadOlderHistory',
  'applyEngine',
  'pushInfoNotice',
  'dismissInfoNotice',
  'attachSession',
  'switchCwd',
  'openProjectPicker',
  'forkAcpAgent',
  'compactAcpAgent',
  'clearThinkingIdle',
  'scheduleThinkingIdle',
  'openAgentSettings',
  'shortcutContext',
  'updateSessionActivity',
  'drainTokens',
  'flushTokens',
  'startTurnUsage',
  'flushNativeFrames',
  'onMessage',
  'handleMessage',
  'replyToSecret',
  'replyToPlan',
  'replyToAskUser',
  'replyToSubagentReview',
  'runReview',
  'runPrReview',
  'decideApproval',
  'queuedFor',
  'setQueuedForSession',
  'queueMessage',
  'removeQueuedMessage',
  'editQueuedMessage',
  'changeRecovery',
  'reorderQueuedMessage',
  'steerQueuedMessage',
  'sendNow',
  'onSend',
  'drainQueuedMessage',
  'messageRefAt',
  'askRestore',
  'forkAt',
  'doRestore',
  'onResend',
  'onSetMode',
  'onSetAcpMode',
  'onSetModel',
  'onSetModelOption',
  'onSetGoal',
  'onClearGoal',
  'onCompact',
  'onNewChat',
  'openAgentTab',
  'hydrateSubagentIfNeeded',
  'closeAgentTab',
  'closeSubagentPanel',
  'openFileTab',
  'closeFileTab',
  'closeToolTab',
  'openToolPane',
  'closeAnyTab',
  'sendWhiteboardToChat',
  'handlePanelResizeStart',
  'toggleTurn',
] as const;

/** Stable delegates let socket callbacks invoke the current render's commands. */
export function useCommandBridge() {
  const commands = useRef<WorkspaceCommands | null>(null);
  const stable = useMemo(
    () =>
      Object.fromEntries(
        COMMAND_NAMES.map((name) => [
          name,
          (...args: unknown[]) => {
            const command = commands.current?.[name];
            if (!command) throw new Error('Workspace commands are not bound');
            return Reflect.apply(command, undefined, args);
          },
        ]),
      ) as WorkspaceCommands,
    [],
  );
  return {
    stable,
    bind: (next: WorkspaceCommands) => {
      commands.current = next;
    },
  };
}
