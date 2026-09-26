import type { ComponentType } from 'react';
import {
  ArrowCounterClockwise,
  ArrowsInLineVertical,
  Brain,
  Eye,
  Folder,
  Lightbulb,
  NotePencil,
  Paperclip,
  Question,
  Shield,
  Sparkle,
  SlidersHorizontal,
  Target,
  Lightning,
  TerminalWindow,
} from '@phosphor-icons/react';
import type { Mode } from '../types';
import type { CommandInfo, Origin } from '../api';

/**
 * Slash-command registry.
 *
 * Three shapes, distinguished by the return type of `run()`:
 *
 *  - **control (silent)** → returns `undefined`. Fires an ambient effect
 *    (new chat, mode swap, opens a picker, etc.). Composer clears after.
 *
 *  - **template** → returns a string. Replaces composer contents so the
 *    user can review + tweak before hitting Enter to actually send.
 *
 *  Errors from control commands throw and render inline as amber hints.
 */
export type SlashCtx = {
  onNewChat: () => void;
  onSetMode: (m: Mode) => void;
  onSetModel: (m: string) => void;
  onOpenModelPicker: () => void;
  onOpenFolderPicker: () => void;
  /** Fire the browser's native OS file picker. Multi-select is supported;
   *  the composer handles read + chip render. Kept as a control callback
   *  (not "OpenX") so the Composer can prompt with `<input type="file">`
   *  and receive the result in one flow. */
  onOpenNativeFiles: () => void;
  onOpenSettings: () => void;
  /** Kicks off a two-stage `mira review` and opens the side panel to stream
   *  progress. `args` is passed through as the git range (empty = default). */
  onRunReview: (args: string) => void;
  /** Set or replace the session's autonomous `/goal`. Empty condition
   *  is a no-op — the composer throws so the user sees inline hint. */
  onSetGoal: (condition: string, maxIterations?: number) => void;
  /** Drop the standing goal (if any). */
  onClearGoal: () => void;
  /** Summarize the conversation now, keeping `focus` in view. */
  onCompact: (focus: string) => void;
  /** Flip the composer into "goal compose" mode — the next Enter
   *  fires `onSetGoal(text)` instead of sending as a chat message.
   *  Owned by the Composer; the slash command dispatches into it. */
  onEnterGoalCompose: () => void;
  /** Append a note to the current project's MIRA.md (or the user-global one
   *  when `scope === 'user'`). Feedback message returned via the promise. */
  onRemember: (scope: 'user' | 'project', text: string) => Promise<string>;
  /** Revert the last N file writes the agent made this session. Resolves
   *  with a human-readable summary; rejects with a message on error. */
  onUndo: (count: number) => Promise<string>;
  /** Invoke a named skill by sending a canned "use the `<name>` skill."
   *  message that the model reads and turns into a `Skill` tool call.
   *  Wired by the Composer so the same code path handles both palette
   *  picks and typed-and-Enter'd skill invocations. */
  onInvokeSkill: (name: string) => void;
};

/** Structural type covering both Lucide and Phosphor icon components — just
 *  needs to accept an optional className. Renamed from `LucideIcon` after the
 *  icon library swap. */
export type IconComponent = ComponentType<{ className?: string }>;

export type SlashCommand = {
  name: string;
  aliases?: string[];
  description: string;
  usage: string;
  icon: IconComponent;
  /** True when free-text after the name is meaningful (arg mode). */
  takesArgs: boolean;
  /** True for skill invocations. The Composer special-cases these so
   *  picking a skill from the palette REPLACES the `/query` trigger
   *  with a `@skill:<name>` mention chip instead of firing an ambient
   *  effect + clearing the composer. Distinguishes from ordinary
   *  "control" commands (`/new`, `/mode plan`, `/settings`, …) whose
   *  `run` fires a side effect and returns `undefined`. */
  isSkill?: boolean;
  /** Custom command (user, project or plugin) or MCP prompt. The
   *  composer sends `/name args` as the message and the server expands
   *  it; `run` is never called. */
  isCustom?: boolean;
  /** Where a custom command comes from, e.g. `plugin:commit-commands`. */
  source?: string;
  /** The plugin, MCP server or folder it comes from. Built-ins have none. */
  origin?: Origin;
  /** What it is, shown on its row: "Command", "Skill", "Prompt". */
  kindLabel?: string;
  /** Name to show when it differs from what you type (an MCP prompt's
   *  own name instead of `mcp__server__prompt`). */
  label?: string;
  run: (args: string, ctx: SlashCtx) => string | undefined;
};

/** A custom command or MCP prompt from `/api/commands` as a palette
 *  entry. */
export function customToCommand(c: CommandInfo): SlashCommand {
  const prompt = c.kind === 'prompt';
  return {
    name: c.name,
    // `mcp__notion__search` is what you type; `search` is what it is.
    label: prompt ? c.name.split('__').slice(2).join('__') || c.name : undefined,
    description: c.description,
    usage: `/${c.name}${c.argument_hint ? ` ${c.argument_hint}` : ''}`,
    icon: prompt ? Lightning : TerminalWindow,
    takesArgs: !!c.argument_hint,
    isCustom: true,
    source: c.source,
    origin: c.origin,
    kindLabel: prompt ? 'Prompt' : 'Command',
    run: () => undefined,
  };
}

/** Icon for an origin: its own, else the site's favicon (for an MCP
 *  server at `mcp.notion.com`, notion.com's). */
export function originIconSrc(o: Origin | undefined): string | null {
  if (!o) return null;
  if (o.icon_url) return o.icon_url;
  if (!o.homepage) return null;
  try {
    const host = new URL(o.homepage).hostname.replace(/^(mcp|api|www)\./, '');
    if (['github.com', 'gitlab.com', 'bitbucket.org', 'npmjs.com', 'pypi.org', 'localhost', '127.0.0.1'].includes(host)) {
      return null;
    }
    return `https://www.google.com/s2/favicons?domain=${host}&sz=64`;
  } catch {
    return null;
  }
}

/** A palette section: Mira's own commands, your skills, then one per
 *  plugin or MCP server. */
export type PaletteGroup = { key: string; label: string; hint?: string; origin?: Origin; items: SlashCommand[] };

const ORIGIN_HINT: Record<Origin['kind'], string> = {
  plugin: 'Plugin',
  mcp: 'MCP server',
  user: '~/.mira/commands',
  project: '.mira/commands',
};

export function groupCommands(matches: SlashCommand[]): PaletteGroup[] {
  const groups = new Map<string, PaletteGroup>();
  const add = (key: string, make: () => Omit<PaletteGroup, 'items'>, cmd: SlashCommand) => {
    if (!groups.has(key)) groups.set(key, { ...make(), items: [] });
    groups.get(key)!.items.push(cmd);
  };
  for (const c of matches) {
    if (c.origin) {
      const o = c.origin;
      add(o.key, () => ({ key: o.key, label: o.label, hint: ORIGIN_HINT[o.kind], origin: o }), c);
    } else if (c.isSkill) {
      add('skills', () => ({ key: 'skills', label: 'Skills' }), c);
    } else {
      add('mira', () => ({ key: 'mira', label: 'Commands' }), c);
    }
  }
  const byName = (a: SlashCommand, b: SlashCommand) =>
    (a.label ?? a.name).localeCompare(b.label ?? b.name);
  const rank = (g: PaletteGroup) =>
    g.key === 'mira' ? 0 : g.key === 'skills' ? 1 : g.origin?.kind === 'project' ? 2 : g.origin?.kind === 'user' ? 3 : 4;
  return [...groups.values()]
    .map((g) => ({ ...g, items: [...g.items].sort(byName) }))
    .sort((a, b) => rank(a) - rank(b) || a.label.localeCompare(b.label));
}

const MODE_VALUES: Mode[] = ['plan', 'manual', 'auto', 'edit', 'yolo'];

export const COMMANDS: SlashCommand[] = [
  {
    name: 'new',
    aliases: ['clear'],
    description: 'Start a new chat in this folder',
    usage: '/new',
    icon: NotePencil,
    takesArgs: false,
    run: (_a, ctx) => { ctx.onNewChat(); return undefined; },
  },
  {
    name: 'model',
    description: 'Pick a model (or pass an id inline)',
    usage: '/model [query]',
    icon: Sparkle,
    takesArgs: true,
    run: (args, ctx) => {
      const q = args.trim();
      // Bare `/model` opens the picker — better UX than forcing the user
      // to remember an id. `/model gpt-4o` sets it directly.
      if (!q) { ctx.onOpenModelPicker(); return undefined; }
      ctx.onSetModel(q);
      return undefined;
    },
  },
  {
    name: 'mode',
    description: 'Change approval mode',
    usage: '/mode <plan|manual|auto|edit|yolo>',
    icon: Shield,
    takesArgs: true,
    run: (args, ctx) => {
      const m = args.trim().toLowerCase();
      if (!m) throw new Error(`usage: /mode <${MODE_VALUES.join('|')}>`);
      if (!MODE_VALUES.includes(m as Mode)) {
        throw new Error(`unknown mode "${m}" — try: ${MODE_VALUES.join(', ')}`);
      }
      ctx.onSetMode(m as Mode);
      return undefined;
    },
  },
  {
    name: 'folder',
    aliases: ['cwd', 'cd'],
    description: 'Switch working folder',
    usage: '/folder',
    icon: Folder,
    takesArgs: false,
    run: (_a, ctx) => { ctx.onOpenFolderPicker(); return undefined; },
  },
  {
    // Codex-style "Files and folders" — opens the OS native file picker
    // via a hidden `<input type="file" multiple>` inside the Composer.
    // Aliased to /attach + /file so muscle memory still works.
    name: 'files',
    aliases: ['file', 'attach'],
    description: 'Attach files from your computer',
    usage: '/files',
    icon: Paperclip,
    takesArgs: false,
    run: (_a, ctx) => { ctx.onOpenNativeFiles(); return undefined; },
  },
  {
    name: 'plan',
    description: 'Toggle plan mode — the agent will call `plan` before acting',
    usage: '/plan',
    icon: Lightbulb,
    takesArgs: false,
    // Simply flips mode to `plan`. The Plan chip in the composer handles the
    // "toggle back" gesture (it remembers your prior mode). Sending a real
    // message while plan mode is on lets the model use the plan tool
    // naturally — no more wrapping the user's text with planning boilerplate.
    run: (_a, ctx) => { ctx.onSetMode('plan'); return undefined; },
  },
  {
    name: 'review',
    description: 'Two-stage diff review (generate + hostile re-verify)',
    usage: '/review [range]',
    icon: Eye,
    takesArgs: true,
    // Action command — kicks off the real `mira review` on the server and
    // opens the side panel. Returns `undefined` so the composer clears
    // instead of prefilling a text template.
    run: (args, ctx) => { ctx.onRunReview(args.trim()); return undefined; },
  },
  {
    name: 'compact',
    description: 'Summarize the conversation to free up context',
    usage: '/compact [what to focus on]',
    icon: ArrowsInLineVertical,
    takesArgs: true,
    run: (args, ctx) => { ctx.onCompact(args.trim()); return undefined; },
  },
  {
    name: 'goal',
    description: 'Set an autonomous goal — mira loops until the condition is met',
    usage: '/goal   (enter goal-compose · then type the condition)',
    icon: Target,
    // `takesArgs: false` so a palette pick + Enter runs immediately —
    // that flips the composer into "goal-compose" mode without a
    // second keystroke. Inline `/goal clear`, `/goal status`, and
    // `/goal <condition>` still work: slashState routes any `/goal `
    // (trailing space) into args mode, and `run` branches on `body`.
    takesArgs: false,
    run: (args, ctx) => {
      const body = args.trim();
      if (!body) {
        // Bare — enter goal-compose. Composer flips its submit target
        // and the purple Goal chip appears.
        ctx.onEnterGoalCompose();
        return undefined;
      }
      if (body === 'status') {
        throw new Error(
          'goal status is shown in the top card + the purple chip when active.',
        );
      }
      if (body === 'clear') {
        ctx.onClearGoal();
        return undefined;
      }
      // Optional `--max N` prefix bumps the iteration cap. Everything else
      // is treated as the condition body.
      let condition = body;
      let maxIter: number | undefined;
      const maxMatch = body.match(/^--max\s+(\d+)\s+([\s\S]+)$/);
      if (maxMatch) {
        maxIter = Number.parseInt(maxMatch[1], 10);
        condition = maxMatch[2].trim();
      }
      if (!condition) throw new Error('usage: /goal <condition>');
      ctx.onSetGoal(condition, maxIter);
      return undefined;
    },
  },
  // `/commit` and `/init` used to live here as template commands. They
  // were superseded by the `commit` and `init` bundled *skills*, which
  // ship richer playbooks (verification steps, "do not" lists) and are
  // user-editable via `~/.mira/skills/`. The dedupe pass in
  // `filterCommands` guarantees a skill wins over a same-named built-in
  // even if a stray entry sneaks back in.
  {
    name: 'undo',
    description: 'Revert the last N file writes the agent made in this session',
    usage: '/undo [N]',
    icon: ArrowCounterClockwise,
    takesArgs: true,
    run: (args, ctx) => {
      const trimmed = args.trim();
      const n = trimmed ? Number.parseInt(trimmed, 10) : 1;
      if (!Number.isFinite(n) || n < 1) {
        throw new Error('usage: /undo [N]  (N is a positive integer)');
      }
      // Fire and forget — success/error surfaces in console + the transcript
      // sees the reverted files reflected on the next read.
      ctx.onUndo(n).catch((e) => {
        // eslint-disable-next-line no-console
        console.error('undo failed:', e);
      });
      return undefined;
    },
  },
  {
    name: 'remember',
    aliases: ['note'],
    description: 'Append a note to project memory (.mira/MIRA.md)',
    usage: '/remember <note>   (add --user to save to ~/.mira/MIRA.md)',
    icon: Brain,
    takesArgs: true,
    run: (args, ctx) => {
      // Optional `--user` prefix routes to the global file. Everything else
      // is treated as the note body and appended to the current project's
      // MIRA.md. Defaults to project so the common case is one token.
      const trimmed = args.trim();
      if (!trimmed) throw new Error('usage: /remember <note>');
      let scope: 'user' | 'project' = 'project';
      let text = trimmed;
      if (text.startsWith('--user ') || text === '--user') {
        scope = 'user';
        text = text.slice('--user'.length).trim();
      }
      if (!text) throw new Error('usage: /remember [--user] <note>');
      // Fire and forget — success is silent (the note is now in MIRA.md),
      // failure surfaces in the browser console. A toast system would be
      // the right long-term home for this feedback.
      ctx.onRemember(scope, text).catch((e) => {
        // eslint-disable-next-line no-console
        console.error('remember failed:', e);
      });
      return undefined;
    },
  },
  {
    name: 'settings',
    description: 'Open the settings panel',
    usage: '/settings',
    icon: SlidersHorizontal,
    takesArgs: false,
    run: (_a, ctx) => { ctx.onOpenSettings(); return undefined; },
  },
  {
    name: 'help',
    description: 'Show all available commands',
    usage: '/help',
    icon: Question,
    takesArgs: false,
    run: () => (
      `Available commands:\n\n` +
      COMMANDS.map((c) => `  ${c.usage.padEnd(28)} ${c.description}`).join('\n') +
      `\n\nType / in the composer to browse the palette interactively.`
    ),
  },
];

/** Palette-visibility rule: the composer contains a `/` or `@` trigger
 *  either at position 0 (the classic "start typing / to open the
 *  palette") OR at the tail of the text preceded by whitespace or a
 *  chip mention (so `hello /gril` and `@skill:foo /gril` also open the
 *  palette). Once a space is typed after the command name, we switch
 *  to args mode. `@` promotes the `files` command to the top so a
 *  bare `@` + Enter fires the OS native file picker.
 *
 *  `triggerStart` is the index of the trigger character in `text`.
 *  Callers use it to splice the trigger + query span out when the user
 *  picks a palette entry so mid-message triggers don't clobber the
 *  surrounding prose.
 *
 *  Args mode (`/name arg`) is only detected when the trigger is at
 *  position 0 — a mid-message `/mode plan` is unusual and would
 *  otherwise fight with normal typing (spaces mean "end command"
 *  mid-sentence). */
export function slashState(
  text: string,
  custom: SlashCommand[] = [],
):
  | { mode: 'palette'; query: string; trigger: '/' | '@'; triggerStart: number }
  | { mode: 'args'; command: SlashCommand; args: string; trigger: '/' | '@'; triggerStart: number }
  | { mode: 'none' } {
  const first = text[0];
  if (first === '/' || first === '@') {
    const trigger = first as '/' | '@';
    const body = text.slice(1);
    const spaceIdx = body.indexOf(' ');
    if (spaceIdx < 0) {
      return { mode: 'palette', query: body, trigger, triggerStart: 0 };
    }
    const rawName = body.slice(0, spaceIdx);
    const name = rawName.toLowerCase();
    const command = custom.find((c) => c.name === rawName) ?? findCommand(name);
    if (!command) {
      return { mode: 'palette', query: name, trigger, triggerStart: 0 };
    }
    return { mode: 'args', command, args: body.slice(spaceIdx + 1), trigger, triggerStart: 0 };
  }
  // Tail-of-text trigger: `<prefix><ws|mention><trigger><word chars>`
  // with no trailing space. Preserves the mid-message palette flow
  // (`hello /gril` opens the palette on `gril`) without needing caret
  // tracking. Skipped when the text starts with a trigger — that's
  // already handled above.
  const m = /(?:^|\s|@skill:[A-Za-z0-9_-]+)([/@])([A-Za-z0-9_-]*)$/.exec(text);
  if (m) {
    const trigger = m[1] as '/' | '@';
    const query = m[2];
    // `m.index` points at the start of the whole match — advance past
    // the leading char (whitespace or the end of a mention token) so
    // `triggerStart` lands on the trigger itself.
    const leadLen = m[0].length - (trigger.length + query.length);
    return {
      mode: 'palette',
      query,
      trigger,
      triggerStart: (m.index ?? 0) + leadLen,
    };
  }
  return { mode: 'none' };
}

/** One entry in the dynamic skill list the composer merges into the
 *  palette. Shape mirrors the `/api/skills` response so the caller can
 *  pass the fetch result through unchanged. */
export type PaletteSkill = {
  name: string;
  description: string;
  tier?: 'bundled' | 'shared' | 'user' | 'project';
  /** Frontmatter `color:` — powers chip tinting in the composer and
   *  the sent user bubble. Optional; when absent the renderer hashes
   *  the name to a stable palette entry so unopinionated skills still
   *  read as distinct. */
  color?: string;
  /** Frontmatter `icon:` — currently unused by the chip renderer (it
   *  ships with a leading dot), but forwarded so future variants can
   *  swap in a Phosphor icon per skill. */
  icon?: string;
  /** The plugin it comes from, if any. */
  origin?: Origin;
};

/** Palette results.
 *
 *  When `trigger === '@'`, the `files` command jumps to the top of the
 *  list (matching Codex's "@ → Files and folders" behaviour). Otherwise
 *  the natural COMMANDS order is preserved.
 *
 *  Empty query returns every command; a non-empty query still filters
 *  by name/alias/description, but the same promotion applies so that
 *  even a partial `@fi` keeps `files` at index 0.
 *
 *  `skills` is the dynamically-loaded roster from `/api/skills`. Each
 *  becomes a `/<skill-name>` command that fires `ctx.onInvokeSkill`
 *  on pick. Skills sit *after* the built-in commands in the palette so
 *  a user typing `/co` still sees `/commit` first, not a skill named
 *  `code-audit`. */
export function filterCommands(
  query: string,
  trigger: '/' | '@' = '/',
  skills: PaletteSkill[] = [],
  custom: SlashCommand[] = [],
): SlashCommand[] {
  const q = query.toLowerCase().trim();
  // Custom commands (project/user/plugin) and MCP prompts ride with the
  // skills: they win a name clash with a built-in.
  const skillCmds = [...custom, ...skills.map(skillToCommand)];
  // Dedupe by name — when a skill collides with a built-in (e.g. a user
  // drops `~/.mira/skills/commit/` that shadows the built-in `commit`
  // template), the skill wins. Otherwise the palette would show two
  // rows with the same `/name` and different behaviour.
  const skillNames = new Set(skillCmds.map((c) => c.name.toLowerCase()));
  const uniqueBuiltins = COMMANDS.filter(
    (c) => !skillNames.has(c.name.toLowerCase()),
  );
  const pool = [...uniqueBuiltins, ...skillCmds];
  const base = q
    ? pool.filter((c) => {
        if (c.name.toLowerCase().includes(q)) return true;
        if (c.aliases?.some((a) => a.toLowerCase().includes(q))) return true;
        if (c.description.toLowerCase().includes(q)) return true;
        // Typing a plugin or server's name lists everything it adds.
        if (c.origin?.label.toLowerCase().includes(q)) return true;
        return false;
      })
    : pool;
  if (trigger !== '@') return base;
  const files = base.find((c) => c.name === 'files');
  if (!files) return base;
  return [files, ...base.filter((c) => c.name !== 'files')];
}

/** Wrap a loaded skill as a slash command so the palette can render it
 *  next to the built-in commands. Picking the entry fires
 *  `ctx.onInvokeSkill(name)`, which the Composer turns into a canned
 *  "use the `<name>` skill." user message. The description is what the
 *  model sees when deciding to actually call the `Skill` tool. */
function skillToCommand(s: PaletteSkill): SlashCommand {
  return {
    name: s.name,
    description: s.description,
    usage: `/${s.name}`,
    icon: Sparkle,
    takesArgs: false,
    isSkill: true,
    origin: s.origin,
    kindLabel: 'Skill',
    // The Composer handles skill picks directly (see `commitPaletteChoice`),
    // so this `run` is only reached if something bypasses the palette and
    // invokes the command by name — kept as a safe fallback.
    run: (_a, ctx) => {
      ctx.onInvokeSkill(s.name);
      return undefined;
    },
  };
}

function findCommand(name: string): SlashCommand | undefined {
  return COMMANDS.find((c) => c.name === name || c.aliases?.includes(name));
}
