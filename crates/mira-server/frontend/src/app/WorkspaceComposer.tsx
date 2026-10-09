import { History, X } from 'lucide-react';
import { appendMemory, applyUndo, listEngines, undoRestore } from '../api';
import { Composer } from '../components/Composer';
import { CONTEXT_PANEL_RESERVE } from '../components/ContextPanel';
import { InfoNoticeHost } from '../components/InfoNoticeHost';
import { SecretPrompt } from '../components/SecretPrompt';
import { MIRA_MODE_TO_POSTURE } from '../lib/agentPostures';
import { loadModelOptions } from '../lib/models';
import { cn } from '../lib/utils';
import type { WorkspaceRuntime } from './WorkspaceRuntime';
export function WorkspaceComposer(
  context: Pick<
    WorkspaceRuntime,
    | 'ctxReserve'
    | 'restoreNote'
    | 'setRestoreNote'
    | 'doRestore'
    | 'status'
    | 'busy'
    | 'mode'
    | 'model'
    | 'providerName'
    | 'cwd'
    | 'usage'
    | 'rateLimit'
    | 'onSend'
    | 'queueMessage'
    | 'queuedFor'
    | 'sessionId'
    | 'removeQueuedMessage'
    | 'editQueuedMessage'
    | 'reorderQueuedMessage'
    | 'steerQueuedMessage'
    | 'onSetMode'
    | 'onSetModel'
    | 'engine'
    | 'engines'
    | 'acpAgents'
    | 'acpStatusPending'
    | 'requestAcpStatus'
    | 'acpDriver'
    | 'acpConfig'
    | 'acpDescriptors'
    | 'onSetModelOption'
    | 'wsRef'
    | 'startAcpAgent'
    | 'openAgentSettings'
    | 'compactAcpAgent'
    | 'forkAcpAgent'
    | 'attachSession'
    | 'acpModes'
    | 'agentPostures'
    | 'setPendingAcpMode'
    | 'openProjectPicker'
    | 'environment'
    | 'environments'
    | 'envSwitching'
    | 'setEnvSwitching'
    | 'setBusy'
    | 'busyRef'
    | 'setThinking'
    | 'onNewChat'
    | 'openSettings'
    | 'runReview'
    | 'onSetGoal'
    | 'onClearGoal'
    | 'onCompact'
    | 'goal'
    | 'skills'
    | 'usageRing'
    | 'pendingApprovals'
    | 'decideApproval'
    | 'visibleApprovalRef'
    | 'ruleEditorCallId'
    | 'setRuleEditorCallId'
    | 'pendingPlans'
    | 'pendingQuestions'
    | 'secretRequests'
    | 'replyToSecret'
    | 'infoNotices'
    | 'dismissInfoNotice'
    | 'completedUpdatesRef'
    | 'setEngines'
    | 'replyToPlan'
    | 'replyToAskUser'
    | 'acpCommands'
    | 'commands'
  >,
) {
  const {
    ctxReserve,
    restoreNote,
    setRestoreNote,
    doRestore,
    status,
    busy,
    mode,
    model,
    providerName,
    cwd,
    usage,
    rateLimit,
    onSend,
    queueMessage,
    queuedFor,
    sessionId,
    removeQueuedMessage,
    editQueuedMessage,
    reorderQueuedMessage,
    steerQueuedMessage,
    onSetMode,
    onSetModel,
    engine,
    engines,
    acpAgents,
    acpStatusPending,
    requestAcpStatus,
    acpDriver,
    acpConfig,
    acpDescriptors,
    onSetModelOption,
    wsRef,
    startAcpAgent,
    openAgentSettings,
    compactAcpAgent,
    forkAcpAgent,
    attachSession,
    acpModes,
    agentPostures,
    setPendingAcpMode,
    openProjectPicker,
    environment,
    environments,
    envSwitching,
    setEnvSwitching,
    setBusy,
    busyRef,
    setThinking,
    onNewChat,
    openSettings,
    runReview,
    onSetGoal,
    onClearGoal,
    onCompact,
    goal,
    skills,
    usageRing,
    pendingApprovals,
    decideApproval,
    visibleApprovalRef,
    ruleEditorCallId,
    setRuleEditorCallId,
    pendingPlans,
    pendingQuestions,
    secretRequests,
    replyToSecret,
    infoNotices,
    dismissInfoNotice,
    completedUpdatesRef,
    setEngines,
    replyToPlan,
    replyToAskUser,
    acpCommands,
    commands,
  } = context;
  return (
    <div
      className="pb-safe shrink-0 transition-[padding-right] duration-200"
      style={{ paddingRight: ctxReserve ? CONTEXT_PANEL_RESERVE : 0 }}
    >
      {restoreNote && (
        <div className="mx-auto mb-2 flex w-full max-w-3xl animate-fade-in items-center gap-2 rounded-lg border border-border/60 bg-secondary/70 px-3 py-1.5 text-[12.5px]">
          <History
            className={cn(
              'size-3.5 shrink-0',
              restoreNote.error ? 'text-amber-400' : 'text-muted-foreground',
            )}
          />
          <span
            className={cn(
              'min-w-0 flex-1',
              restoreNote.error ? 'text-amber-200' : 'text-foreground/85',
            )}
          >
            {restoreNote.text}
          </span>
          {restoreNote.undo && (
            <button
              type="button"
              onClick={() => {
                const undo = restoreNote.undo!;
                setRestoreNote(null);
                void doRestore(() => undoRestore(undo), 'Put back');
              }}
              className="shrink-0 rounded px-1.5 py-0.5 text-[12px] font-medium text-foreground/80 hover:bg-fg/[0.06] hover:text-foreground"
            >
              Undo
            </button>
          )}
          <button
            type="button"
            onClick={() => setRestoreNote(null)}
            aria-label="Dismiss"
            className="shrink-0 rounded p-0.5 text-muted-foreground/60 hover:text-foreground"
          >
            <X className="size-3.5" />
          </button>
        </div>
      )}
      <Composer
        disabled={status !== 'open'}
        busy={busy}
        mode={mode}
        model={model}
        providerName={providerName}
        cwd={cwd}
        usage={usage}
        rateLimit={rateLimit}
        onSend={onSend}
        onQueueMessage={queueMessage}
        queuedMessages={queuedFor(sessionId)}
        onRemoveQueuedMessage={removeQueuedMessage}
        onEditQueuedMessage={editQueuedMessage}
        onReorderQueuedMessage={reorderQueuedMessage}
        onSteerQueuedMessage={steerQueuedMessage}
        onSetMode={onSetMode}
        onSetModel={onSetModel}
        engine={engine}
        engines={engines}
        agents={acpAgents}
        agentsChecking={acpStatusPending}
        onCheckAgents={requestAcpStatus}
        agentConfig={acpDriver ? acpConfig : null}
        agentDescriptors={acpDescriptors}
        onSetModelOption={onSetModelOption}
        onPickProvider={(instance, m) => {
          wsRef.current?.send({
            type: 'set_model',
            model: m,
            instance: instance ?? null,
            options: loadModelOptions(m, instance ?? null),
          });
        }}
        onPickAgent={(driver, m) => {
          // Already this chat's agent: only the model can change, and
          // that is an option on the running agent, not a restart
          // with a new setup.
          if (acpDriver === driver) {
            if (m && m !== engine?.model) onSetModel(m);
            return;
          }
          // Its saved setup (Settings → Agents) is the engine's, on the
          // server. Prefer the engine instance named after the driver (its
          // default), else the only external instance of that driver.
          const external = (engines ?? []).filter(
            (e) => e.flavor === 'external' && e.driver === driver,
          );
          const instance =
            (
              external.find((e) => e.instance === driver) ??
              (external.length === 1 ? external[0] : undefined)
            )?.instance ?? null;
          startAcpAgent(driver, null, m, instance);
        }}
        onConfigureAgents={openAgentSettings}
        sessionId={sessionId}
        onAgentCompact={compactAcpAgent}
        onAgentFork={forkAcpAgent}
        onAgentReverted={() => attachSession(sessionId)}
        onAcpModes={acpModes?.available ?? null}
        onAcpCurrentMode={acpModes?.current ?? null}
        onPickAgentMode={(m) => {
          if (!acpDriver) return;
          const key = MIRA_MODE_TO_POSTURE[m];
          const opt = agentPostures.find((o) => o.posture.key === key);
          // Unmapped or already active: nothing to confirm.
          if (!opt || opt.current) return;
          setPendingAcpMode({ modeId: opt.modeId });
        }}
        agentDriving={acpDriver != null}
        onOpenPicker={() => void openProjectPicker()}
        onCwdSwitched={(_path, id) => {
          if (id) attachSession(id);
        }}
        environment={environment}
        environments={environments}
        envSwitching={envSwitching}
        onSwitchEnvironment={(target) => {
          setEnvSwitching(`switching to ${target}…`);
          wsRef.current?.send({ type: 'environment', target });
        }}
        onInterrupt={() => {
          wsRef.current?.send({ type: 'interrupt' });
          // Settle the composer now; the server's turn end follows
          // (within a few seconds even for an agent that never says).
          setBusy(false);
          busyRef.current = false;
          setThinking(false);
        }}
        onNewChat={onNewChat}
        onOpenSettings={() => openSettings()}
        onRunReview={runReview}
        onSetGoal={onSetGoal}
        onClearGoal={onClearGoal}
        onCompact={onCompact}
        goal={goal}
        onRemember={async (scope, text) => {
          const r = await appendMemory(scope, text);
          return `remembered → ${r.path}`;
        }}
        onUndo={async (count) => {
          const r = await applyUndo(count);
          if (r.applied.length === 0) return 'nothing to undo';
          return `reverted ${r.applied.length} write${r.applied.length === 1 ? '' : 's'}`;
        }}
        skills={skills}
        usageRing={usageRing}
        pendingApproval={pendingApprovals[0] ?? null}
        pendingApprovalCount={pendingApprovals.length}
        onAllowAllPending={() => {
          for (const a of pendingApprovals) decideApproval(a.callId, true, 'once');
        }}
        onActiveApprovalChange={(callId) => {
          visibleApprovalRef.current = callId;
        }}
        pendingApprovals={pendingApprovals}
        ruleEditorCallId={ruleEditorCallId}
        onRuleEditorCancel={() => setRuleEditorCallId(null)}
        pendingPlans={pendingPlans}
        pendingQuestions={pendingQuestions}
        notices={[
          ...(status !== 'open'
            ? [
                {
                  id: 'connection',
                  kind: 'connection' as const,
                  title: 'Reconnecting to Mira',
                  content: (
                    <p role="status" className="px-3 pb-2 text-[12px] text-muted-foreground">
                      Your draft is kept here. Sending and decisions are available when the
                      connection returns.
                    </p>
                  ),
                },
              ]
            : []),
          ...secretRequests.map((request) => ({
            id: `secret:${request.promptId}`,
            kind: 'question' as const,
            title: `An agent needs ${request.name}`,
            content: <SecretPrompt request={request} onReply={replyToSecret} />,
          })),
          ...infoNotices
            .filter((notice) => notice.kind === 'persistent')
            .map((notice) => ({
              id: notice.id,
              kind: 'update' as const,
              title: notice.title,
              detail: notice.body ?? undefined,
              content: (
                <InfoNoticeHost
                  inline
                  notices={[notice]}
                  onDismiss={dismissInfoNotice}
                  onAgentUpdated={(kind) => {
                    completedUpdatesRef.current.add(kind);
                    requestAcpStatus();
                    void listEngines()
                      .then((view) => setEngines(view.engines))
                      .catch(() => {});
                  }}
                />
              ),
            })),
        ]}
        onDecide={(callId, allow, scope, rules) => decideApproval(callId, allow, scope, rules)}
        onPlanReply={replyToPlan}
        onAskUserReply={replyToAskUser}
        commands={
          // An agent's advertised slash commands take over while it is
          // driving the session — Mira's own command list would offer
          // prompts the agent has never heard of.
          acpDriver && acpCommands.length > 0
            ? acpCommands.map((name) => ({
                name,
                description: '',
                argument_hint: null,
                source: 'acp',
                kind: 'command' as const,
                origin: {
                  kind: 'user' as const,
                  key: 'acp',
                  label: acpDriver,
                  icon_url: null,
                  homepage: null,
                },
              }))
            : commands
        }
      />
    </div>
  );
}
