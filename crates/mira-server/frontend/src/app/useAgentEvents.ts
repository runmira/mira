import { listEngines } from '../api';
import { saveAgentCaps } from '../lib/acpAgents';
import { applyNativeMetadata } from '../lib/nativeStream';
import {
  addTurnUsage,
  appendAcpText,
  appendAcpThought,
  describeAcpStop,
  finishCompaction,
  isSuccessfulAcpStop,
  sealAcpThought,
  settleTools,
  stampLastTurn,
  upsertAcpTool,
  withTurnStats,
  type AcpPlanEntry,
} from '../transcript/entries';
import type { ServerMsg } from '../types';

import { useStableCallback } from './shared';
import type { WorkspaceRuntime } from './WorkspaceRuntime';
export function useAgentEvents(
  context: Pick<
    WorkspaceRuntime,
    | 'setEntries'
    | 'nativeTurnsRef'
    | 'setBusy'
    | 'busyRef'
    | 'setThinking'
    | 'nativeWorkRef'
    | 'playPing'
    | 'clearThinkingIdle'
    | 'acpStopRef'
    | 'refreshRepo'
    | 'setTurnTimings'
    | 'turnStartRef'
    | 'drainQueuedMessage'
    | 'setPendingAcpMode'
    | 'setEngines'
    | 'capsDriverRef'
    | 'setAcpModes'
    | 'setAcpConfig'
    | 'setAcpCommands'
    | 'setAcpUsage'
    | 'setAcpLimits'
    | 'setSidebarRefresh'
    | 'setAcpError'
    | 'engineRef'
    | 'applyEngine'
    | 'setAcpStatusPending'
    | 'loadEngines'
  >,
) {
  const {
    setEntries,
    nativeTurnsRef,
    setBusy,
    busyRef,
    setThinking,
    nativeWorkRef,
    playPing,
    clearThinkingIdle,
    acpStopRef,
    refreshRepo,
    setTurnTimings,
    turnStartRef,
    drainQueuedMessage,
    setPendingAcpMode,
    setEngines,
    capsDriverRef,
    setAcpModes,
    setAcpConfig,
    setAcpCommands,
    setAcpUsage,
    setAcpLimits,
    setSidebarRefresh,
    setAcpError,
    engineRef,
    applyEngine,
    setAcpStatusPending,
    loadEngines,
  } = context;
  return useStableCallback((msg: ServerMsg) => {
    switch (msg.type) {
      case 'acp_message_metadata':
        setEntries((previous) => applyNativeMetadata(previous, msg.message_id, msg.phase));
        break;

      case 'runtime_turn_updated':
        if (msg.turn.running) {
          nativeTurnsRef.current.add(msg.turn.native_turn_id);
          setBusy(true);
          busyRef.current = true;
          setThinking(true);
        } else {
          nativeTurnsRef.current.delete(msg.turn.native_turn_id);
        }
        break;

      case 'runtime_work_updated': {
        const active = ['pending', 'running', 'waiting'].includes(msg.work.status);
        nativeWorkRef.current = nativeWorkRef.current.filter((w) => w.id !== msg.work.id);
        if (active) nativeWorkRef.current.push(msg.work);
        const running =
          active || nativeTurnsRef.current.size > 0 || nativeWorkRef.current.length > 0;
        setBusy(running);
        busyRef.current = running;
        break;
      }

      // -------- ACP (external agent) --------
      case 'acp_text':
        setEntries((prev) => appendAcpText(prev, msg.text, msg.message_id));
        break;

      case 'acp_thought':
        setEntries((prev) => appendAcpThought(prev, msg.text));
        break;

      case 'acp_tool_call':

      case 'acp_tool_call_update':
        setEntries((prev) => upsertAcpTool(prev, msg.call));
        break;

      case 'acp_plan':
        setEntries((prev) => {
          const sealed = sealAcpThought(prev);
          const idx = sealed.findIndex((e) => e.kind === 'acp_plan');
          const entry: AcpPlanEntry = { kind: 'acp_plan', entries: msg.entries };
          if (idx >= 0) {
            return [...sealed.slice(0, idx), entry, ...sealed.slice(idx + 1)];
          }
          return [...sealed, entry];
        });
        break;

      case 'acp_turn_end':
        // An agent compaction ends with its turn.
        setEntries((prev) =>
          prev.some((e) => e.kind === 'compact' && e.state === 'running' && e.trigger === 'agent')
            ? finishCompaction(
                prev,
                isSuccessfulAcpStop(msg.stop_reason)
                  ? { state: 'done' }
                  : { state: 'failed', error: describeAcpStop(msg.stop_reason) },
              )
            : prev,
        );
        // End the turn. This is the only place an external agent's turn can
        // be declared over — the prompt is fire-and-forget, so unlike the
        // harness there is no surrounding await to imply completion. Without
        // clearing `busy` here the composer spins forever, which is what a
        // rate-limited turn looks like: the agent is long finished, the UI
        // just never hears about it.
        // The same "turn finished" sound Mira's own turns make — only for a
        // turn that was running here (a Stop already settled the composer,
        // and its confirmation shouldn't chime).
        if (busyRef.current) playPing();
        const nativeStillWorking =
          nativeTurnsRef.current.size > 0 || nativeWorkRef.current.length > 0;
        setBusy(nativeStillWorking);
        busyRef.current = nativeStillWorking;
        setThinking(false);
        clearThinkingIdle();
        acpStopRef.current = null;
        // The agent edits files in its own process; nothing else tells the
        // panels their counts are stale.
        refreshRepo();
        // Close out the turn's timing, or "Worked for" keeps ticking on a
        // turn that ended minutes ago. The harness path does this on `done`;
        // the agent path never did, which is why a finished turn still showed
        // a live duration.
        setTurnTimings((prev) => stampLastTurn(prev, Date.now()));
        {
          const started = turnStartRef.current;
          const ended = Date.now();
          setEntries((prev) =>
            withTurnStats(prev, (st) => ({
              ...st,
              startedAt: st.startedAt ?? started,
              endedAt: ended,
            })),
          );
        }

        // A turn that did not complete should say why. A usage limit is an
        // error, not an aside: the agent produced no answer, so presenting it
        // as a warning styled like a reply made a failed turn look like a
        // completed one. The server puts the agent's own context (e.g. when
        // a limit resets) on the frame so this is one message, not two.
        const stopped = msg.stop_reason;
        setEntries((prev) => {
          const sealed = settleTools(sealAcpThought(prev));
          if (isSuccessfulAcpStop(stopped)) return sealed;
          const detail = msg.detail ? ` ${msg.detail}` : '';
          return [
            ...sealed,
            {
              kind: 'error',
              text: `${describeAcpStop(stopped)}${detail}`,
              responseFailure: true,
            },
          ];
        });
        window.setTimeout(drainQueuedMessage, 0);
        break;

      case 'acp_privileged_mode_confirmation':
        // The server refused without an acknowledgement. Show what the mode
        // does and let the user decide — the flag alone is not consent,
        // because the client sets it.
        // Folded into the same confirm state as a picker pick: one state,
        // one dialog, whether the mode came from the user or the server.
        setPendingAcpMode({ modeId: msg.mode_id, reason: msg.reason });
        break;

      case 'acp_mode_changed':
        // A standing change to what the agent may do, recorded in the
        // transcript so it is visible after the fact and not only in a
        // dropdown that may have been closed by then.
        setEntries((prev) => [
          ...prev,
          {
            kind: 'warning',
            text: msg.privileged
              ? `${msg.display_name}: ${msg.mode_name} enabled — ${msg.mode_id} grants more access than Mira would`
              : `${msg.display_name}: mode set to ${msg.mode_name}`,
          },
        ]);
        setPendingAcpMode(null);
        break;

      case 'engines_changed':
        // mira.yaml changed and the server rebuilt its engines: pickers
        // show new instances and keys without a restart.
        void listEngines()
          .then((view) => setEngines(view.engines))
          .catch(() => {});
        break;

      case 'acp_unmodelled':
        // Logged, not shown. Printing the agent's raw JSON into the chat
        // buried answers under diagnostics ("ACP system: {…}"), and a
        // wedged agent is already visible: its turn never ends and the
        // composer keeps spinning.
        console.debug(`[acp] unmodelled ${msg.method}:`, msg.reason);
        break;

      // State the agent owns rather than transcript content. Captured so it
      // is available to the model/mode pickers, and logged so none of it is
      // invisible while that wiring lands.
      case 'acp_modes': {
        // State, not a ref: these drive the picker and mode row, so the
        // transcript must re-render when they arrive.
        // Frames carry their driver (server-stamped); late frames from a
        // stopped agent must neither overwrite the live state nor pollute
        // another driver's "last seen" cache (OpenCode showing Claude
        // models). Unstamped frames keep the old behaviour.
        const modesDriver = msg.driver ?? capsDriverRef.current;
        if (modesDriver) {
          saveAgentCaps(modesDriver, { modes: { current: msg.current, available: msg.available } });
        }
        if (!msg.driver || msg.driver === capsDriverRef.current) {
          setAcpModes({ current: msg.current, available: msg.available, postures: msg.postures });
        }
        break;
      }

      case 'acp_config_options': {
        const configDriver = msg.driver ?? capsDriverRef.current;
        if (configDriver) saveAgentCaps(configDriver, { config: msg.options });
        if (!msg.driver || msg.driver === capsDriverRef.current) {
          setAcpConfig(msg.options);
        }
        break;
      }

      case 'acp_commands':
        setAcpCommands(msg.names);
        break;

      case 'acp_usage':
        setAcpUsage(msg);
        break;

      case 'acp_turn_usage':
        setEntries((prev) => addTurnUsage(prev, msg));
        break;

      case 'acp_limits':
        setAcpLimits(msg.windows);
        break;

      case 'acp_session_info':
        // The agent retitled itself; the sidebar reads its own title source,
        // so just nudge a refresh when one arrived.
        if (msg.title) setSidebarRefresh((n) => n + 1);
        break;

      case 'acp_agent_started':
        // Startup outcome. Surfaced as a warning on failure so a user learns
        // "grok isn't installed" instead of watching an empty pane.
        if (msg.error) {
          setAcpError(`${msg.display_name}: ${msg.error}`);
          setEntries((prev) => [
            ...prev,
            { kind: 'warning', text: `${msg.display_name}: ${msg.error}` },
          ]);
        } else {
          setAcpError(null);
        }
        break;

      case 'session_engine': {
        // A switch the user made (or an agent exiting) marks the
        // transcript, so the reader can see where one engine handed off
        // to the other. Status-only transitions (starting → ready) don't.
        const prev = engineRef.current;
        const next = msg.engine;
        const moved =
          prev != null &&
          (prev.kind !== next.kind ||
            (next.kind === 'agent' ? prev.driver !== next.driver : false));
        if (moved) {
          setEntries((es) => [...es, { kind: 'engine_switch', engine: next, from: prev }]);
        }
        // The sidebar badges each chat by its engine.
        if (moved || prev?.model !== next.model) setSidebarRefresh((n) => n + 1);
        applyEngine(next);
        break;
      }

      case 'acp_agent_status':
        // A probe ran on the server: the engines list has the fresh result.
        setAcpStatusPending(false);
        loadEngines();
        break;
    }
  });
}
