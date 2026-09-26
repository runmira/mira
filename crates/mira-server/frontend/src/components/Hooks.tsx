/**
 * Settings → Hooks: make Mira do something at key moments (get a
 * notification, have the AI double-check, run your own script) without
 * writing YAML. Saves to `hooks:` in ~/.mira/mira.yaml via /api/hooks.
 */
import React, { useEffect, useMemo, useState } from 'react';
import {
  Bell,
  CircleNotch,
  Lightning,
  PencilSimple,
  Plus,
  Robot,
  TerminalWindow,
  Trash,
  Warning,
} from '@phosphor-icons/react';
import { getHooks, saveHooks, type HookRule, type HooksView } from '../api';
import { SectionInput } from '@/components/ui/input';
import { Select } from '@/components/ui/select';
import { Button } from '@/components/ui/button';
import { cn } from '@/lib/utils';

/* ---------- what can happen, in plain words ---------- */

type Action = 'notify' | 'check' | 'command';

type Moment = {
  event: string;
  /** "When …" */
  label: string;
  /** One line on when exactly it fires. */
  help: string;
  /** Picks which tools it's about. */
  tools?: boolean;
  actions: Action[];
  /** For "Ask the AI to check": an example, and what a "no" does. */
  check?: { example: string; onNo: string };
  /** For "Run a command": what the command's output does here. */
  commandNote?: string;
  notifyMessage?: string;
};

const MOMENTS: Moment[] = [
  {
    event: 'Notification',
    label: 'Mira needs your approval',
    help: 'A step is waiting for you to allow or deny it.',
    actions: ['notify', 'command'],
    notifyMessage: 'Mira needs your approval',
  },
  {
    event: 'Stop',
    label: 'Mira finishes a reply',
    help: 'Mira is done and about to hand back to you.',
    actions: ['notify', 'check', 'command'],
    check: {
      example: 'Did it finish what I asked, and run the tests if it changed code?',
      onNo: 'If the answer is no, Mira keeps working and is told why.',
    },
    commandNote: 'To make Mira keep working, exit with code 2 and print the reason.',
    notifyMessage: 'Mira is done',
  },
  {
    event: 'PreToolUse',
    label: 'Before Mira uses a tool',
    help: 'Runs before the step happens, so it can stop it.',
    tools: true,
    actions: ['check', 'command', 'notify'],
    check: {
      example: 'Is this safe? It must not delete anything outside the project.',
      onNo: 'If the answer is no, the step is blocked and Mira is told why.',
    },
    commandNote: 'To block the step, exit with code 2 and print the reason.',
    notifyMessage: 'Mira is about to use a tool',
  },
  {
    event: 'PostToolUse',
    label: 'After Mira uses a tool',
    help: 'Runs once the step is done, e.g. to format or lint a file.',
    tools: true,
    actions: ['command', 'notify'],
    commandNote: 'Anything it prints to stderr with exit code 2 is shown to Mira as feedback.',
    notifyMessage: 'Mira used a tool',
  },
  {
    event: 'UserPromptSubmit',
    label: 'You send a message',
    help: 'Runs before your message reaches the AI.',
    actions: ['check', 'command'],
    check: {
      example: 'Does the message avoid pasting passwords or API keys?',
      onNo: "If the answer is no, the message isn't sent and you see why.",
    },
    commandNote: 'What it prints is added to your message as extra context. Exit code 2 stops the message.',
  },
  {
    event: 'SessionStart',
    label: 'A chat starts',
    help: 'Runs once, at the first message of a chat.',
    actions: ['command'],
    commandNote: 'What it prints is added to what Mira knows for this chat (e.g. `git status`).',
  },
  {
    event: 'SubagentStop',
    label: 'A helper agent finishes',
    help: 'A helper Mira started to explore or work in parallel is done.',
    actions: ['notify', 'check', 'command'],
    check: {
      example: 'Did the helper answer the question it was given?',
      onNo: 'If the answer is no, the helper keeps working.',
    },
    notifyMessage: 'A helper agent finished',
  },
  {
    event: 'PreCompact',
    label: 'A long chat is about to be summarized',
    help: 'Older messages are about to be condensed to make room.',
    actions: ['command', 'notify'],
    commandNote: 'Good for saving a copy of the conversation first. It can’t stop the summary.',
    notifyMessage: 'Mira is summarizing a long chat',
  },
  {
    event: 'SessionEnd',
    label: 'You close Mira',
    help: 'The terminal app is exiting.',
    actions: ['command', 'notify'],
    notifyMessage: 'Mira closed',
  },
];

/** Tool choices: `label` for the dropdown, `does` for "Before Mira …". */
const TOOLS: { matcher: string; label: string; does: string }[] = [
  { matcher: '', label: 'Any tool', does: 'uses any tool' },
  { matcher: 'Bash', label: 'Shell commands', does: 'runs a shell command' },
  { matcher: 'Edit|MultiEdit|Write', label: 'File changes', does: 'changes a file' },
  { matcher: 'Read', label: 'Reading files', does: 'reads a file' },
  { matcher: 'WebFetch|WebSearch', label: 'Web access', does: 'uses the web' },
  { matcher: 'mcp__.*', label: 'Connected apps (MCP)', does: 'uses a connected app' },
];
const CUSTOM_TOOLS = '__custom__';

const ACTION_LABEL: Record<Action, string> = {
  notify: 'Show a notification',
  check: 'Ask the AI to check',
  command: 'Run a command',
};

const momentFor = (event: string) => MOMENTS.find((m) => m.event === event);

/* ---------- desktop notifications as commands ---------- */

function cleanMessage(s: string): string {
  return s.replace(/["'`$\\]/g, '').trim() || 'Mira';
}

function notifyCommand(os: string, message: string): string | null {
  const m = cleanMessage(message);
  if (os === 'macos') return `osascript -e 'display notification "${m}" with title "Mira"'`;
  if (os === 'linux') return `notify-send "Mira" "${m}"`;
  return null;
}

/** The message, if `command` is one of our notification commands. */
function notifyMessageOf(command: string | null | undefined): string | null {
  if (!command) return null;
  const mac = command.match(/^osascript -e 'display notification "(.*)" with title "Mira"'$/);
  if (mac) return mac[1];
  const linux = command.match(/^notify-send "Mira" "(.*)"$/);
  return linux ? linux[1] : null;
}

/* ---------- a rule, as the UI edits it ---------- */

type Draft = {
  event: string;
  toolChoice: string; // a TOOLS matcher, or CUSTOM_TOOLS
  customTools: string;
  action: Action;
  message: string;
  question: string;
  command: string;
  timeout: string;
};

function draftFrom(rule: HookRule): Draft {
  const matcher = rule.matcher ?? '';
  const known = TOOLS.some((t) => t.matcher === matcher);
  const note = rule.type === 'command' ? notifyMessageOf(rule.command) : null;
  return {
    event: rule.event,
    toolChoice: known ? matcher : CUSTOM_TOOLS,
    customTools: known ? '' : matcher,
    action: rule.type === 'prompt' ? 'check' : note != null ? 'notify' : 'command',
    message: note ?? momentFor(rule.event)?.notifyMessage ?? 'Mira',
    question: rule.prompt ?? '',
    command: rule.type === 'command' && note == null ? rule.command ?? '' : '',
    timeout: rule.timeout ? String(rule.timeout) : '',
  };
}

function freshDraft(event = 'Notification'): Draft {
  const m = momentFor(event)!;
  return {
    event,
    toolChoice: '',
    customTools: '',
    action: m.actions[0],
    message: m.notifyMessage ?? 'Mira',
    question: '',
    command: '',
    timeout: '',
  };
}

/** The rule to save, or why it can't be saved yet. */
function ruleFrom(d: Draft, os: string): HookRule | string {
  const m = momentFor(d.event);
  if (!m) return 'Pick when this should happen.';
  const matcher = m.tools
    ? (d.toolChoice === CUSTOM_TOOLS ? d.customTools.trim() : d.toolChoice) || null
    : null;
  const timeout = Number(d.timeout);
  const extra = Number.isFinite(timeout) && timeout > 0 ? { timeout } : {};
  if (d.action === 'check') {
    if (!d.question.trim()) return 'Write the question for the AI.';
    return { event: d.event, matcher, type: 'prompt', prompt: d.question.trim(), ...extra };
  }
  if (d.action === 'notify') {
    const command = notifyCommand(os, d.message);
    if (!command) return 'Notifications aren’t set up for this computer yet; use “Run a command”.';
    return { event: d.event, matcher, type: 'command', command, ...extra };
  }
  if (!d.command.trim()) return 'Write the command to run.';
  return { event: d.event, matcher, type: 'command', command: d.command.trim(), ...extra };
}

/** The moment as a sentence start: "When Mira finishes a reply",
 *  "Before Mira runs a shell command". */
function whenText(rule: HookRule): string {
  const m = momentFor(rule.event);
  if (m?.tools) {
    const tool = TOOLS.find((t) => t.matcher === (rule.matcher ?? ''));
    const does = tool ? tool.does : `uses a tool matching ${rule.matcher}`;
    return `${rule.event === 'PreToolUse' ? 'Before' : 'After'} Mira ${does}`;
  }
  const label = m?.label ?? rule.event;
  // Keep "Mira" capitalized; lower the rest ("You close Mira" → "you close Mira").
  const lowered = label.startsWith('Mira') ? label : label.charAt(0).toLowerCase() + label.slice(1);
  return `When ${lowered}`;
}

/** "When … → …", for the list. */
function describe(rule: HookRule): { when: string; does: React.ReactNode; icon: React.ReactNode } {
  const when = whenText(rule);
  if (rule.type === 'prompt') {
    return {
      when,
      icon: <Robot className="size-4" />,
      does: <>Ask the AI: <q className="italic">{rule.prompt}</q></>,
    };
  }
  const note = notifyMessageOf(rule.command);
  if (note != null) {
    return { when, icon: <Bell className="size-4" />, does: <>Show a notification: <q>{note}</q></> };
  }
  return {
    when,
    icon: <TerminalWindow className="size-4" />,
    does: <>Run <code className="rounded bg-muted/70 px-1 py-px font-mono text-[11.5px]">{rule.command}</code></>,
  };
}

/* ---------- the section ---------- */

export function HooksSection() {
  const [view, setView] = useState<HooksView | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  // Index being edited; -1 = adding a new one.
  const [editing, setEditing] = useState<number | null>(null);

  useEffect(() => {
    getHooks().then(setView).catch((e) => setError(e instanceof Error ? e.message : String(e)));
  }, []);

  async function save(rules: HookRule[]): Promise<boolean> {
    setSaving(true);
    setError(null);
    try {
      setView(await saveHooks(rules));
      return true;
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
      return false;
    } finally {
      setSaving(false);
    }
  }

  const os = view?.os ?? '';
  const canNotify = notifyCommand(os, 'x') != null;
  const presets = useMemo(() => {
    const list: { label: string; rule: HookRule | null }[] = [
      {
        label: 'Notify me when Mira needs approval',
        rule: canNotify
          ? { event: 'Notification', type: 'command', command: notifyCommand(os, 'Mira needs your approval') }
          : null,
      },
      {
        label: 'Notify me when Mira finishes',
        rule: canNotify ? { event: 'Stop', type: 'command', command: notifyCommand(os, 'Mira is done') } : null,
      },
      {
        label: 'Have the AI check the work before stopping',
        rule: {
          event: 'Stop',
          type: 'prompt',
          prompt: 'Did the assistant finish what the user asked, and run the tests if it changed code?',
        },
      },
    ];
    return list.filter((p) => p.rule != null) as { label: string; rule: HookRule }[];
  }, [os, canNotify]);

  if (!view) {
    return (
      <div className="flex items-center gap-2 px-1 text-[12.5px] text-muted-foreground">
        {error ? <span className="text-destructive">{error}</span> : <><CircleNotch className="size-3.5 animate-spin" /> Loading hooks…</>}
      </div>
    );
  }

  const rules = view.rules;
  return (
    <div className="flex flex-col gap-5">
      <div className="px-1">
        <div className="text-[18px] font-semibold tracking-tight text-foreground">Hooks</div>
        <div className="mt-1 text-[12.5px] text-muted-foreground/85">
          Make Mira do something at key moments: get a notification, have the AI double-check its
          work, or run your own script.
        </div>
      </div>

      {(error || view.problems.length > 0) && (
        <div className="flex flex-col gap-2">
          {error && <Banner tone="error">{error}</Banner>}
          {view.problems.map((p) => <Banner key={p} tone="warn">{p}</Banner>)}
        </div>
      )}

      <div className="overflow-hidden rounded-2xl border border-border/50 bg-mira-elev1/60">
        <div className="flex items-center justify-between gap-3 px-5 py-4">
          <div>
            <div className="text-[14px] font-semibold text-foreground">Your hooks</div>
            <div className="mt-0.5 text-[12.5px] text-muted-foreground">
              {rules.length === 0 ? 'None yet.' : `${rules.length} ${rules.length === 1 ? 'hook' : 'hooks'}, on for every chat.`}
            </div>
          </div>
          {editing == null && (
            <Button className="h-8 gap-1.5 px-3 text-[12px]" onClick={() => setEditing(-1)} disabled={saving}>
              <Plus className="size-3.5" /> Add a hook
            </Button>
          )}
        </div>

        {editing === -1 && (
          <div className="border-t border-border/40 px-5 py-4">
            <Editor
              initial={freshDraft()}
              os={os}
              saving={saving}
              onCancel={() => setEditing(null)}
              onSave={async (rule) => { if (await save([...rules, rule])) setEditing(null); }}
            />
          </div>
        )}

        {rules.length === 0 && editing == null && (
          <div className="border-t border-border/40 px-5 py-5">
            <div className="text-[12.5px] text-muted-foreground">Start with one of these:</div>
            <div className="mt-2.5 flex flex-wrap gap-2">
              {presets.map((p) => (
                <button
                  key={p.label}
                  type="button"
                  disabled={saving}
                  onClick={() => void save([p.rule])}
                  className="inline-flex items-center gap-1.5 rounded-full border border-border/60 px-3 py-1.5 text-[12px] text-foreground transition-colors hover:bg-muted/40 disabled:opacity-60"
                >
                  <Lightning className="size-3.5 text-amber-500" /> {p.label}
                </button>
              ))}
            </div>
          </div>
        )}

        {rules.map((rule, i) =>
          editing === i ? (
            <div key={i} className="border-t border-border/40 px-5 py-4">
              <Editor
                initial={draftFrom(rule)}
                os={os}
                saving={saving}
                onCancel={() => setEditing(null)}
                onSave={async (next) => {
                  if (await save(rules.map((r, j) => (j === i ? next : r)))) setEditing(null);
                }}
              />
            </div>
          ) : (
            <RuleRow
              key={i}
              rule={rule}
              busy={saving}
              onEdit={editing == null ? () => setEditing(i) : undefined}
              onRemove={() => void save(rules.filter((_, j) => j !== i))}
            />
          ),
        )}
      </div>

      {view.plugin_rules.length > 0 && (
        <div className="overflow-hidden rounded-2xl border border-border/50 bg-mira-elev1/60">
          <div className="px-5 py-4">
            <div className="text-[14px] font-semibold text-foreground">From your plugins</div>
            <div className="mt-0.5 text-[12.5px] text-muted-foreground">
              These come with plugins you’ve turned on. Turn the plugin off to stop them.
            </div>
          </div>
          {view.plugin_rules.map((r, i) => (
            <RuleRow key={i} rule={r} source={r.plugin} />
          ))}
        </div>
      )}

      <p className="px-1 text-[11.5px] leading-relaxed text-muted-foreground/80">
        Saved in <code>~/.mira/mira.yaml</code> and applied right away. Hooks in a project’s own
        settings are ignored on purpose, so opening someone else’s repo can’t run commands on your
        computer. Commands get the details as JSON on their input, in the same format Claude Code
        uses, so hook scripts written for it work here too.
      </p>
    </div>
  );
}

function RuleRow({
  rule, source, busy, onEdit, onRemove,
}: {
  rule: HookRule;
  source?: string;
  busy?: boolean;
  onEdit?: () => void;
  onRemove?: () => void;
}) {
  const { when, does, icon } = describe(rule);
  return (
    <div className="flex items-start gap-3 border-t border-border/40 px-5 py-3">
      <div className="mt-0.5 flex size-8 shrink-0 items-center justify-center rounded-lg border border-border/50 bg-background/60 text-foreground/80">
        {icon}
      </div>
      <div className="min-w-0 flex-1">
        <div className="text-[13px] font-medium text-foreground">{when}</div>
        <div className="mt-0.5 break-words text-[12px] text-muted-foreground">
          {does}
          {source && <span className="ml-1.5 text-muted-foreground/70">· from {source}</span>}
        </div>
      </div>
      {(onEdit || onRemove) && (
        <div className="flex shrink-0 items-center gap-1">
          {onEdit && (
            <IconButton title="Edit" onClick={onEdit} disabled={busy}>
              <PencilSimple className="size-3.5" />
            </IconButton>
          )}
          {onRemove && (
            <IconButton title="Remove" onClick={onRemove} disabled={busy}>
              <Trash className="size-3.5" />
            </IconButton>
          )}
        </div>
      )}
    </div>
  );
}

function IconButton({
  title, onClick, disabled, children,
}: {
  title: string;
  onClick: () => void;
  disabled?: boolean;
  children: React.ReactNode;
}) {
  return (
    <button
      type="button"
      title={title}
      aria-label={title}
      onClick={onClick}
      disabled={disabled}
      className="flex size-8 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-muted/50 hover:text-foreground disabled:opacity-50"
    >
      {children}
    </button>
  );
}

function Editor({
  initial, os, saving, onSave, onCancel,
}: {
  initial: Draft;
  os: string;
  saving: boolean;
  onSave: (rule: HookRule) => void;
  onCancel: () => void;
}) {
  const [d, setD] = useState<Draft>(initial);
  const [problem, setProblem] = useState<string | null>(null);
  const m = momentFor(d.event) ?? MOMENTS[0];
  const canNotify = notifyCommand(os, 'x') != null;
  const actions = m.actions.filter((a) => a !== 'notify' || canNotify);
  const set = (patch: Partial<Draft>) => { setProblem(null); setD((x) => ({ ...x, ...patch })); };

  function pickMoment(event: string) {
    const next = momentFor(event)!;
    const allowed = next.actions.filter((a) => a !== 'notify' || canNotify);
    set({
      event,
      action: allowed.includes(d.action) ? d.action : allowed[0],
      message: next.notifyMessage ?? d.message,
    });
  }

  function submit() {
    const r = ruleFrom(d, os);
    if (typeof r === 'string') setProblem(r);
    else onSave(r);
  }

  return (
    <div className="flex flex-col gap-3.5">
      <Row label="When">
        <Select
          value={d.event}
          onChange={pickMoment}
          options={MOMENTS.map((x) => ({ value: x.event, label: x.label, hint: x.help }))}
        />
      </Row>

      {m.tools && (
        <Row label="For">
          <div className="flex flex-col gap-2">
            <Select
              value={d.toolChoice}
              onChange={(v) => set({ toolChoice: v })}
              options={[
                ...TOOLS.map((t) => ({ value: t.matcher, label: t.label })),
                { value: CUSTOM_TOOLS, label: 'Specific tools…', hint: 'Tool names, e.g. Bash|Write' },
              ]}
            />
            {d.toolChoice === CUSTOM_TOOLS && (
              <SectionInput
                value={d.customTools}
                onChange={(e) => set({ customTools: e.target.value })}
                placeholder="e.g. Bash|Write or mcp__github__.*"
                spellCheck={false}
                className="font-mono text-[12.5px]"
              />
            )}
          </div>
        </Row>
      )}

      <Row label="Do">
        <div className="flex flex-wrap gap-1.5">
          {actions.map((a) => (
            <button
              key={a}
              type="button"
              onClick={() => set({ action: a })}
              className={cn(
                'inline-flex items-center gap-1.5 rounded-lg border px-3 py-1.5 text-[12px] transition-colors',
                d.action === a
                  ? 'border-foreground/40 bg-muted text-foreground'
                  : 'border-border/60 text-muted-foreground hover:text-foreground',
              )}
            >
              {a === 'notify' ? <Bell className="size-3.5" /> : a === 'check' ? <Robot className="size-3.5" /> : <TerminalWindow className="size-3.5" />}
              {ACTION_LABEL[a]}
            </button>
          ))}
        </div>
      </Row>

      {d.action === 'notify' && (
        <Row label="Message">
          <SectionInput value={d.message} onChange={(e) => set({ message: e.target.value })} />
        </Row>
      )}

      {d.action === 'check' && (
        <Row label="Question" note={m.check?.onNo}>
          <textarea
            value={d.question}
            onChange={(e) => set({ question: e.target.value })}
            placeholder={m.check?.example}
            rows={2}
            className="w-full resize-y rounded-md border border-border/60 bg-transparent px-3 py-2 text-[13px] text-foreground outline-none placeholder:text-muted-foreground/70 hover:border-border focus-visible:border-border focus-visible:ring-1 focus-visible:ring-ring"
          />
        </Row>
      )}

      {d.action === 'command' && (
        <Row label="Command" note={m.commandNote ?? 'It gets the details as JSON on its input.'}>
          <SectionInput
            value={d.command}
            onChange={(e) => set({ command: e.target.value })}
            placeholder="e.g. ~/.mira/hooks/check.sh"
            spellCheck={false}
            className="font-mono text-[12.5px]"
          />
        </Row>
      )}

      {d.action !== 'notify' && (
        <Row label="Time limit">
          <div className="flex items-center gap-2">
            <SectionInput
              value={d.timeout}
              onChange={(e) => set({ timeout: e.target.value.replace(/[^0-9.]/g, '') })}
              placeholder={d.action === 'check' ? '30' : '60'}
              className="w-20"
            />
            <span className="text-[12px] text-muted-foreground">seconds, then it’s skipped</span>
          </div>
        </Row>
      )}

      {problem && <Banner tone="error">{problem}</Banner>}

      <div className="flex items-center justify-end gap-2">
        <Button variant="outline" className="h-8 px-3 text-[12px]" onClick={onCancel} disabled={saving}>Cancel</Button>
        <Button className="h-8 px-3 text-[12px]" onClick={submit} disabled={saving}>
          {saving && <CircleNotch className="size-3.5 animate-spin" />}
          Save hook
        </Button>
      </div>
    </div>
  );
}

function Row({ label, note, children }: { label: string; note?: string; children: React.ReactNode }) {
  return (
    <div className="grid gap-1.5 sm:grid-cols-[92px_1fr] sm:gap-3">
      <div className="pt-2 text-[12px] font-medium text-muted-foreground">{label}</div>
      <div className="min-w-0">
        {children}
        {note && <div className="mt-1.5 text-[11.5px] text-muted-foreground/85">{note}</div>}
      </div>
    </div>
  );
}

function Banner({ tone, children }: { tone: 'warn' | 'error'; children: React.ReactNode }) {
  return (
    <div
      className={cn(
        'flex items-start gap-2 rounded-lg px-3 py-2 text-[12px]',
        tone === 'warn' ? 'bg-amber-500/10 text-amber-700 dark:text-amber-400' : 'bg-destructive/10 text-destructive',
      )}
    >
      <Warning weight="fill" className="mt-px size-3.5 shrink-0" />
      <div className="min-w-0">{children}</div>
    </div>
  );
}
