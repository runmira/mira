import type { ServerMsg } from '../types';
import { useAgentEvents } from './useAgentEvents';
import { useApprovalCommands } from './useApprovalCommands';
import { useApprovalEvents } from './useApprovalEvents';
import { useCommandBridge } from './useCommandBridge';
import { useComposerCommands } from './useComposerCommands';
import { useNoticeEvents } from './useNoticeEvents';
import { usePanelCommands } from './usePanelCommands';
import { usePanelEvents } from './usePanelEvents';
import { useSessionCommands } from './useSessionCommands';
import { useSessionEvents } from './useSessionEvents';
import { useStreamCommands } from './useStreamCommands';
import { useTranscriptEvents } from './useTranscriptEvents';
import { useWorkspaceEffects } from './useWorkspaceEffects';
import { useWorkspacePalette } from './useWorkspacePalette';
import { useWorkspaceState } from './useWorkspaceState';
import { useWorkspaceView } from './useWorkspaceView';
import { WorkspaceLayout } from './WorkspaceLayout';

export function Application() {
  const commands = useCommandBridge();
  const state = useWorkspaceState();
  const view = useWorkspaceView({ ...state, ...commands.stable });
  const palette = useWorkspacePalette({ ...state, ...view, ...commands.stable });
  const runtime = { ...state, ...view, ...palette, ...commands.stable };
  const sessionCommands = useSessionCommands(runtime);
  const approvalCommands = useApprovalCommands(runtime);
  const composerCommands = useComposerCommands(runtime);
  const panelCommands = usePanelCommands(runtime);
  const streamCommands = useStreamCommands(runtime);
  const sessionEvents = useSessionEvents(runtime);
  const approvalEvents = useApprovalEvents(runtime);
  const panelEvents = usePanelEvents(runtime);
  const noticeEvents = useNoticeEvents(runtime);
  const transcriptEvents = useTranscriptEvents(runtime);
  const agentEvents = useAgentEvents(runtime);
  function handleMessage(msg: ServerMsg) {
    if (
      !['token', 'session_activity', 'session_activity_snapshot', 'queue_updated'].includes(
        msg.type,
      )
    )
      runtime.flushTokens();
    switch (msg.type) {
      case 'queue_mutation_result':
      case 'session_activity':
      case 'session_activity_snapshot':
      case 'history_page':
      case 'queue_updated':
      case 'queue_delivery':
      case 'ready':
      case 'steer_result':
      case 'session_title_updated':
      case 'background_mode_changed':
      case 'session_background_idle':
      case 'session_background_running':
        sessionEvents(msg);
        return;
      case 'approval_rules':
      case 'approval_resolved':
      case 'approval_request':
      case 'plan_request':
      case 'prompt_resolved':
      case 'runtime_request_updated':
      case 'ask_user_request':
      case 'secret_request':
        approvalEvents(msg);
        return;
      case 'review_started':
      case 'review_progress':
      case 'review_result':
      case 'review_error':
      case 'subagent_started':
      case 'subagent_token':
      case 'subagent_tool_start':
      case 'subagent_tool_end':
      case 'subagent_warning':
      case 'subagent_progress':
      case 'subagent_review_request':
      case 'subagent_done':
      case 'subagent_scratchpad_note':
      case 'delegate_progress':
      case 'browser_active':
      case 'html_render':
        panelEvents(msg);
        return;
      case 'environment_status':
      case 'environment_progress':
      case 'environment_switched':
      case 'warning':
      case 'extensions_changed':
      case 'skills_reloaded':
      case 'error':
      case 'memory_learned':
        noticeEvents(msg);
        return;
      case 'turn_diffs':
      case 'stream_activity':
      case 'reasoning':
      case 'token':
      case 'tool_start':
      case 'tool_end':
      case 'turn_complete':
      case 'done':
      case 'tool_progress':
      case 'tool_preview':
      case 'model_changed':
      case 'mode_changed':
      case 'usage':
      case 'rate_limit':
      case 'compacting':
      case 'compacted':
      case 'compaction_failed':
      case 'goal_set':
      case 'goal_cleared':
      case 'goal_progress':
      case 'goal_done':
        transcriptEvents(msg);
        return;
      case 'acp_message_metadata':
      case 'runtime_turn_updated':
      case 'runtime_work_updated':
      case 'acp_text':
      case 'acp_thought':
      case 'acp_tool_call':
      case 'acp_tool_call_update':
      case 'acp_plan':
      case 'acp_turn_end':
      case 'acp_privileged_mode_confirmation':
      case 'acp_mode_changed':
      case 'engines_changed':
      case 'acp_unmodelled':
      case 'acp_modes':
      case 'acp_config_options':
      case 'acp_commands':
      case 'acp_usage':
      case 'acp_turn_usage':
      case 'acp_limits':
      case 'acp_session_info':
      case 'acp_agent_started':
      case 'session_engine':
      case 'acp_agent_status':
        agentEvents(msg);
        return;
      default:
        console.warn('[ws] unhandled server message type:', msg.type);
    }
  }
  commands.bind({
    ...sessionCommands,
    ...approvalCommands,
    ...composerCommands,
    ...panelCommands,
    ...streamCommands,
    handleMessage,
  });
  useWorkspaceEffects(runtime);
  return <WorkspaceLayout {...runtime} />;
}
