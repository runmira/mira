/**
 * Rebuilding transcript entries from persisted history: Mira's own message
 * log, external-agent transcript lines, and the interleaved pages the
 * server sends on load. Split out of App.tsx.
 */
import { applyNativeMetadata } from '../lib/nativeStream';
import { type AskUserDecision } from '../components/AskUserCard';
import type {
  AskUserProposal,
  DiffPreview,
  Message,
  PlanProposal,
  PlanStep,
  ServerMsg,
  ToolCall,
  ToolResult,
  AgentTranscriptLine,
} from '../types';
import { type Entry, engineFromRef, upsertToolStart, updateTool, attachToolResult, applyNativeFrame, type NativeFrame, appendAcpText, appendAcpThought, upsertAcpTool, sealAcpThought, type AcpPlanEntry, addTurnUsage, withTurnStats, isSuccessfulAcpStop, describeAcpStop, compactionSummary, stripHookContext, type ToolEntry } from './entries';

/**
 * Rebuild transcript entries from an agent sidecar.
 *
 * Pure fold over the persisted lines using the same entry builders as live
 * traffic (`appendAcpText`, `upsertAcpTool`, …), so replayed turns render
 * exactly like live ones. Deliberately free of side effects: no busy flags,
 * no pings, no git refreshes, no turn-timing stamps — replay must not
 * disturb a session that may have a live agent running right now.
 *
 * State frames (modes, config, commands, usage) are NOT applied here; the
 * caller feeds those through the live handler, which owns the picker state.
 * Only transcript content is returned.
 */
/**
 * Rebuild a reloaded session's transcript with provider and agent turns in
 * the order they happened.
 *
 * The two histories are stored apart (the harness must not read agent
 * words as its own), so the server writes a `switch` marker into the agent
 * sidecar at every engine switch, recording how much harness history
 * preceded it. Between two markers, agent turns come first and provider
 * turns after — a switch to the provider is followed by provider turns,
 * and a switch to the agent by agent turns. Sidecars written before the
 * markers existed keep the old order: harness, then agent.
 */
export function entriesForTranscriptPage(page: import('../types').TranscriptPage, previews?: Record<string, DiffPreview>): Entry[] {
  const entries: Entry[] = [];
  let offset = 0;
  while (offset < page.items.length) {
    const first = page.items[offset];
    let end = offset + 1;
    while (end < page.items.length && page.items[end].turn_index === first.turn_index && !!page.items[end].message === !!first.message) end++;
    const block = page.items.slice(offset, end);
    const rebuilt = first.message
      ? historyToEntries(block.flatMap(item => item.message ? [item.message] : []), previews)
      : interleaveReplay([], previews, block.flatMap(item => item.line ? [item.line] : []));
    entries.push(...rebuilt.map(entry => ({ ...entry, transcriptTurnIndex: first.turn_index, providerTurnIndex: first.provider_turn_index })));
    offset = end;
  }
  return entries;
}

export function interleaveReplay(
  history: Message[],
  previews: Record<string, DiffPreview> | undefined,
  lines: AgentTranscriptLine[],
): Entry[] {
  if (!lines.some((l) => l.switch)) {
    return [...historyToEntries(history, previews), ...replayAgentTranscript(lines)];
  }
  const out: Entry[] = [];
  let from = 0;
  let segment: AgentTranscriptLine[] = [];
  for (const line of lines) {
    if (!line.switch) {
      segment.push(line);
      continue;
    }
    out.push(...replayAgentTranscript(segment));
    segment = [];
    const to = Math.min(Math.max(line.switch.harness_len, from), history.length);
    out.push(...historyToEntries(history.slice(from, to), previews));
    from = to;
    const toAgent = line.switch.to !== 'provider';
    const fromRef = line.switch.from_engine;
    const toRef = line.switch.to_engine;
    out.push({
      kind: 'engine_switch',
      engine: toRef
        ? engineFromRef(toRef)
        : toAgent
          ? { kind: 'agent', driver: line.driver, display_name: line.driver, status: 'ready' }
          : { kind: 'provider', display_name: 'provider', status: 'ready' },
      from: fromRef ? engineFromRef(fromRef) : null,
    });
  }
  out.push(...replayAgentTranscript(segment));
  out.push(...historyToEntries(history.slice(from), previews));
  return out;
}

export function replayAgentTranscript(lines: AgentTranscriptLine[]): Entry[] {
  let out: Entry[] = [];
  let turnStartedAt: number | null = null;
  for (const line of lines) {
    if (line.user) {
      out = [
        ...out,
        { kind: 'msg', msg: { role: 'user', created_at: line.t, content: line.user.text, images: line.user.attached_images, input_id:line.user.input_id, input_intent: line.user.input_intent } },
      ];
      if (!line.user.input_intent) turnStartedAt = line.t;
      continue;
    }
    const beforeLength = out.length;
    const f = line.frame as ServerMsg | null | undefined;
    if (!f || typeof f !== 'object' || !('type' in f)) continue;
    switch ((f as ServerMsg).type) {
      case 'tool_start': {
        const msg = f as Extract<ServerMsg, { type: 'tool_start' }>;
        out = upsertToolStart(out, msg.call);
        out = updateTool(out, msg.call.id, entry => ({ ...entry, startedAt: line.t }));
        break;
      }
      case 'tool_end': {
        const msg = f as Extract<ServerMsg, { type: 'tool_end' }>;
        out = attachToolResult(out, msg.result);
        out = updateTool(out, msg.result.call_id, entry => {
          const askUser = entry.call.function.name === 'ask_user' ? restoreAskUserFromCall(entry.call, msg.result) : undefined;
          const plan = entry.call.function.name === 'plan' ? restorePlanFromCall(entry.call, msg.result) : undefined;
          return { ...entry, ...(askUser ? { askUser } : {}), ...(plan ? { plan } : {}) };
        });
        break;
      }
      case 'stream_activity': {
        const activity = f as Extract<ServerMsg, { type: 'stream_activity' }>;
        out.push({ kind: 'activity', activityKind: activity.kind, title: activity.title, detail: activity.detail });
        break;
      }
      case 'acp_message_metadata': {
        const meta = f as Extract<ServerMsg, { type: 'acp_message_metadata' }>;
        out = applyNativeMetadata(out, meta.message_id, meta.phase);
        break;
      }
      case 'acp_text_snapshot':
      case 'acp_tool_output_delta':
        out = applyNativeFrame(out, f as NativeFrame);
        break;
      case 'acp_text':
        out = appendAcpText(out, (f as Extract<ServerMsg, { type: 'acp_text' }>).text, (f as Extract<ServerMsg, { type: 'acp_text' }>).message_id);
        break;
      case 'acp_thought':
        out = appendAcpThought(out, (f as Extract<ServerMsg, { type: 'acp_thought' }>).text);
        break;
      case 'acp_tool_call':
      case 'acp_tool_call_update':
        out = upsertAcpTool(
          out,
          (f as Extract<ServerMsg, { type: 'acp_tool_call' }>).call,
        );
        break;
      case 'acp_plan': {
        const sealed = sealAcpThought(out);
        const msg = f as Extract<ServerMsg, { type: 'acp_plan' }>;
        const entry: AcpPlanEntry = { kind: 'acp_plan', entries: msg.entries };
        const idx = sealed.findIndex((e) => e.kind === 'acp_plan');
        out =
          idx >= 0
            ? [...sealed.slice(0, idx), entry, ...sealed.slice(idx + 1)]
            : [...sealed, entry];
        break;
      }
      case 'acp_turn_usage':
        out = addTurnUsage(out, f as Extract<ServerMsg, { type: 'acp_turn_usage' }>);
        break;
      case 'acp_turn_end': {
        const msg = f as Extract<ServerMsg, { type: 'acp_turn_end' }>;
        const started = turnStartedAt;
        out = withTurnStats(out, (s) => ({ ...s, startedAt: s.startedAt ?? started, endedAt: line.t }));
        const sealed = sealAcpThought(out);
        if (isSuccessfulAcpStop(msg.stop_reason)) {
          out = sealed;
          break;
        }
        const detail = msg.detail ? ` ${msg.detail}` : '';
        out = [
          ...sealed,
          { kind: 'error', text: `${describeAcpStop(msg.stop_reason)}${detail}` },
        ];
        break;
      }
      case 'acp_mode_changed': {
        const msg = f as Extract<ServerMsg, { type: 'acp_mode_changed' }>;
        out = [
          ...out,
          {
            kind: 'warning',
            text: msg.privileged
              ? `${msg.display_name}: ${msg.mode_name} enabled — ${msg.mode_id} grants more access than Mira would`
              : `${msg.display_name}: mode set to ${msg.mode_name}`,
          },
        ];
        break;
      }
      case 'html_render': {
        const msg = f as Extract<ServerMsg, { type: 'html_render' }>;
        out = [...out, { kind: 'html_render', id: msg.id, title: msg.title, html: msg.html }];
        break;
      }
      case 'acp_unmodelled':
        // Diagnostics, not conversation — see the live handler.
        break;
      case 'warning':
        out = [...out, { kind: 'warning', text: (f as Extract<ServerMsg, { type: 'warning' }>).text }];
        break;
      case 'error':
        out = [...out, { kind: 'error', text: (f as Extract<ServerMsg, { type: 'error' }>).text }];
        break;
      default:
        // State frames and session ephemera are handled by the caller, or
        // deliberately skipped — never rendered as transcript content.
        break;
    }
    // Persisted frame time owns replayed messages, not the current wall clock.
    for (let i = beforeLength; i < out.length; i++) {
      const entry = out[i];
      if (entry.kind === 'msg') out[i] = { ...entry, msg: { ...entry.msg, created_at: line.t } };
    }
  }
  // Replayed thoughts have no real timing: rebuilding them stamped "now"
  // on both ends, which read as "Thought for <1s". Unknown is shown as
  // unknown, the way Mira's own restored history does.
  out = out.map((e) => (e.kind === 'thought' ? { ...e, live: false, startedAt: null, endedAt: null } : e));
  return out;
}

export function historyToEntries(
  history: Message[],
  /** Persisted diff previews from `SessionRecord.previews` (Ready
   *  frame). Attaches per call id so a reloaded transcript shows the
   *  same diff the user saw live, instead of dropping to the arg-only
   *  reconstruction fallback. */
  previews?: Record<string, DiffPreview>,
): Entry[] {
  // First pass — index tool results by call_id so the assistant walk can
  // attach them in O(1) rather than re-scanning history for each call.
  const resultByCallId = new Map<string, ToolResult>();
  for (const m of history) {
    if (m.role === 'tool' && m.tool_call_id) {
      resultByCallId.set(String(m.tool_call_id), {
        call_id: String(m.tool_call_id),
        content: m.content ?? '',
        is_error: false,
        // Screenshots arrive as URLs (fetched lazily when the card opens).
        ...(m.images?.length ? { images: m.images } : {}),
      });
    }
  }

  const entries: Entry[] = [];
  for (const m of history) {
    if (m.role === 'system' || m.role === 'tool') continue;

    if (m.role === 'user') {
      const summary = compactionSummary(m.content);
      if (summary != null) {
        entries.push({ kind: 'compact', state: 'done', summarized: null, summary });
      } else {
        // Prompt hooks append a `<hook-context>` block for the model; show
        // only what the user typed.
        entries.push({ kind: 'msg', msg: { ...m, content: stripHookContext(m.content) } });
      }
      continue;
    }

    // Assistant: thinking first, then any text, then a ToolEntry per
    // tool_call — the live turn order (`reasoning…`, `token…`,
    // `tool_start`) so a resumed transcript reads identically.
    const thought = (m.reasoning ?? [])
      .map((b) => (b.text ?? '').trim())
      .filter(Boolean)
      .join('\n\n');
    if (thought) {
      entries.push({ kind: 'thought', text: thought, live: false, startedAt: null, endedAt: null });
    }
    if ((m.content ?? '').trim()) {
      entries.push({ kind: 'msg', msg: m });
    }
    for (const call of m.tool_calls ?? []) {
      const result = resultByCallId.get(call.id) ?? null;
      const entry: ToolEntry = {
        kind: 'tool',
        call,
        preview: previews?.[call.id] ?? null,
        status: 'complete',
        result,
      };
      // On reload the `ask_user_request` / `plan_request` live frames
      // don't fire, so rebuild the interactive-card state directly from
      // the persisted call args (the proposal) + tool result text (the
      // resolved decision). Without this, completed interactive tools
      // render as raw JSON args.
      if (call.function.name === 'ask_user') {
        const restored = restoreAskUserFromCall(call, result);
        if (restored) entry.askUser = restored;
      } else if (call.function.name === 'plan') {
        const restored = restorePlanFromCall(call, result);
        if (restored) entry.plan = restored;
      }
      entries.push(entry);
    }
  }
  return entries;
}

/** Reconstruct the ask_user proposal + decision from persisted tool
 *  state. `call.function.arguments` is the JSON we sent to the tool
 *  (i.e. the AskUserProposal); `result.content` is the textual summary
 *  the tool wrote back — parseable because we own both sides of that
 *  format (see `AskUserTool::invoke` in `interactive.rs`). */
export function restoreAskUserFromCall(
  call: ToolCall,
  result: ToolResult | null,
): { proposal: AskUserProposal; decision: AskUserDecision } | undefined {
  let proposal: AskUserProposal;
  try {
    const args = JSON.parse(call.function.arguments) as { questions?: unknown };
    if (!Array.isArray(args.questions)) return undefined;
    proposal = { questions: args.questions as AskUserProposal['questions'] };
  } catch {
    return undefined;
  }
  const decision = parseAskUserResultText(result?.content ?? '', proposal.questions.length);
  return { proposal, decision };
}

/** Reconstruct the plan proposal + decision from persisted tool state.
 *  Same shape as `restoreAskUserFromCall`: the args carry the proposal,
 *  the result text carries the verdict + edited steps. */
export function restorePlanFromCall(
  call: ToolCall,
  result: ToolResult | null,
): { proposal: PlanProposal; decision: null | { approved: boolean; steps?: PlanStep[]; note?: string } } | undefined {
  let proposal: PlanProposal;
  try {
    const args = JSON.parse(call.function.arguments) as { title?: unknown; steps?: unknown };
    if (typeof args.title !== 'string' || !Array.isArray(args.steps)) return undefined;
    proposal = { title: args.title, steps: args.steps as PlanStep[] };
  } catch {
    return undefined;
  }
  const decision = parsePlanResultText(result?.content ?? '');
  return { proposal, decision };
}

/** Parse the plan tool's result body — see `PlanTool::invoke` in
 *  `interactive.rs` for the exact strings emitted. Missing result →
 *  render as no-decision-yet so the card stays actionable. */
export function parsePlanResultText(text: string):
  | null
  | { approved: boolean; steps?: PlanStep[]; note?: string } {
  if (!text.trim()) return null;
  if (/^Plan cancelled by user/i.test(text)) {
    const noteMatch = text.match(/Note:\s*(.+?)(?:\n\n|$)/s);
    return { approved: false, note: noteMatch ? noteMatch[1].trim() : undefined };
  }
  if (/^Plan approved/i.test(text)) {
    // Parse `1. description (why)` lines from the "Agreed steps:" block.
    const steps: PlanStep[] = [];
    const stepsBlock = text.split(/Agreed steps:\s*\n/i)[1] ?? '';
    for (const raw of stepsBlock.split('\n')) {
      const m = raw.match(/^\s*\d+\.\s*(.+?)(?:\s*\(([^)]+)\))?\s*$/);
      if (m) steps.push({ description: m[1].trim(), why: m[2]?.trim() ?? null });
    }
    return { approved: true, steps: steps.length > 0 ? steps : undefined };
  }
  return null;
}

/** Parse the tool result text into structured answers. Falls back to
 *  `cancelled: true` when the tool wrote its "dismissed" / "cancelled"
 *  copy. Anything we can't parse becomes an empty answer so the resolved
 *  card still lines up with the proposal by index. */
export function parseAskUserResultText(text: string, expectedQuestions: number): AskUserDecision {
  if (/dismissed the question card/i.test(text) || /prompt cancelled/i.test(text)) {
    return { cancelled: true };
  }
  const answers: { picked: string[]; custom: string | null }[] = [];
  let curr: { picked: string[]; custom: string | null } | null = null;
  for (const raw of text.split('\n')) {
    const line = raw.trimEnd();
    // Each answered question starts with `[Header] question…`.
    if (/^\[[^\]]+\]/.test(line)) {
      if (curr) answers.push(curr);
      curr = { picked: [], custom: null };
      continue;
    }
    if (!curr) continue;
    const trimmed = line.trim();
    const pickedMatch = trimmed.match(/^→\s*picked:\s*(.+)$/);
    if (pickedMatch) {
      curr.picked = pickedMatch[1].split(',').map((s) => s.trim()).filter(Boolean);
      continue;
    }
    const customMatch = trimmed.match(/^→\s*user said:\s*(.+)$/);
    if (customMatch) {
      curr.custom = customMatch[1];
      continue;
    }
    // "(skipped)" / "(no answer captured)" — leave the empty defaults.
  }
  if (curr) answers.push(curr);
  while (answers.length < expectedQuestions) {
    answers.push({ picked: [], custom: null });
  }
  return { cancelled: false, answers };
}
