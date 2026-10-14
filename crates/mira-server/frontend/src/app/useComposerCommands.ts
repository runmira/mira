import { forkSession,newSession,previewCheckpoint,type MessageRef,type Restored } from '../api';
import { type QueuedComposerMessage } from '../components/Composer';
import { type RecoveryAction } from '../components/UsageRecoveryCard';
import { loadModelOptions } from '../lib/models';
import { useAppRuntime } from '../state/RuntimeProvider';
import { countUserMessages,sealThought,type Entry } from '../transcript/entries';
import type { Mode } from '../types';

import type { WorkspaceRuntime } from './WorkspaceRuntime';
export function useComposerCommands(
  context: Pick<
    WorkspaceRuntime,
    | 'getSessionId'
    | 'updateQueueProjection'
    | 'wsRef'
    | 'queueMutationsRef'
    | 'pendingSteerRef'
    | 'getQueues'
    | 'isTurnRunning'
    | 'flushNativeFrames'
    | 'flushTokens'
    | 'followRef'
    | 'updateTranscript'
    | 'setMainView'
    | 'setSidebarRefresh'
    | 'setShowJump'
    | 'turnStartRef'
    | 'getEngine'
    | 'providerTurnCursorRef'
    | 'historyOffsetRef'
    | 'startTurnUsage'
    | 'recordTurnTimings'
    | 'reportTurnRunning'
    | 'reportThinking'
    | 'busy'
    | 'setRestoreNote'
    | 'setRestoreAsk'
    | 'sessionId'
    | 'attachSession'
    | 'refreshRepo'
    | 'recordTurnUsage'
    | 'acpDriver'
    | 'acpModelOption'
  >,
) {
  const appRuntime = useAppRuntime();
  const {
    getSessionId,
    updateQueueProjection,
    wsRef,
    queueMutationsRef,
    pendingSteerRef,
    getQueues,
    flushNativeFrames,
    flushTokens,
    followRef,
    updateTranscript,
    setMainView,
    setSidebarRefresh,
    setShowJump,
    turnStartRef,
    getEngine,
    providerTurnCursorRef,
    historyOffsetRef,
    startTurnUsage,
    recordTurnTimings,
    reportTurnRunning,
    reportThinking,
    busy,
    setRestoreNote,
    setRestoreAsk,
    sessionId,
    attachSession,
    refreshRepo,
    recordTurnUsage,
    acpDriver,
    acpModelOption,
  } = context;

  function queuedFor(id = getSessionId()) {
    return appRuntime.sessionState.get().queued.get(id) ?? [];
  }

  function updateSessionQueue(
    id: string,
    update: (items: QueuedComposerMessage[]) => QueuedComposerMessage[],
  ) {
    updateQueueProjection((prev) => {
      const next = new Map(prev);
      const items = update(next.get(id) ?? []);
      if (items.length === 0) next.delete(id);
      else next.set(id, items);
      return next;
    });
  }

  function queueMessage(text: string, images?: { media_type: string; data: string }[]) {
    const id = getSessionId();
    if (!id) return false;
    const draftKey = appRuntime.drafts.key(id);
    if (appRuntime.drafts.awaiting(draftKey)) return false;
    const item: QueuedComposerMessage = {
      id: crypto.randomUUID(),
      text,
      images,
    };
    if (!wsRef.current?.sendImmediate({ type: 'queue_input', session_id: id, id: item.id, text, images })) return false;
    appRuntime.drafts.markDelivery(draftKey, item.id);
    updateSessionQueue(id, items => [...items, { ...item, error: 'Waiting for server queue confirmation.' }]);
    // Keep the draft until QueueUpdated acknowledges this stable input ID.
    return false;
  }

  function removeQueuedMessage(id: string) {
    const session = getSessionId();
    if (!session) return;
    wsRef.current?.send({ type: 'remove_queued_input', session_id: session, id });
  }

  function editQueuedMessage(item: QueuedComposerMessage, text: string) {
    return queueMutationsRef.current.request(
      {
        type: 'edit_queued_input',
        session_id: getSessionId(),
        id: item.id,
        fingerprint: item.fingerprint ?? '',
        text,
        images: item.images,
      },
      (message) => {
        if (!wsRef.current?.sendImmediate(message))
          throw new Error('Reconnect before changing queued messages.');
      },
    );
  }

  function changeRecovery(id: string, action: RecoveryAction) {
    return queueMutationsRef.current.request(
      { type: 'update_limit_recovery', session_id: getSessionId(), id, action },
      (message) => {
        if (!wsRef.current?.sendImmediate(message))
          throw new Error('Reconnect before changing recovery.');
      },
    );
  }

  function reorderQueuedMessage(id: string, beforeId: string | null) {
    return queueMutationsRef.current.request(
      {
        type: 'reorder_queued_input',
        session_id: getSessionId(),
        id,
        before_id: beforeId,
      },
      (message) => {
        if (!wsRef.current?.sendImmediate(message))
          throw new Error('Reconnect before changing queued messages.');
      },
    );
  }

  function steerQueuedMessage(id: string) {
    const session = getSessionId();
    if (!session || pendingSteerRef.current.has(session)) return;
    const picked = getQueues().get(session)?.find((item) => item.id === id);
    if (!picked) return;
    flushNativeFrames();
    flushTokens();
    pendingSteerRef.current.set(session, picked);
    updateSessionQueue(session, (items) =>
      items.map((item) => (item.id === id ? { ...item, steering: true, error: undefined } : item)),
    );
    const createdAt = Date.now();
    followRef.current = true;
    updateTranscript((entries) => [
      ...sealThought(entries, createdAt),
      {
        kind: 'msg',
        steerRequestId: id,
        msg: {
          role: 'user',
          created_at: createdAt,
          content: picked.text,
          images: picked.images,
          input_intent: 'steer',
        },
      },
    ]);
    if (wsRef.current?.send({
      type: 'steer',
      request_id: id,
      text: picked.text,
      images: picked.images,
    }) === false || !wsRef.current) {
      pendingSteerRef.current.delete(session);
      updateSessionQueue(session, items => items.map(item => item.id === id ? {...item, steering:false, error:'Reconnect before steering. Your message is still queued.'} : item));
      updateTranscript(entries => entries.filter(entry => entry.kind !== 'msg' || entry.steerRequestId !== id));
    }
  }

  function sendNow(
    text: string,
    images?: { media_type: string; data: string }[],
    transmit = true,
    inputId?: string,
  ) {
    if (transmit) return queueMessage(text, images);
    if (!appRuntime.sessionState.get().active) {
      setRestoreNote({ text: 'The message could not be sent. Your draft is kept here.', error: true });
      return false;
    }
    if (inputId && appRuntime.readSession().entries.some(entry => entry.kind === 'msg' && entry.msg.input_id === inputId)) return true;
    if (inputId) appRuntime.updateSessionData('activeInputId', inputId);
    // Belt-and-suspenders — the composer isn't visible on non-chat views,
    // but a keyboard-driven send would still land the message and it should
    // pull the user back to the transcript.
    setMainView('chat');
    // Refresh the sidebar immediately so a brand-new thread shows up in
    // the projects list on the first send, not after the model finishes
    // responding. The harness checkpoints the pushed user message at the
    // top of `run_loop` so this refetch sees the new row.
    setSidebarRefresh((n) => n + 1);
    followRef.current = true;
    setShowJump(false);
    const now = Date.now();
    turnStartRef.current = now;
    const previous = appRuntime.readSession().entries;
    const providerIndex = getEngine()?.kind === 'provider' ? providerTurnCursorRef.current++ : undefined;
    const next: Entry[] = [...previous, { kind: 'msg', providerTurnIndex: providerIndex,
      transcriptTurnIndex: countUserMessages(previous) + historyOffsetRef.current,
      msg: { role: 'user', created_at: now, content: text, images, input_id: inputId } }];
    const turnIndex = providerIndex ?? countUserMessages(next) - 1 + historyOffsetRef.current;
    startTurnUsage(turnIndex);
    appRuntime.beginTerminalTurn(inputId, now, turnIndex);
    updateTranscript(next);
    recordTurnTimings(previous => new Map(previous).set(turnIndex, { startedAt: now, endedAt: null }));
    reportTurnRunning(true);
    reportThinking(true);
    // The server routes by the session's engine: a session on an agent
    // sends this to the agent (starting it if needed, with the
    // conversation so far if it just took over), otherwise to the
    // provider. One message type, so a prompt can never go to the wrong
    // engine because this client's view lagged the server's.
    return true;
  }

  function onSend(text: string, images?: { media_type: string; data: string }[]) {
    // The existing durable outbox provides input IDs, acceptance snapshots and
    // dispatch events. Plain Send has no acknowledgement identity.
    return queueMessage(text, images);
  }

  /** Edit & resend (or retry, with the same text) the user message at
   *  `userIdx`: the server rewinds history to just before it and starts a
   *  new turn. Later entries are dropped here to match. */
  /** How the server identifies a user message: its text, and which match
   *  of it counting from the latest. Shared by edit and restore so they can
   *  never disagree about which message is meant. */
  function messageRefAt(userIdx: number): MessageRef | null {
    const target = appRuntime.readSession().entries[userIdx];
    if (!target || target.kind !== 'msg' || target.msg.role !== 'user') return null;
    const text = target.msg.content ?? '';
    const occurrence = appRuntime.readSession().entries
      .slice(userIdx + 1)
      .filter((e) => e.kind === 'msg' && e.msg.role === 'user' && e.msg.content === text).length;
    return { text, occurrence };
  }

  async function askRestore(userIdx: number) {
    const ref = messageRefAt(userIdx);
    if (!ref || busy) return;
    try {
      const changes = await previewCheckpoint(ref, appRuntime.sessionState.get().active ?? undefined);
      if (changes.length === 0) {
        setRestoreNote({ text: 'Nothing to restore — the files already match.' });
        return;
      }
      setRestoreAsk({ ref, changes });
    } catch (e) {
      setRestoreNote({ text: (e as Error).message, error: true });
    }
  }

  /** "Fork from here": copy the chat through this message's turn into a
   *  new chat (nested under this one in the sidebar) and switch to it. */
  async function forkAt(userIdx: number) {
    const ref = messageRefAt(userIdx);
    if (!ref || !sessionId) return;
    try {
      const id = await forkSession(sessionId, ref);
      setSidebarRefresh((n) => n + 1);
      attachSession(id);
    } catch (e) {
      setRestoreNote({ text: `Couldn't fork: ${(e as Error).message}`, error: true });
    }
  }

  async function doRestore(run: () => Promise<Restored>, verb: string) {
    try {
      const r = await run();
      const n = r.changes.length;
      setRestoreNote({
        text: `${verb} ${n} file${n === 1 ? '' : 's'}.`,
        undo: verb === 'Restored' ? r.undo : undefined,
      });
    } catch (e) {
      setRestoreNote({ text: (e as Error).message, error: true });
    } finally {
      refreshRepo();
    }
  }

  function onResend(userIdx: number, text: string) {
    const ref = messageRefAt(userIdx);
    const target = appRuntime.readSession().entries[userIdx];
    if (busy || !ref || !target || target.kind !== 'msg') return;
    const { text: original, occurrence } = ref;
    if (!wsRef.current?.sendImmediate({ type: 'resend', original, occurrence, text })) {
      setRestoreNote({ text: 'The edit could not be sent. Reconnect before retrying.', error: true });
      return;
    }
    followRef.current = true;
    setShowJump(false);
    const next: Entry[] = [
      ...appRuntime.readSession().entries.slice(0, userIdx),
      {
        kind: 'msg',
        transcriptTurnIndex: target.transcriptTurnIndex,
        providerTurnIndex: target.providerTurnIndex,
        msg: { role: 'user', created_at: Date.now(), content: text, images: target.msg.images },
      },
    ];
    const turnIndex =
      target.providerTurnIndex ?? countUserMessages(next) - 1 + historyOffsetRef.current;
    startTurnUsage(turnIndex);
    recordTurnUsage((prev) => new Map([...prev].filter(([i]) => i < turnIndex)));
    updateTranscript(next);
    const startedAt = Date.now();
    turnStartRef.current = startedAt;
    appRuntime.beginTerminalTurn(undefined, startedAt, turnIndex);
    recordTurnTimings(previous => new Map([...previous].filter(([index]) => index < turnIndex))
      .set(turnIndex, { startedAt, endedAt: null }));
    reportTurnRunning(true);
    reportThinking(true);
  }

  function onSetMode(m: Mode) {
    wsRef.current?.send({ type: 'set_mode', mode: m });
  }

  /** Switch an external agent's own session mode. Separate from Mira's
   *  `set_mode`: different system, and the ids are the agent's. */
  function onSetAcpMode(modeId: string, acknowledgePrivileged = false) {
    wsRef.current?.send({
      type: 'acp_set_mode',
      mode_id: modeId,
      acknowledge_privileged: acknowledgePrivileged,
    });
  }

  function onSetModel(m: string, instance?: string | null) {
    // With an agent driving, a pick from the list is the *agent's* model, not
    // Mira's: it goes out as the agent's model config option. Sending
    // `set_model` here changed Mira's model behind the agent's back while the
    // agent kept running whatever it was running — the picker lied.
    if (acpDriver) {
      // Not yet running: the server remembers the pick and launches with it.
      wsRef.current?.send({
        type: 'acp_set_config_option',
        option_id: acpModelOption?.id ?? 'model',
        value: m,
      });
      return;
    }
    // The model's remembered options travel with the switch (#85).
    wsRef.current?.send({
      type: 'set_model',
      model: m,
      instance: instance ?? null,
      options: loadModelOptions(m, instance ?? null),
    });
  }

  function onSetModelOption(id: string, value: string) {
    // With an ACP agent driving the session, the option ids are the *agent's*
    // (`model`, `thought_level`, …) and Mira's server would ignore them.
    if (acpDriver) {
      wsRef.current?.send({ type: 'acp_set_config_option', option_id: id, value });
      return;
    }
    wsRef.current?.send({ type: 'set_model_option', id, value });
  }

  /** Kick off (or replace) an autonomous run against a condition. The
   *  server broadcasts `goal_set` so the panel state syncs there — we
   *  don't set it locally to avoid a brief drift if the server rejects
   *  the request (e.g. empty condition). */
  function onSetGoal(condition: string, maxIterations?: number) {
    const trimmed = condition.trim();
    if (!trimmed) return;
    wsRef.current?.send({
      type: 'set_goal',
      condition: trimmed,
      max_iterations: maxIterations ?? null,
    });
  }

  function onClearGoal() {
    wsRef.current?.send({ type: 'clear_goal' });
  }

  function onCompact(focus: string) {
    // The server announces it (`compacting`), which shows the card.
    wsRef.current?.send({ type: 'compact', focus: focus || null });
  }

  async function onNewChat() {
    // The new chat inherits this one's engine on the server — provider and
    // model, or agent and its settings — and its `ready` reports it, so
    // the composer carries on exactly as it was. Nothing to reset here.
    try {
      const { id } = await newSession(getSessionId() || undefined);
      // The server just built a fresh slot for `id`, marked it active,
      // and published a Ready on ITS channel. Our WS forwarder is still
      // subscribed to the previous slot — attach so we start receiving
      // the new slot's frames (Ready + subsequent tokens).
      attachSession(id);
    } catch (e) {
      updateTranscript((prev) => [...prev, { kind: 'error', text: `new chat: ${(e as Error).message}` }]);
    }
  }
  return {
    queuedFor,
    updateSessionQueue,
    queueMessage,
    removeQueuedMessage,
    editQueuedMessage,
    changeRecovery,
    reorderQueuedMessage,
    steerQueuedMessage,
    sendNow,
    onSend,
    messageRefAt,
    askRestore,
    forkAt,
    doRestore,
    onResend,
    onSetMode,
    onSetAcpMode,
    onSetModel,
    onSetModelOption,
    onSetGoal,
    onClearGoal,
    onCompact,
    onNewChat,
  };
}
