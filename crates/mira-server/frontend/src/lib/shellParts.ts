/**
 * Split a shell command into the operations a reviewer approves.
 *
 * `git log -3 && git status; ls | head` is four things, and an approval
 * card that shows it as one long line hides which of them is the risky
 * one. Splits on top-level `&&`, `||`, `;`, `|` and newlines — never inside
 * quotes, `$(…)` or backticks — and keeps the joiner, so the parts read
 * back as the original command.
 */
export type ShellPart = { text: string; joiner: string | null };

export function splitShellCommand(cmd: string): ShellPart[] {
  const parts: ShellPart[] = [];
  let buf = '';
  let quote: '"' | "'" | '`' | null = null;
  let depth = 0; // $( … ) nesting
  const push = (joiner: string | null) => {
    const t = buf.trim();
    if (t) parts.push({ text: t, joiner });
    else if (parts.length && joiner) parts[parts.length - 1].joiner = joiner;
    buf = '';
  };
  for (let i = 0; i < cmd.length; i++) {
    const c = cmd[i];
    const next = cmd[i + 1];
    if (quote) {
      buf += c;
      if (c === '\\' && quote !== "'" && next !== undefined) {
        buf += next;
        i++;
      } else if (c === quote) {
        quote = null;
      }
      continue;
    }
    if (c === '\\' && next !== undefined) {
      buf += c + next;
      i++;
      continue;
    }
    if (c === '"' || c === "'" || c === '`') {
      quote = c;
      buf += c;
      continue;
    }
    if (c === '$' && next === '(') {
      depth++;
      buf += '$(';
      i++;
      continue;
    }
    if (depth > 0) {
      if (c === '(') depth++;
      if (c === ')') depth--;
      buf += c;
      continue;
    }
    if ((c === '&' && next === '&') || (c === '|' && next === '|')) {
      push(c + next);
      i++;
      continue;
    }
    if (c === ';' || c === '\n' || c === '|') {
      push(c === '\n' ? ';' : c);
      continue;
    }
    buf += c;
  }
  push(null);
  // A trailing joiner has nothing after it.
  if (parts.length) parts[parts.length - 1].joiner = null;
  return mergeBlocks(parts);
}

const OPENERS: Record<string, string> = { for: 'done', while: 'done', until: 'done', select: 'done', if: 'fi', case: 'esac' };
const CLOSERS = new Set(['done', 'fi', 'esac']);

/** Keep a loop or conditional as one part: `for …; do …; done` is one
 *  thing to approve, not three fragments that mean nothing alone. */
function mergeBlocks(parts: ShellPart[]): ShellPart[] {
  const out: ShellPart[] = [];
  let open: ShellPart | null = null;
  let depth = 0;
  for (const p of parts) {
    const first = p.text.split(/\s+/)[0];
    const last = p.text.split(/\s+/).pop() ?? '';
    if (open) {
      open.text += `${open.joiner === ';' ? '; ' : ` ${open.joiner} `}${p.text}`;
      open.joiner = p.joiner;
      if (OPENERS[first]) depth++;
      if (CLOSERS.has(first) || CLOSERS.has(last)) depth--;
      if (depth <= 0) {
        out.push(open);
        open = null;
      }
      continue;
    }
    if (OPENERS[first] && !CLOSERS.has(last)) {
      open = { ...p };
      depth = 1;
      continue;
    }
    out.push(p);
  }
  if (open) out.push(open);
  return out;
}

/**
 * The parts an agent's reason names as needing approval. Claude Code says
 * "…The following parts require approval: git -C a log, git -C b log"
 * for compound commands; anything else yields none.
 */
export function partsNeedingApproval(reason: string | null | undefined): string[] {
  if (!reason) return [];
  const m = reason.match(/require[s]? approval:\s*([\s\S]+)$/i);
  if (!m) return [];
  // Split on ", " only where the next part starts a new command word, so a
  // comma inside a part (a format string) does not split it.
  return m[1]
    .split(/,\s+(?=[A-Za-z0-9_./~-]+(?:\s|$))/)
    .map((s) => s.trim())
    .filter(Boolean);
}
