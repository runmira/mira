import type { AcpSessionMode, Mode } from '../types';
import type { ToolCall } from '../types';

/**
 * The five permission postures, in escalating order.
 *
 * This is the entire vocabulary for "what may the agent do without asking".
 * It is defined once, here, so a tool request and a mode change offer
 * literally the same options instead of two dialogs that almost agree:
 * previously the composer offered Deny / Allow / Always-allow while the
 * agent-mode dialog offered the agent's raw mode ids, and using an agent
 * meant learning both.
 */
export type PostureKey = 'plan' | 'ask' | 'edits' | 'auto' | 'yolo';

export type Posture = {
  key: PostureKey;
  /** Short label, e.g. "Plan only". */
  label: string;
  /** One line on what this means in practice. */
  blurb: string;
  /** Matches against an agent's mode id or name, first hit wins. */
  match: RegExp[];
};

export const POSTURES: Posture[] = [
  {
    key: 'plan',
    label: 'Plan only',
    blurb: 'The agent plans and explains, but changes nothing.',
    match: [/^plan$/i, /read-only/i, /readonly/i],
  },
  {
    key: 'ask',
    label: 'Ask each time',
    blurb: 'The agent asks before every consequential action.',
    match: [/^(default|ask)$/i, /^agent$/i, /approval-required/i],
  },
  {
    key: 'edits',
    label: 'Auto edits',
    blurb: 'File edits go through; commands still ask.',
    match: [/accept/i],
  },
  {
    key: 'auto',
    label: 'Auto everything',
    blurb: 'The agent acts on its own judgement within the workspace.',
    match: [/^auto$/i],
  },
  {
    key: 'yolo',
    label: 'YOLO',
    blurb: 'No prompts. Everything the agent attempts is allowed.',
    match: [/^(dontask|don't ask|never-ask|bypass)$/i, /bypass/i, /full-access/i, /full_access/i, /danger/i],
  },
];

/**
 * Mira's own modes speak the same five postures under slightly different
 * names (`manual` is "ask each time", `auto` is "auto edits", `edit` is
 * "auto everything"). The mapping is what lets one picker drive both the
 * harness and an external agent — the user was right that they are almost
 * the same commands.
 */
export const MIRA_MODE_TO_POSTURE: Record<Mode, PostureKey> = {
  plan: 'plan',
  manual: 'ask',
  auto: 'edits',
  edit: 'auto',
  yolo: 'yolo',
};

/** Reverse lookup: posture back to the Mira mode that means it. */
export const POSTURE_TO_MIRA_MODE: Record<PostureKey, Mode> = {
  plan: 'plan',
  ask: 'manual',
  edits: 'auto',
  auto: 'edit',
  yolo: 'yolo',
};

export type PostureOption = {
  posture: Posture;
  /** The agent's mode id this posture maps to. */
  modeId: string;
  /** The agent's own name for it, when it differs from the posture label. */
  modeName: string;
  /** The agent's own description, when it has one. */
  modeDescription?: string | null;
  /** True when this is already the active mode. */
  current: boolean;
};

/**
 * Map the canonical postures onto an agent's advertised modes.
 *
 * Only postures with a matching mode are returned — offering "YOLO" to an
 * agent that has no such mode would be offering a lie. Modes nothing matches
 * are dropped rather than appended: the point is one fixed vocabulary, not
 * the agent's raw ids under new paint.
 */
export function mapPosturesToModes(
  available: AcpSessionMode[],
  currentId: string | null,
): PostureOption[] {
  const used = new Set<string>();
  const out: PostureOption[] = [];
  for (const posture of POSTURES) {
    const hit = available.find(
      (m) => !used.has(m.id) && posture.match.some((re) => re.test(m.id) || re.test(m.name)),
    );
    if (!hit) continue;
    used.add(hit.id);
    out.push({
      posture,
      modeId: hit.id,
      modeName: hit.name,
      modeDescription: hit.description,
      current: hit.id === currentId,
    });
  }
  return out;
}

/** What an agent's tool request looks like once parsed for display. */
export type ToolRequestSummary = {
  /** e.g. "Bash". */
  tool: string;
  /** The input the user should judge, pretty-printed later. */
  input: unknown;
  /** Raw arguments, for callers that need more. */
  rawArguments: string;
};

/**
 * Parse a pending tool call for the decision dialog.
 *
 * Native agents send `{tool_call_id, title, input, source: "native"}`;
 * ACP adapters send `{tool_call_id, title, options}`; anything else is shown
 * as its function name plus raw arguments rather than hidden.
 */
export function summarizeToolRequest(call: ToolCall): ToolRequestSummary {
  let parsed: unknown = null;
  try {
    parsed = JSON.parse(call.function.arguments);
  } catch {
    parsed = null;
  }
  if (parsed && typeof parsed === 'object' && !Array.isArray(parsed)) {
    const o = parsed as Record<string, unknown>;
    const title = typeof o['title'] === 'string' ? o['title'] : null;
    if (title) {
      return {
        tool: title,
        input: o['input'] ?? o['options'] ?? o,
        rawArguments: call.function.arguments,
      };
    }
  }
  return {
    tool: call.function.name.replace(/^acp:\s*/, ''),
    input: parsed ?? call.function.arguments,
    rawArguments: call.function.arguments,
  };
}

/** Render tool input compactly: one line per scalar, capped. */
export function prettyToolInput(input: unknown, maxChars = 600): string {
  let text: string;
  if (typeof input === 'string') {
    text = input;
  } else {
    try {
      text = JSON.stringify(input, null, 2) ?? '';
    } catch {
      text = String(input);
    }
  }
  return text.length > maxChars ? text.slice(0, maxChars) + '…' : text;
}
