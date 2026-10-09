import { EntryBoundary } from '../components/EntryBoundary';
import { MessageActionsContext } from '../components/transcript/EntryView';
import { TurnView } from '../components/transcript/TurnView';
import { VirtualTranscript } from '../components/VirtualTranscript';
import type { WorkspaceRuntime } from './WorkspaceRuntime';
export function WorkspaceTranscript(
  context: Pick<
    WorkspaceRuntime,
    | 'messageActions'
    | 'paneRef'
    | 'sessionId'
    | 'historyCursor'
    | 'loadOlderHistory'
    | 'turns'
    | 'historyOffsetRef'
    | 'turnDiffs'
    | 'turnTimings'
    | 'turnUsage'
    | 'turnModels'
    | 'model'
    | 'expandedTurns'
    | 'stableToggleTurn'
    | 'stableDecide'
    | 'stablePlanReply'
    | 'stableAskUserReply'
    | 'stableOpenAgent'
    | 'stableOpenFile'
    | 'busy'
    | 'skills'
    | 'mode'
    | 'stableSetMode'
    | 'recoveryBySession'
    | 'stableFailureSettings'
    | 'status'
    | 'changeRecovery'
  >,
) {
  const {
    messageActions,
    paneRef,
    sessionId,
    historyCursor,
    loadOlderHistory,
    turns,
    historyOffsetRef,
    turnDiffs,
    turnTimings,
    turnUsage,
    turnModels,
    model,
    expandedTurns,
    stableToggleTurn,
    stableDecide,
    stablePlanReply,
    stableAskUserReply,
    stableOpenAgent,
    stableOpenFile,
    busy,
    skills,
    mode,
    stableSetMode,
    recoveryBySession,
    stableFailureSettings,
    status,
    changeRecovery,
  } = context;
  return (
    <MessageActionsContext.Provider value={messageActions}>
      <VirtualTranscript
        pane={paneRef}
        identity={sessionId ?? 'new'}
        hasOlder={!!historyCursor}
        onNeedOlder={loadOlderHistory}
      >
        {turns.map((turn, i) => (
          <EntryBoundary
            key={
              turn.user
                ? `${sessionId}:turn-${turn.user.transcriptTurnIndex ?? i + historyOffsetRef.current}`
                : `${sessionId}:preamble-${i}`
            }
          >
            <TurnView
              diffSummary={
                turn.user?.kind === 'msg'
                  ? turnDiffs.find(
                      (summary) =>
                        summary.text ===
                          (turn.user?.kind === 'msg' ? turn.user.msg.content : null) &&
                        summary.occurrence ===
                          turns
                            .slice(i + 1)
                            .filter(
                              (other) =>
                                other.user?.kind === 'msg' &&
                                other.user.msg.content === summary.text,
                            ).length,
                    )
                  : undefined
              }
              minimapId={`turn-${turn.user?.transcriptTurnIndex ?? i + historyOffsetRef.current}`}
              turn={turn}
              timing={
                turnTimings.get(turn.user?.providerTurnIndex ?? i + historyOffsetRef.current) ??
                null
              }
              usage={
                turnUsage.get(turn.user?.providerTurnIndex ?? i + historyOffsetRef.current) ?? null
              }
              model={
                turnModels.get(turn.user?.providerTurnIndex ?? i + historyOffsetRef.current) ??
                model
              }
              index={turn.user?.transcriptTurnIndex ?? i + historyOffsetRef.current}
              expanded={expandedTurns.has(
                turn.user?.transcriptTurnIndex ?? i + historyOffsetRef.current,
              )}
              onToggle={stableToggleTurn}
              onDecide={stableDecide}
              onPlanReply={stablePlanReply}
              onAskUserReply={stableAskUserReply}
              onOpenAgent={stableOpenAgent}
              onOpenFile={stableOpenFile}
              isActive={busy && i === turns.length - 1}
              skills={skills}
              mode={mode}
              onSetMode={stableSetMode}
              approvalViaDialog={false}
              recovery={i === turns.length - 1 ? recoveryBySession[sessionId]?.at(-1) : undefined}
              failureRetryEnabled={i === turns.length - 1 && !!turn.user}
              onFailureSettings={stableFailureSettings}
              recoveryDisabled={status !== 'open' || busy}
              onRecoveryAction={changeRecovery}
              offscreenOk={false}
            />
          </EntryBoundary>
        ))}
      </VirtualTranscript>
    </MessageActionsContext.Provider>
  );
}
