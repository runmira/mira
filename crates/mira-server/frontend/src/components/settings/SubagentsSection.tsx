/**
 * Settings → Subagents: Mira's helpers, each with a name and a face.
 *
 * Subagents are the specialists Mira's own model hands work to mid-turn —
 * Scout explores, Iris reviews, Bolt writes code. Everything about them is
 * editable here: who they are (name, face), when Mira should use them, how
 * they work (instructions, model, tools, budget) and how careful they are
 * (worktree isolation, approvals, review). Saving writes the same agent
 * file a person would write by hand, and applies on the next delegation.
 */
import { useEffect, useMemo, useState } from 'react';
import { Check, Loader2, Plus, RotateCcw, Trash2, X } from 'lucide-react';
import { cn } from '@/lib/utils';
import {
  FACE_COLORS,
  FACE_EYES,
  FACE_SHAPES,
  SubagentFace,
  resolveFace,
  type FaceSpec,
} from '../SubagentFace';
import {
  createSubagent,
  deleteSubagent,
  personaName,
  refreshSubagents,
  saveSubagent,
  useSubagents,
  type Subagent,
  type SubagentEdit,
} from '../../lib/subagents';

type Draft = {
  isNew: boolean;
  name: string;
  display_name: string;
  description: string;
  instructions: string;
  model: string;
  tools: string[] | null;
  max_rounds: string;
  worktree: boolean;
  route_approvals_to_parent: boolean;
  review_required: boolean;
  enabled: boolean;
  face: Required<{ [K in keyof FaceSpec]: NonNullable<FaceSpec[K]> }>;
};

function draftOf(s: Subagent): Draft {
  return {
    isNew: false,
    name: s.name,
    display_name: s.display_name ?? '',
    description: s.description,
    instructions: s.system_prompt_addendum ?? '',
    model: s.model ?? '',
    tools: s.tools ?? null,
    max_rounds: s.max_rounds ? String(s.max_rounds) : '',
    worktree: s.worktree ?? false,
    route_approvals_to_parent: s.route_approvals_to_parent ?? false,
    review_required: s.review_required ?? false,
    enabled: s.enabled ?? true,
    face: resolveFace(s.name, s.face),
  };
}

function blankDraft(): Draft {
  return {
    isNew: true,
    name: '',
    display_name: '',
    description: '',
    instructions: '',
    model: '',
    tools: ['read_file', 'grep', 'glob'],
    max_rounds: '',
    worktree: false,
    route_approvals_to_parent: true,
    review_required: false,
    enabled: true,
    face: resolveFace(String(Date.now()), null),
  };
}

const SOURCE_LABEL: Record<string, string> = {
  builtin: 'Built in',
  plugin: 'Plugin',
  user: 'Yours',
  project: 'This project',
};

export function SubagentsSection() {
  const view = useSubagents();
  const [draft, setDraft] = useState<Draft | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    void refreshSubagents().catch((e) => setError(String((e as Error).message)));
  }, []);

  const selected = draft && !draft.isNew ? view?.subagents.find((s) => s.name === draft.name) ?? null : null;

  async function save() {
    if (!draft) return;
    setSaving(true);
    setError(null);
    const edit: SubagentEdit = {
      display_name: draft.display_name,
      description: draft.description,
      instructions: draft.instructions,
      model: draft.model,
      tools: draft.tools ?? [],
      max_rounds: draft.max_rounds ? Number(draft.max_rounds) : 0,
      worktree: draft.worktree,
      route_approvals_to_parent: draft.route_approvals_to_parent,
      review_required: draft.review_required,
      enabled: draft.enabled,
      face: draft.face,
    };
    try {
      if (draft.isNew) {
        const id = draft.name.trim().toLowerCase();
        await createSubagent({ ...edit, name: id });
        setDraft((d) => (d ? { ...d, isNew: false, name: id } : d));
      } else {
        await saveSubagent(draft.name, edit);
      }
    } catch (e) {
      setError(String((e as Error).message));
    } finally {
      setSaving(false);
    }
  }

  async function remove() {
    if (!selected) return;
    setError(null);
    try {
      await deleteSubagent(selected.name);
      const after = (await refreshSubagents()).subagents.find((s) => s.name === selected.name);
      setDraft(after ? draftOf(after) : null);
    } catch (e) {
      setError(String((e as Error).message));
    }
  }

  return (
    <div className="space-y-5">
      <div className="text-[12.5px] leading-relaxed text-muted-foreground">
        Subagents are Mira&apos;s helpers. When a task fits one, Mira hands it over — and you&apos;ll see
        who&apos;s on it in the chat. They work for Mira&apos;s own models; when an external agent like
        Claude Code drives a chat, it brings its own.
      </div>

      {error && (
        <div className="rounded-lg border border-destructive/40 bg-destructive/10 px-3 py-2 text-[12px] text-destructive">
          {error}
        </div>
      )}

      {!view ? (
        <div className="flex items-center gap-2 text-[12.5px] text-muted-foreground">
          <Loader2 className="size-3.5 animate-spin" /> Loading subagents…
        </div>
      ) : (
        <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-3">
          {view.subagents.map((s) => {
            const active = draft?.name === s.name && !draft.isNew;
            const off = s.enabled === false;
            return (
              <button
                key={s.name}
                type="button"
                onClick={() => setDraft(draftOf(s))}
                className={cn(
                  'group relative flex items-start gap-3 rounded-2xl border p-3.5 text-left transition-all',
                  active
                    ? 'border-mira-blue/50 bg-mira-blue/[0.06]'
                    : 'border-border/60 bg-fg/[0.02] hover:-translate-y-0.5 hover:border-border hover:bg-fg/[0.04]',
                  off && 'opacity-55',
                )}
              >
                <SubagentFace id={s.name} face={off ? { ...resolveFace(s.name, s.face), eyes: 'sleepy' } : s.face} size={44} animate={!off} />
                <span className="min-w-0 flex-1">
                  <span className="flex items-center gap-1.5">
                    <span className="truncate text-[14px] font-semibold text-foreground">{personaName(s, s.name)}</span>
                    <span className="truncate font-mono text-[10.5px] text-muted-foreground/60">{s.name}</span>
                  </span>
                  <span className="mt-1 line-clamp-2 text-[11.5px] leading-snug text-muted-foreground">
                    {s.description || 'No description yet.'}
                  </span>
                  <span className="mt-2 flex flex-wrap gap-1">
                    <Badge>{SOURCE_LABEL[s.source ?? ''] ?? 'Custom'}</Badge>
                    {s.customized && <Badge tone="blue">Customized</Badge>}
                    {off && <Badge>Off</Badge>}
                  </span>
                </span>
              </button>
            );
          })}
          <button
            type="button"
            onClick={() => setDraft(blankDraft())}
            className="flex min-h-[7.5rem] flex-col items-center justify-center gap-1.5 rounded-2xl border border-dashed border-border/70 text-[12.5px] text-muted-foreground transition-colors hover:border-border hover:text-foreground"
          >
            <Plus className="size-4" />
            New subagent
          </button>
        </div>
      )}

      {draft && view && (
        <Editor
          draft={draft}
          setDraft={setDraft}
          tools={view.tools}
          selected={selected}
          saving={saving}
          onSave={() => void save()}
          onRemove={() => void remove()}
          onClose={() => setDraft(null)}
        />
      )}
    </div>
  );
}

function Editor({
  draft, setDraft, tools, selected, saving, onSave, onRemove, onClose,
}: {
  draft: Draft;
  setDraft: (f: (d: Draft | null) => Draft | null) => void;
  tools: string[];
  selected: Subagent | null;
  saving: boolean;
  onSave: () => void;
  onRemove: () => void;
  onClose: () => void;
}) {
  const set = <K extends keyof Draft>(k: K, v: Draft[K]) => setDraft((d) => (d ? { ...d, [k]: v } : d));
  const setFace = (patch: Partial<Draft['face']>) => setDraft((d) => (d ? { ...d, face: { ...d.face, ...patch } } : d));
  const allTools = draft.tools === null;
  const toolSet = useMemo(() => new Set(draft.tools ?? []), [draft.tools]);
  const display = draft.display_name || (draft.isNew ? 'New subagent' : personaName(selected, draft.name));

  return (
    <div className="overflow-hidden rounded-2xl border border-border/60 bg-background/40">
      {/* Who they are */}
      <div className="flex items-start gap-5 border-b border-border/50 bg-gradient-to-b from-fg/[0.03] to-transparent px-5 py-5">
        <div className="grid size-24 shrink-0 place-items-center rounded-3xl bg-fg/[0.03]">
          <SubagentFace id={draft.name || 'new'} face={draft.face} size={72} state="idle" />
        </div>
        <div className="min-w-0 flex-1 space-y-3">
          <div className="flex items-center gap-2">
            <span className="truncate text-[17px] font-semibold text-foreground">{display}</span>
            <button type="button" onClick={onClose} className="ml-auto rounded-md p-1 text-muted-foreground hover:bg-fg/[0.06] hover:text-foreground" aria-label="Close">
              <X className="size-4" />
            </button>
          </div>
          <div className="grid gap-3 sm:grid-cols-2">
            <Field label="Name">
              <input className={inputCls} value={draft.display_name} placeholder="Scout" onChange={(e) => set('display_name', e.target.value)} />
            </Field>
            <Field label="Id" hint={draft.isNew ? 'How Mira refers to it. Lowercase, no spaces.' : 'Fixed once created.'}>
              <input
                className={cn(inputCls, 'font-mono text-[12px]')}
                value={draft.name}
                disabled={!draft.isNew}
                placeholder="scout"
                onChange={(e) => set('name', e.target.value.toLowerCase().replace(/[^a-z0-9_-]/g, ''))}
              />
            </Field>
          </div>
        </div>
      </div>

      <div className="space-y-5 px-5 py-5">
        {/* Face */}
        <Group title="Face">
          <div className="flex flex-wrap gap-1.5">
            {FACE_COLORS.map((c) => (
              <button
                key={c}
                type="button"
                aria-label={`Color ${c}`}
                onClick={() => setFace({ color: c })}
                className={cn('size-6 rounded-full ring-offset-2 ring-offset-background transition-transform hover:scale-110', draft.face.color === c && 'ring-2 ring-foreground/70')}
                style={{ background: c }}
              />
            ))}
          </div>
          <div className="mt-3 flex flex-wrap gap-1.5">
            {FACE_SHAPES.map((shape) => (
              <Choice key={shape} active={draft.face.shape === shape} onClick={() => setFace({ shape })} label={shape}>
                <SubagentFace id="x" face={{ ...draft.face, shape }} size={30} animate={false} />
              </Choice>
            ))}
          </div>
          <div className="mt-2 flex flex-wrap items-center gap-1.5">
            {FACE_EYES.map((eyes) => (
              <Choice key={eyes} active={draft.face.eyes === eyes} onClick={() => setFace({ eyes })} label={eyes}>
                <SubagentFace id="x" face={{ ...draft.face, eyes }} size={30} animate={false} />
              </Choice>
            ))}
            <label className="ml-2 inline-flex cursor-pointer items-center gap-1.5 text-[12px] text-muted-foreground">
              <input type="checkbox" checked={draft.face.cheeks} onChange={(e) => setFace({ cheeks: e.target.checked })} className="accent-pink-400" />
              Rosy cheeks
            </label>
          </div>
        </Group>

        {/* What they're for */}
        <Group title="When Mira should use it">
          <textarea
            className={cn(inputCls, 'h-16 resize-y')}
            value={draft.description}
            placeholder="Explores the codebase read-only and reports where things live."
            onChange={(e) => set('description', e.target.value)}
          />
          <p className="mt-1 text-[11px] text-muted-foreground/70">Mira picks subagents by this, so say what it&apos;s good at.</p>
        </Group>

        <Group title="Instructions">
          <textarea
            className={cn(inputCls, 'h-40 resize-y font-mono text-[11.5px] leading-relaxed')}
            value={draft.instructions}
            placeholder="You are Scout. You explore before anyone edits…"
            onChange={(e) => set('instructions', e.target.value)}
          />
        </Group>

        <div className="grid gap-4 sm:grid-cols-2">
          <Field label="Model" hint="Leave empty to use the chat's model.">
            <input className={cn(inputCls, 'font-mono text-[12px]')} value={draft.model} placeholder="(same as the chat)" onChange={(e) => set('model', e.target.value)} />
          </Field>
          <Field label="Round budget" hint="How many steps it may take.">
            <input className={inputCls} type="number" min={1} value={draft.max_rounds} placeholder="default" onChange={(e) => set('max_rounds', e.target.value)} />
          </Field>
        </div>

        <Group title="Tools">
          <label className="mb-2 inline-flex cursor-pointer items-center gap-2 text-[12.5px] text-foreground/85">
            <Toggle on={allTools} onChange={(v) => set('tools', v ? null : ['read_file', 'grep', 'glob'])} />
            Everything Mira can use
          </label>
          {!allTools && (
            <div className="flex flex-wrap gap-1.5">
              {tools.map((t) => {
                const on = toolSet.has(t);
                return (
                  <button
                    key={t}
                    type="button"
                    onClick={() => {
                      const next = new Set(toolSet);
                      if (on) next.delete(t);
                      else next.add(t);
                      set('tools', [...next]);
                    }}
                    className={cn(
                      'rounded-full border px-2.5 py-1 font-mono text-[11px] transition-colors',
                      on ? 'border-mira-blue/50 bg-mira-blue/15 text-foreground' : 'border-border/60 text-muted-foreground hover:text-foreground',
                    )}
                  >
                    {on && <Check className="mr-1 inline size-3" />}
                    {t}
                  </button>
                );
              })}
            </div>
          )}
        </Group>

        <Group title="Behaviour">
          <div className="space-y-2.5">
            <Row label="On" hint="Off keeps it here but out of Mira's reach.">
              <Toggle on={draft.enabled} onChange={(v) => set('enabled', v)} />
            </Row>
            <Row label="Work in its own copy" hint="Edits happen in an isolated git worktree and are merged back when it finishes.">
              <Toggle on={draft.worktree} onChange={(v) => set('worktree', v)} />
            </Row>
            <Row label="Ask me before risky steps" hint="Its approval prompts come to you, like your own.">
              <Toggle on={draft.route_approvals_to_parent} onChange={(v) => set('route_approvals_to_parent', v)} />
            </Row>
            <Row label="Let me review its report" hint="You approve its summary before Mira acts on it.">
              <Toggle on={draft.review_required} onChange={(v) => set('review_required', v)} />
            </Row>
          </div>
        </Group>
      </div>

      <div className="flex items-center gap-2 border-t border-border/50 px-5 py-3">
        {selected && selected.customized && (
          <button type="button" onClick={onRemove} className={secondaryBtn}>
            <RotateCcw className="size-3.5" /> Reset to default
          </button>
        )}
        {selected && !selected.has_builtin && (selected.source === 'user' || selected.source === 'project') && (
          <button type="button" onClick={onRemove} className={cn(secondaryBtn, 'text-destructive/90 hover:text-destructive')}>
            <Trash2 className="size-3.5" /> Delete
          </button>
        )}
        <span className="flex-1" />
        <button type="button" onClick={onClose} className={secondaryBtn}>Cancel</button>
        <button
          type="button"
          onClick={onSave}
          disabled={saving || (draft.isNew && (!draft.name || !draft.description.trim()))}
          className="inline-flex items-center gap-1.5 rounded-lg bg-mira-blue/90 px-3.5 py-1.5 text-[12.5px] font-medium text-white transition-colors hover:bg-mira-blue disabled:opacity-40"
        >
          {saving && <Loader2 className="size-3.5 animate-spin" />}
          {draft.isNew ? 'Create' : 'Save'}
        </button>
      </div>
    </div>
  );
}

const inputCls =
  'w-full rounded-lg border border-border/80 bg-background/60 px-3 py-2 text-[12.5px] outline-none transition-colors ' +
  'placeholder:text-muted-foreground/40 focus:border-mira-blue/50 focus:bg-background disabled:opacity-60';
const secondaryBtn =
  'inline-flex items-center gap-1.5 rounded-lg border border-border/80 px-3 py-1.5 text-[12px] text-muted-foreground transition-colors hover:bg-fg/[0.04] hover:text-foreground';

function Group({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <section>
      <div className="mb-2 text-[11px] font-semibold uppercase tracking-wider text-muted-foreground/60">{title}</div>
      {children}
    </section>
  );
}

function Field({ label, hint, children }: { label: string; hint?: string; children: React.ReactNode }) {
  return (
    <label className="block">
      <div className="mb-1 text-[12px] font-medium text-foreground/85">{label}</div>
      {children}
      {hint && <div className="mt-1 text-[11px] text-muted-foreground/65">{hint}</div>}
    </label>
  );
}

function Row({ label, hint, children }: { label: string; hint: string; children: React.ReactNode }) {
  return (
    <div className="flex items-start justify-between gap-4">
      <div>
        <div className="text-[12.5px] text-foreground/90">{label}</div>
        <div className="text-[11px] text-muted-foreground/70">{hint}</div>
      </div>
      {children}
    </div>
  );
}

function Toggle({ on, onChange }: { on: boolean; onChange: (v: boolean) => void }) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={on}
      onClick={() => onChange(!on)}
      className={cn('relative h-5 w-9 shrink-0 rounded-full transition-colors', on ? 'bg-mira-blue' : 'bg-muted-foreground/25 hover:bg-muted-foreground/35')}
    >
      <span className={cn('absolute top-0.5 size-4 rounded-full bg-white shadow-sm transition-[left]', on ? 'left-[18px]' : 'left-0.5')} />
    </button>
  );
}

function Choice({ active, onClick, label, children }: { active: boolean; onClick: () => void; label: string; children: React.ReactNode }) {
  return (
    <button
      type="button"
      onClick={onClick}
      title={label}
      className={cn(
        'grid size-11 place-items-center rounded-xl border transition-colors',
        active ? 'border-mira-blue/60 bg-mira-blue/10' : 'border-border/50 hover:border-border hover:bg-fg/[0.04]',
      )}
    >
      {children}
    </button>
  );
}

function Badge({ children, tone }: { children: React.ReactNode; tone?: 'blue' }) {
  return (
    <span
      className={cn(
        'rounded-full px-1.5 py-px text-[10px] font-medium',
        tone === 'blue' ? 'bg-mira-blue/15 text-mira-blue' : 'bg-fg/[0.06] text-muted-foreground',
      )}
    >
      {children}
    </span>
  );
}
