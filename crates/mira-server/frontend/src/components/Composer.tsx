import { cn } from '@/lib/utils';
import { m } from 'framer-motion';
import { ArrowUp, Square } from 'lucide-react';
import { captureScreenshot } from '../lib/screenCapture';
import { ComposerNoticeStack } from './ComposerNoticeStack';
import { EnginePicker } from './EnginePicker';
import { FilePicker } from './FilePicker';
import { ImageAttachmentDetails } from './ImageAttachmentDetails';
import { ImageLightbox } from './ImageLightbox';
import { MentionInput } from './MentionInput';
import { QuoteCard } from './QuoteCard';
import {
  AGENT_APPROVAL_MODES,
  AttachMenu,
  ModePicker,
  MODES,
  ProjectChip,
} from './composer/ComposerMenus';
import { ComposerContextDock, GoalChip, GoalComposeChip, PlanChip } from './composer/ContextChips';
import { ApprovalQueue, EmbeddedApprovalCard } from './composer/EmbeddedApprovalCard';
import { EmbeddedAskUserCard } from './composer/EmbeddedAskUserCard';
import { EmbeddedPlanCard } from './composer/EmbeddedPlanCard';
import { QueuedMessageStack } from './composer/QueuedMessageStack';
import { OriginIcon, SlashButton, SlashPalette } from './composer/SlashPalette';
import { AttachmentChip } from './composer/attachments';
import { Props } from './composer/types';
import { useComposerController } from './composer/useComposerController';
export function Composer(props: Props) {
  const {
    disabled,
    busy,
    mode,
    model,
    providerName,
    cwd,
    environment,
    environments,
    envSwitching,
    onSwitchEnvironment,
    queuedMessages = [],
    onRemoveQueuedMessage,
    onEditQueuedMessage,
    onReorderQueuedMessage,
    onSteerQueuedMessage,
    onSetMode,
    onSetModel,
    onSetModelOption,
    onPickAgentMode,
    agentDriving,
    onOpenPicker,
    onCwdSwitched,
    onInterrupt,
    onClearGoal,
    goal,
    engine,
    engines,
    agents,
    agentsChecking,
    onCheckAgents,
    agentConfig,
    agentDescriptors,
    onPickProvider,
    onPickAgent,
    onConfigureAgents,
    sessionId,
    onAgentCompact,
    onAgentFork,
    onAgentReverted,
    skills,
    usageRing,
    ruleEditorCallId,
    onRuleEditorCancel,
    notices = [],
    pendingApprovalCount = 0,
    onAllowAllPending,
    onDecide,
    onPlanReply,
    onAskUserReply,
  } = props;
  const {
    draftError,
    dragging,
    submit,
    setDragging,
    attachNativeFiles,
    selected,
    noticeItems,
    setSelectedNotice,
    preview,
    setPreview,
    planActive,
    goalComposing,
    togglePlan,
    setGoalComposing,
    activePromptKind,
    images,
    setImages,
    quotes,
    setQuotes,
    attachments,
    attachError,
    removeAttachment,
    plans,
    pendingPlan,
    questions,
    pendingAskUser,
    mentionRef,
    addImages,
    text,
    setText,
    restoringDraft,
    persistText,
    setSlashFeedback,
    paletteVisible,
    paletteMatches,
    setSlashIdx,
    slashIdx,
    commitPaletteChoice,
    slash,
    updateText,
    paletteTrigger,
    cmdEnterSend,
    phone,
    paletteGroups,
    slashFeedback,
    approvals,
    pendingApproval,
    openNativeFiles,
    attachLoading,
    textOwner,
    setAttachLoading,
    setAttachError,
    attachNativeFile,
    modelPopOpen,
    setModelPopOpen,
    agentPicker,
    modeLabel,
    filePickerOpen,
    setFilePickerOpen,
    attachFile,
    fileInputRef,
  } = useComposerController(props);
  return (
    <div className="flex flex-col items-center gap-1.5 px-4 pb-4 pt-2 max-md:px-2 max-md:pb-2">
      {draftError && (
        <p
          role="alert"
          className="w-full max-w-3xl px-2 text-[12px] text-amber-700 dark:text-amber-400"
        >
          {draftError}
        </p>
      )}
      <QueuedMessageStack
        key={sessionId ?? 'unsaved'}
        items={queuedMessages}
        onEdit={onEditQueuedMessage}
        onReorder={onReorderQueuedMessage}
        canSteer={
          !disabled &&
          !!busy &&
          (engine?.capabilities?.steering === 'native' ||
            engine?.capabilities?.steering === 'safe_boundary')
        }
        onRemove={onRemoveQueuedMessage}
        onSteer={onSteerQueuedMessage}
      />
      <ComposerContextDock
        environment={environment ?? null}
        environments={environments ?? []}
        envSwitching={envSwitching ?? null}
        envDisabled={busy || disabled}
        onSwitchEnvironment={onSwitchEnvironment}
        usageRing={usageRing}
        cwd={cwd}
        onCwdSwitched={onCwdSwitched}
      />
      <form
        className={cn(
          'relative w-full max-w-3xl flex flex-col gap-1.5 rounded-[22px] bg-white shadow-[0_12px_30px_-26px_rgba(15,23,42,0.55)] ring-1 ring-border/55 backdrop-blur-xl p-2.5 transition-colors dark:bg-secondary/85 dark:shadow-[0_14px_36px_-28px_rgba(0,0,0,0.8)]',
          dragging && 'bg-mira-blue/[0.06] ring-1 ring-mira-blue/45',
        )}
        onSubmit={(e) => {
          e.preventDefault();
          submit();
        }}
        onDragOverCapture={(e) => {
          if (!e.dataTransfer.types.includes('Files')) return;
          e.preventDefault();
          e.stopPropagation();
          setDragging(true);
        }}
        onDragLeaveCapture={(e) => {
          if (!e.currentTarget.contains(e.relatedTarget as Node | null)) setDragging(false);
        }}
        onDropCapture={(e) => {
          setDragging(false);
          if (e.dataTransfer.files.length === 0) return;
          e.preventDefault();
          e.stopPropagation();
          void attachNativeFiles(e.dataTransfer.files);
        }}
      >
        {dragging && (
          <div className="pointer-events-none absolute inset-0 z-10 flex items-center justify-center rounded-[22px] text-[13px] font-medium text-mira-blue">
            Drop files or images to attach
          </div>
        )}
        {selected && (
          <ComposerNoticeStack
            items={noticeItems}
            selected={selected.id}
            onSelect={setSelectedNotice}
          />
        )}
        {notices.map((notice) => (
          <div key={notice.id} hidden={selected?.id !== notice.id}>
            {notice.content}
          </div>
        ))}
        <ImageLightbox src={preview} onClose={() => setPreview(null)} />
        {(planActive || goal || goalComposing) && (
          <div className="flex flex-wrap items-center gap-1.5 px-1.5 pt-0.5">
            {planActive && <PlanChip onExit={togglePlan} />}
            {goalComposing && !goal && <GoalComposeChip onCancel={() => setGoalComposing(false)} />}
            {goal && <GoalChip goal={goal} onClear={onClearGoal} />}
          </div>
        )}

        {!activePromptKind && images.length > 0 && (
          <div className="flex flex-wrap gap-2 px-1.5 pt-1">
            {images.map((img, i) => (
              <div key={i} className="group relative">
                <img
                  src={`data:${img.media_type};base64,${img.data}`}
                  alt=""
                  onClick={() => setPreview(`data:${img.media_type};base64,${img.data}`)}
                  className="size-14 cursor-zoom-in rounded-lg border border-border object-cover"
                />
                <ImageAttachmentDetails image={img} />
                <button
                  type="button"
                  aria-label="Remove image"
                  onClick={() => setImages((prev) => prev.filter((_, j) => j !== i))}
                  className="absolute -right-1.5 -top-1.5 flex size-5 items-center justify-center rounded-full border border-border bg-background text-[11px] leading-none text-muted-foreground transition-opacity hover:text-foreground [@media(hover:hover)]:opacity-0 [@media(hover:hover)]:group-hover:opacity-100"
                >
                  ×
                </button>
              </div>
            ))}
          </div>
        )}

        {!activePromptKind && quotes.length > 0 && (
          <div className="flex flex-col gap-1.5 px-1.5 pt-1">
            {quotes.map((q) => (
              <QuoteCard
                key={q.id}
                quote={q}
                onRemove={() => setQuotes((prev) => prev.filter((p) => p.id !== q.id))}
              />
            ))}
          </div>
        )}

        {!activePromptKind && (attachments.length > 0 || attachError) && (
          <div className="flex flex-wrap gap-1.5 px-1.5">
            {attachments.map((a) => (
              <AttachmentChip
                key={a.path}
                attachment={a}
                onRemove={() => removeAttachment(a.path)}
                cwd={cwd}
              />
            ))}
            {attachError && (
              <span className="rounded-md border border-destructive/40 bg-destructive/10 px-2 py-1 text-[11.5px] text-destructive">
                {attachError}
              </span>
            )}
          </div>
        )}

        {/* Keep unsent answers and edited plans mounted when switching notices. */}
        {plans.map((plan) => (
          <m.div
            key={`plan:${plan.callId}`}
            hidden={pendingPlan?.callId !== plan.callId}
            initial={false}
            animate={{ opacity: pendingPlan?.callId === plan.callId ? 1 : 0 }}
            transition={{ duration: 0.15 }}
          >
            {onPlanReply && (
              <EmbeddedPlanCard
                proposal={plan.proposal}
                onApprove={(steps) => onPlanReply(plan.callId, true, steps)}
                onCancel={(note) => onPlanReply(plan.callId, false, undefined, note || undefined)}
              />
            )}
          </m.div>
        ))}
        {questions.map((question) => (
          <m.div
            key={`question:${question.callId}`}
            hidden={pendingAskUser?.callId !== question.callId}
            initial={false}
            animate={{ opacity: pendingAskUser?.callId === question.callId ? 1 : 0 }}
            transition={{ duration: 0.15 }}
          >
            {onAskUserReply && (
              <EmbeddedAskUserCard
                asker={engine?.kind === 'agent' ? engine.display_name : 'Mira'}
                proposal={question.proposal}
                onSubmit={(answers) =>
                  onAskUserReply(question.callId, { cancelled: false, answers })
                }
                onCancel={() => onAskUserReply(question.callId, { cancelled: true })}
              />
            )}
          </m.div>
        ))}

        {!activePromptKind && (
          <div className="relative">
            <MentionInput
              handleRef={mentionRef}
              onPasteImages={(files) => void addImages(files)}
              value={text}
              onChange={(next) => {
                setText(next);
                if (!restoringDraft.current) persistText(next);
                setSlashFeedback(null);
              }}
              onKeyDown={(e) => {
                // Palette is open → arrows navigate, Enter picks, Esc closes.
                if (paletteVisible && paletteMatches.length > 0) {
                  if (e.key === 'ArrowDown') {
                    e.preventDefault();
                    setSlashIdx((i) => Math.min(i + 1, paletteMatches.length - 1));
                    return;
                  }
                  if (e.key === 'ArrowUp') {
                    e.preventDefault();
                    setSlashIdx((i) => Math.max(i - 1, 0));
                    return;
                  }
                  if (e.key === 'Enter' && !e.shiftKey) {
                    e.preventDefault();
                    const cmd = paletteMatches[slashIdx];
                    if (cmd) commitPaletteChoice(cmd);
                    return;
                  }
                  if (e.key === 'Tab') {
                    e.preventDefault();
                    const cmd = paletteMatches[slashIdx];
                    if (cmd) {
                      const prefix =
                        slash.mode === 'palette' ? text.slice(0, slash.triggerStart) : '';
                      updateText(
                        `${prefix}${paletteTrigger}${cmd.name}${cmd.takesArgs ? ' ' : ''}`,
                      );
                    }
                    return;
                  }
                  if (e.key === 'Escape') {
                    e.preventDefault();
                    // Escape only clears the trigger span, not the whole
                    // composer — mid-message prefix survives.
                    const prefix =
                      slash.mode === 'palette' ? text.slice(0, slash.triggerStart) : '';
                    updateText(prefix);
                    return;
                  }
                }
                // Escape while goal-composing → abort the compose flow
                // without sending anything, matching how Esc dismisses
                // the slash palette above.
                if (goalComposing && e.key === 'Escape') {
                  e.preventDefault();
                  setGoalComposing(false);
                  updateText('');
                  return;
                }
                // Enter behavior depends on the send mode: by default
                // Enter sends and Shift+Enter inserts a newline; with
                // "send with Cmd/Ctrl+Enter" on, plain Enter inserts the
                // newline and only the mod chord sends.
                if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) {
                  e.preventDefault();
                  submit();
                  return;
                }
                if (e.key === 'Enter' && e.shiftKey) {
                  e.preventDefault();
                  document.execCommand('insertText', false, '\n');
                  return;
                }
                if (e.key === 'Enter' && !cmdEnterSend) {
                  e.preventDefault();
                  submit();
                  return;
                }
                if (e.key === 'Enter' && cmdEnterSend) {
                  e.preventDefault();
                  document.execCommand('insertText', false, '\n');
                  return;
                }
              }}
              placeholder={
                disabled
                  ? 'Waiting for connection…'
                  : goalComposing
                    ? "Describe what 'done' looks like — mira will loop until it's met."
                    : planActive
                      ? 'Describe your task to generate a plan…'
                      : phone
                        ? 'Ask mira anything'
                        : 'Ask mira anything · @ for files · / for commands'
              }
              disabled={disabled}
              roster={skills}
              ariaLabel="Message composer"
            />

            {paletteVisible && paletteMatches.length > 0 && (
              <SlashPalette
                groups={paletteGroups}
                activeIdx={slashIdx}
                onHover={setSlashIdx}
                onPick={commitPaletteChoice}
              />
            )}

            {slash.mode === 'args' && (
              <div className="tooltip pointer-events-none absolute -top-7 left-0">
                {slash.command.origin && (
                  <>
                    <OriginIcon origin={slash.command.origin} fallback={slash.command.icon} />
                    <span className="text-foreground/80">{slash.command.origin.label}</span>
                    <span className="text-muted-foreground/50">·</span>
                  </>
                )}
                <span className="font-mono text-foreground">
                  {slash.trigger}
                  {slash.command.name}
                </span>
                <span className="truncate">
                  {slash.command.usage.replace(`/${slash.command.name}`, '').trim()}
                </span>
                {slash.command.kindLabel && (
                  <span className="ml-1 shrink-0 rounded border border-border/60 px-1 text-[10px] uppercase tracking-wide text-muted-foreground/80">
                    {slash.command.kindLabel}
                  </span>
                )}
              </div>
            )}
          </div>
        )}

        {!activePromptKind && slashFeedback && (
          <div className="mx-1 mb-1 rounded-md border border-amber-500/30 bg-amber-500/10 px-2 py-1 text-[11.5px] text-amber-200">
            {slashFeedback}
          </div>
        )}

        {/* Bottom slot: the approval card when one is pending (where the
            model picker used to sit), the toolbar otherwise. */}
        {approvals.length > 1 && onDecide && (
          <ApprovalQueue
            approvals={approvals}
            activeId={pendingApproval?.callId}
            onFocus={(id) => setSelectedNotice(`approval:${id}`)}
            onDecide={(id, allow) => onDecide(id, allow, 'once')}
          />
        )}
        {approvals.map((approval, index) => (
          <div
            key={`approval:${approval.callId}`}
            hidden={pendingApproval?.callId !== approval.callId}
          >
            {onDecide && (
              <EmbeddedApprovalCard
                position={index + 1}
                openRuleEditor={ruleEditorCallId === approval.callId}
                onRuleEditorCancel={onRuleEditorCancel}
                approval={approval}
                queued={pendingApprovalCount}
                onAllowAll={onAllowAllPending}
                onDecide={(allow, scope, rules) => onDecide(approval.callId, allow, scope, rules)}
              />
            )}
          </div>
        ))}
        {!activePromptKind && (
          <div className="flex items-center gap-1.5 px-1">
            <AttachMenu
              onAttachFile={openNativeFiles}
              loading={attachLoading}
              onScreenshot={async () => {
                const owner = textOwner.current;
                setAttachLoading(true);
                setAttachError(null);
                try {
                  const file = await captureScreenshot();
                  if (file) await attachNativeFile(file, owner);
                } catch (error) {
                  if (textOwner.current === owner)
                    setAttachError(error instanceof Error ? error.message : String(error));
                } finally {
                  if (textOwner.current === owner) setAttachLoading(false);
                }
              }}
            />

            {/* At phone width the row keeps only what's used from a phone:
                typing "/" still opens commands, and projects are in the
                sidebar drawer. */}
            <span className="contents max-md:hidden">
              <SlashButton
                onClick={() =>
                  updateText(text.startsWith('/') || text.startsWith('@') ? text : '/' + text)
                }
              />
            </span>

            <EnginePicker
              open={modelPopOpen}
              onOpenChange={setModelPopOpen}
              engine={engine ?? null}
              fallbackModel={model}
              providerName={providerName ?? null}
              engines={engines}
              agents={agents}
              agentsChecking={agentsChecking}
              onCheckAgents={onCheckAgents}
              agentConfig={agentConfig}
              agentDescriptors={agentDescriptors}
              onPickProvider={(instance, m) =>
                onPickProvider ? onPickProvider(instance, m) : onSetModel(m, instance)
              }
              onPickAgent={(driver, m) => onPickAgent?.(driver, m)}
              onSetModelOption={onSetModelOption}
              onConfigureAgents={onConfigureAgents}
              sessionId={sessionId}
              onAgentCompact={onAgentCompact}
              onAgentFork={onAgentFork}
              onAgentReverted={onAgentReverted}
            />

            <span className="contents max-md:hidden">
              <ProjectChip cwd={cwd} onClick={onOpenPicker} />
            </span>

            <span className="flex-1 max-md:min-w-1" />

            {agentDriving && agentPicker ? (
              /* One picker for both systems: it lists the agent's postures
                 under Mira's names and confirms the pick in the universal
                 approval dialog. */
              <ModePicker
                mode={agentPicker.current ?? 'manual'}
                label={MODES.find((d) => d.value === agentPicker.current)?.label ?? 'Agent mode'}
                onPick={(m) => onPickAgentMode?.(m)}
                modes={agentPicker.modes.length > 0 ? agentPicker.modes : undefined}
              />
            ) : (
              <ModePicker
                mode={mode}
                label={modeLabel}
                onPick={onSetMode}
                modes={agentDriving ? AGENT_APPROVAL_MODES : undefined}
              />
            )}

            {busy &&
            !text.trim() &&
            attachments.length === 0 &&
            images.length === 0 &&
            quotes.length === 0 ? (
              <button
                type="button"
                onClick={onInterrupt}
                className="flex size-8 shrink-0 items-center justify-center rounded-full bg-mira-error text-mira-on-accent transition-colors hover:brightness-110 touch:size-10"
                title="Stop"
                aria-label="Stop"
              >
                <Square className="size-3.5" style={{ fill: 'currentColor' }} />
              </button>
            ) : (
              <button
                type="submit"
                disabled={
                  disabled ||
                  (!text.trim() &&
                    attachments.length === 0 &&
                    images.length === 0 &&
                    quotes.length === 0)
                }
                className="flex size-8 shrink-0 items-center justify-center rounded-full bg-foreground text-background transition-opacity hover:opacity-90 disabled:opacity-35 touch:size-10"
                title={
                  busy ? 'Queue message' : cmdEnterSend ? 'Send (⌘/Ctrl+Enter)' : 'Send (Enter)'
                }
                aria-label={busy ? 'Queue message' : 'Send'}
              >
                <ArrowUp className="size-4" />
              </button>
            )}
          </div>
        )}
      </form>

      <FilePicker
        open={filePickerOpen}
        startPath={cwd || undefined}
        onClose={() => setFilePickerOpen(false)}
        onPicked={(p) => {
          attachFile(p);
        }}
      />

      {/* Hidden native file input. `openNativeFiles()` above `.click()`s
       *  this; the OS shows its own open dialog. `multiple` matches
       *  Codex's picker. Kept out of the tab order (aria-hidden + no
       *  focus ring) so keyboard users don't stumble into it. */}
      <input
        ref={fileInputRef}
        data-composer-attachments=""
        type="file"
        multiple
        aria-hidden
        tabIndex={-1}
        className="sr-only"
        onChange={(e) => {
          attachNativeFiles(e.target.files);
        }}
      />
    </div>
  );
}

export { parseSentAttachments, SentAttachmentChip } from './composer/attachments';
export { type ImageData, type PendingApproval, type QueuedComposerMessage } from './composer/types';
