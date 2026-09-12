import type { ComponentType } from 'react';
import {
  ArrowCounterClockwise,
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
} from '@phosphor-icons/react';
import type { Mode } from '../types';

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
  run: (args: string, ctx: SlashCtx) => string | undefined;
};

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

/** Palette-visibility rule: text starts with `/` OR `@` and doesn't yet
 *  have a space (still typing command name). Once space is typed, we
 *  hide the palette and match against known commands for argument-mode
 *  hint. `@` is a Codex-style shorthand for "attach a file" — it opens
 *  the same palette but promotes the `files` command to the top so a
 *  bare `@` + Enter fires the OS native file picker without further
 *  keystrokes. Any command still matches: `@goal ship it` works. */
export function slashState(
  text: string,
):
  | { mode: 'palette'; query: string; trigger: '/' | '@' }
  | { mode: 'args'; command: SlashCommand; args: string; trigger: '/' | '@' }
  | { mode: 'none' } {
  const first = text[0];
  if (first !== '/' && first !== '@') return { mode: 'none' };
  const trigger = first as '/' | '@';
  const body = text.slice(1);
  const spaceIdx = body.indexOf(' ');
  if (spaceIdx < 0) return { mode: 'palette', query: body, trigger };
  const name = body.slice(0, spaceIdx).toLowerCase();
  const command = findCommand(name);
  if (!command) return { mode: 'palette', query: name, trigger };
  return { mode: 'args', command, args: body.slice(spaceIdx + 1), trigger };
}

/** One entry in the dynamic skill list the composer merges into the
 *  palette. Shape mirrors the `/api/skills` response so the caller can
 *  pass the fetch result through unchanged. */
export type PaletteSkill = {
  name: string;
  description: string;
  tier?: 'bundled' | 'user' | 'project';
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
): SlashCommand[] {
  const q = query.toLowerCase().trim();
  const skillCmds = skills.map(skillToCommand);
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
    run: (_a, ctx) => {
      ctx.onInvokeSkill(s.name);
      return undefined;
    },
  };
}

function findCommand(name: string): SlashCommand | undefined {
  return COMMANDS.find((c) => c.name === name || c.aliases?.includes(name));
}
