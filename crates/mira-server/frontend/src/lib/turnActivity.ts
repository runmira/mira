import type { Entry } from '../App';

/** Item produced by `groupAgentRuns`: either a single passthrough entry,
 *  a run of `>=2` consecutive `agent` tool entries folded into a group,
 *  or a run of `>=2` consecutive non-agent tool entries folded
 *  into a group (e.g. three `read_file`s → one "Read × 3" chip).
 *  External calls get the parent row from their first live event. */
export type GroupItem =
  | { kind: 'entry'; entry: Entry }
  | { kind: 'agent-group'; entries: (Entry & { kind: 'tool' })[] }
  | { kind: 'tool-group'; entries: (Entry & { kind: 'tool' })[] };

/** Types that render as their own cards (agent, delegate, plan, ask_user) —
 *  never fold into a generic tool-group. Agent has its own AgentGroup path;
 *  delegate gets the cross-engine hand-off card; plan and ask_user each swap
 *  in for the tool row when their proposal attaches, so grouping would hide
 *  the interactive card. */
const SPECIAL_TOOLS = new Set(['agent', 'delegate_task', 'plan', 'ask_user']);

export function groupAgentRuns(entries: Entry[]): GroupItem[] {
  const out: GroupItem[] = [];
  let i = 0;
  while (i < entries.length) {
    const e = entries[i];

    // Agent run: consume all consecutive `agent` entries into a single
    // AgentGroup regardless of individual state (pending is auto-approved
    // anyway since AgentTool is Pure).
    if (isAgentEntry(e)) {
      const run: (Entry & { kind: 'tool' })[] = [];
      while (i < entries.length && isAgentEntry(entries[i])) {
        run.push(entries[i] as Entry & { kind: 'tool' });
        i++;
      }
      out.push(
        run.length >= 2
          ? { kind: 'agent-group', entries: run }
          : { kind: 'entry', entry: run[0] },
      );
      continue;
    }

    // Generic tool run: any consecutive tool entries, none pending. Mixing
    // tool names is fine — ToolGroup renders a "Working/Worked" umbrella
    // with per-entry verbs in the preview when the run isn't homogeneous.
    // Pending calls must render individually so the approval UI is visible
    // and unmissable — folding them into a group would hide the y/n prompt.
    if (isGroupableTool(e)) {
      const run: (Entry & { kind: 'tool' })[] = [];
      while (i < entries.length && isGroupableTool(entries[i])) {
        run.push(entries[i] as Entry & { kind: 'tool' });
        i++;
      }
      out.push(
        (run.length >= 2 || run[0].agentCall != null)
          ? { kind: 'tool-group', entries: run }
          : { kind: 'entry', entry: run[0] },
      );
      continue;
    }

    out.push({ kind: 'entry', entry: e });
    i++;
  }
  return out;
}

function isAgentEntry(e: Entry): boolean {
  return e.kind === 'tool' && e.call.function.name === 'agent';
}

/** A tool entry is groupable when it isn't a special one-off renderer
 *  (agent/delegate/plan) and isn't currently awaiting user approval. */
function isGroupableTool(e: Entry): boolean {
  if (e.kind !== 'tool') return false;
  if ((e.call.function.name === 'delegate_task' || e.call.function.name.endsWith('__delegate_task'))) return false;
  if (SPECIAL_TOOLS.has(e.call.function.name)) return false;
  if (e.status === 'pending' || e.askUser || e.plan) return false;
  return true;
}

export function findFinalAssistantIndex(body: Entry[]): number {
  for (let index = body.length - 1; index >= 0; index--) {
    const entry = body[index];
    if (entry.kind === 'msg' && entry.nativePhase === 'final_answer' && entry.msg.content?.trim()) return index;
  }
  for (let i = body.length - 1; i >= 0; i--) {
    const e = body[i];
    if (e.kind === 'msg' && e.msg.role === 'assistant' && (e.msg.content ?? '').trim()) {
      return i;
    }
  }
  return -1;
}


/** Both sides of assistant text use the same live grouping path. Completing
 * a native turn only folds trailing work into its history disclosure. */
export function turnActivity(body: Entry[], foldTrailing: boolean) {
  const finalIdx = findFinalAssistantIndex(body);
  const intermediateRaw = finalIdx < 0 ? body
    : foldTrailing ? body.filter((_, index) => index !== finalIdx) : body.slice(0, finalIdx);
  const trailingRaw = finalIdx >= 0 && !foldTrailing ? body.slice(finalIdx + 1) : [];
  return {
    intermediateRaw,
    intermediate: groupAgentRuns(intermediateRaw),
    finalEntry: finalIdx >= 0 ? body[finalIdx] : null,
    trailing: groupAgentRuns(trailingRaw),
  };
}

export type Turn = {
  /** The user's message that opened this turn. `null` for any pre-user
   *  entries (e.g. a system-emitted warning before the first send). */
  user: Entry | null;
  /** Everything after `user` up to the next user message. */
  body: Entry[];
};

export function groupByTurn(entries: Entry[]): Turn[] {
  const turns: Turn[] = [];
  let current: Turn = { user: null, body: [] };
  for (const e of entries) {
    if (e.kind === 'msg' && e.msg.role === 'user' && e.msg.input_intent !== 'steer') {
      // Close previous turn if it had anything.
      if (current.user || current.body.length) turns.push(current);
      current = { user: e, body: [] };
    } else {
      current.body.push(e);
    }
  }
  if (current.user || current.body.length) turns.push(current);
  return turns;
}

