import type { ComponentType } from 'react';
import {
  Eye,
  FileText,
  Folder,
  GitCommit,
  Lightbulb,
  NotePencil,
  Paperclip,
  Question,
  Shield,
  Sparkle,
  SlidersHorizontal,
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
  onOpenAttachPicker: () => void;
  onOpenSettings: () => void;
  /** Kicks off a two-stage `mira review` and opens the side panel to stream
   *  progress. `args` is passed through as the git range (empty = default). */
  onRunReview: (args: string) => void;
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
    name: 'attach',
    aliases: ['file'],
    description: 'Attach a file to your next message',
    usage: '/attach',
    icon: Paperclip,
    takesArgs: false,
    run: (_a, ctx) => { ctx.onOpenAttachPicker(); return undefined; },
  },
  {
    name: 'plan',
    description: 'Ask for a step-by-step plan without touching files',
    usage: '/plan <task>',
    icon: Lightbulb,
    takesArgs: true,
    run: (args) => {
      const target = args.trim() || '<describe the task here>';
      return (
        `Read the relevant parts of the codebase, then propose a concrete ` +
        `step-by-step plan for the task below. Do NOT edit any files — ` +
        `planning only. List: files you'd touch, order of changes, tests ` +
        `to add or update, and any open questions.\n\n` +
        `Task: ${target}`
      );
    },
  },
  {
    name: 'commit',
    description: 'Compose a git commit for the current changes',
    usage: '/commit [hint]',
    icon: GitCommit,
    takesArgs: true,
    run: (args) => {
      const hint = args.trim();
      const hintLine = hint ? `\n\nThe user hinted the change is about: ${hint}` : '';
      return (
        `Run \`git status\` and \`git diff --stat\`, then \`git diff\` on ` +
        `the specific hunks. Propose a concise conventional-commit message ` +
        `(under 72 chars for the subject). Once I approve it, stage and ` +
        `commit the changes — do NOT push.` +
        hintLine
      );
    },
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
    name: 'init',
    description: 'Create a MIRA.md documenting this project',
    usage: '/init',
    icon: FileText,
    takesArgs: false,
    run: () => (
      `Explore the current folder — read package/build files, the top-level ` +
      `README, and skim the main source directories. Then create a MIRA.md ` +
      `at the repo root with:\n\n` +
      `- One-line project summary\n` +
      `- Tech stack (languages, frameworks, key libraries)\n` +
      `- Repo layout (top-level dirs, what each is for)\n` +
      `- How to build / test / run\n` +
      `- Notable conventions or gotchas future-you would want to know\n\n` +
      `Keep it tight — under 150 lines. Skip boilerplate the reader would ` +
      `already know from the ecosystem.`
    ),
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

/** Palette-visibility rule: text starts with `/` and doesn't yet have a
 *  space (still typing command name). Once space is typed, we hide the
 *  palette and match against known commands for argument-mode hint. */
export function slashState(
  text: string,
):
  | { mode: 'palette'; query: string }
  | { mode: 'args'; command: SlashCommand; args: string }
  | { mode: 'none' } {
  if (!text.startsWith('/')) return { mode: 'none' };
  const body = text.slice(1);
  const spaceIdx = body.indexOf(' ');
  if (spaceIdx < 0) return { mode: 'palette', query: body };
  const name = body.slice(0, spaceIdx).toLowerCase();
  const command = findCommand(name);
  if (!command) return { mode: 'palette', query: name };
  return { mode: 'args', command, args: body.slice(spaceIdx + 1) };
}

export function filterCommands(query: string): SlashCommand[] {
  const q = query.toLowerCase().trim();
  if (!q) return COMMANDS;
  return COMMANDS.filter((c) => {
    if (c.name.includes(q)) return true;
    if (c.aliases?.some((a) => a.includes(q))) return true;
    if (c.description.toLowerCase().includes(q)) return true;
    return false;
  });
}

function findCommand(name: string): SlashCommand | undefined {
  return COMMANDS.find((c) => c.name === name || c.aliases?.includes(name));
}
