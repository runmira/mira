import { Dialog, DialogContent } from '@/components/ui/dialog';
import { cn } from '@/lib/utils';
import {
  Book,
  BookOpen,
  Braces,
  Bug,
  ChevronRight,
  Code,
  Cog,
  Database,
  Eye,
  FileText,
  FolderOpen,
  GitBranch,
  GitCommit,
  GitMerge,
  GitPullRequest,
  Lightbulb,
  NotebookPen,
  Package,
  Paperclip,
  Rocket,
  RotateCw,
  ScanFace,
  Search,
  Shield,
  ShieldCheck,
  Sparkle,
  Target,
  Terminal,
  Wrench,
} from 'lucide-react';
import React, { useEffect, useMemo, useState } from 'react';
import { getSkill, listSkills, reloadSkills, type SkillDetail, type SkillView } from '../../api';
import { Markdown } from '../Markdown';
import { TSection } from './SettingsFields';
/* ---------- skills ---------- */

/**
 * Loaded-skill browser. Card grid, each skill rendered as a rounded
 * card with a colored icon badge + name + description + metadata chips.
 * Colors + icons come from the skill's frontmatter (`color:` and
 * `icon:` fields); unknowns fall back to stable name-hash-derived tints
 * so user-added skills still look distinct even without opinions.
 *
 * Fetches `/api/skills` on mount; a "Reload from disk" button re-reads
 * `~/.mira/skills/` + `<cwd>/.mira/skills/` (via `POST /api/skills/reload`)
 * so newly-added skill files appear without a restart.
 *
 * Groups by tier (Bundled → User → Project). Each tier gets its own
 * header row with a count. Bundled skills always render first — they're
 * the "official" roster the user can rely on.
 */
export function SkillsSection({ version = 0 }: { version?: number }) {
  const [skills, setSkills] = useState<SkillView[] | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [lastReload, setLastReload] = useState<number | null>(null);
  // Name of the skill whose detail drawer is open (null = closed). The
  // drawer fetches the full payload lazily on open so the list-view
  // fetch stays cheap.
  const [selected, setSelected] = useState<string | null>(null);

  useEffect(() => {
    setLoading(true);
    listSkills()
      .then((s) => {
        setSkills(s);
        setError(null);
        // Stamp reload time on any fetch trigger so the "reloaded Xs
        // ago" hint applies to auto-reloads (via the fs watcher) too.
        if (version > 0) setLastReload(Date.now());
      })
      .catch((e) => setError(String((e as Error).message)))
      .finally(() => setLoading(false));
  }, [version]);

  async function reload() {
    setLoading(true);
    setError(null);
    try {
      const fresh = await reloadSkills();
      setSkills(fresh);
      setLastReload(Date.now());
    } catch (e) {
      setError(String((e as Error).message));
    } finally {
      setLoading(false);
    }
  }

  const grouped = useMemo(() => {
    const out: Record<'bundled' | 'shared' | 'user' | 'project', SkillView[]> = {
      bundled: [],
      shared: [],
      user: [],
      project: [],
    };
    for (const s of skills ?? []) {
      out[s.tier].push(s);
    }
    (['bundled', 'shared', 'user', 'project'] as const).forEach((k) => {
      out[k].sort((a, b) => a.name.localeCompare(b.name));
    });
    return out;
  }, [skills]);

  const total = skills?.length ?? 0;

  return (
    <div className="flex flex-col gap-2.5">
      <TSection
        icon={<Sparkle className="size-3.5" />}
        title="Skills"
        description="Reusable instruction bundles the agent invokes to accomplish a specific task. Add your own to ~/.mira/skills/ (user-wide) or <cwd>/.mira/skills/ (per-repo)."
        action={
          <>
            <span className="text-[11px] tabular-nums text-muted-foreground">
              {loading ? 'Loading…' : `${total} skill${total === 1 ? '' : 's'}`}
              {lastReload && !loading ? ` · reloaded ${timeAgoSecs(lastReload)}` : ''}
            </span>
            <button
              type="button"
              onClick={reload}
              disabled={loading}
              title="Re-read skill files from disk"
              aria-label="Reload skills from disk"
              className="rounded-md p-1.5 text-muted-foreground transition-colors hover:bg-accent hover:text-foreground disabled:opacity-40"
            >
              <RotateCw className={cn('size-3.5', loading && 'animate-spin')} strokeWidth={2.5} />
            </button>
          </>
        }
      >
        {error && (
          <div className="border-b border-border/50 px-4 py-2.5 text-[12px] text-destructive">
            {error}
          </div>
        )}

        {!error && !loading && skills != null && total === 0 && <EmptySkillsState />}

        {(['bundled', 'shared', 'user', 'project'] as const).map((tier) => {
          const entries = grouped[tier];
          if (entries.length === 0) return null;
          return (
            <div key={tier}>
              <div className="flex items-center gap-2 border-b border-border/50 px-4 pb-2 pt-3">
                <span className={cn('size-1.5 rounded-full', tierDotClass(tier))} />
                <span className="text-[10.5px] font-semibold uppercase tracking-wider text-muted-foreground">
                  {tierLabel(tier)}
                </span>
                <span className="text-[11px] tabular-nums text-muted-foreground/70">
                  {entries.length}
                </span>
              </div>
              <div className="[&>*+*]:border-t [&>*+*]:border-border/50">
                {entries.map((s) => (
                  <SkillRow
                    key={`${tier}-${s.name}`}
                    skill={s}
                    onOpen={() => setSelected(s.name)}
                  />
                ))}
              </div>
            </div>
          );
        })}
      </TSection>

      <SkillDetailDialog name={selected} onClose={() => setSelected(null)} />
    </div>
  );
}

/** One skill as a row: icon square, name + description, chevron.
 *  Colour lives only on the icon square — the visual anchor that lets the
 *  eye pick a skill out without overwhelming the list. */
export function SkillRow({ skill, onOpen }: { skill: SkillView; onOpen: () => void }) {
  const iconKey = skill.icon ?? defaultIconKey(skill);
  const Icon = iconFor(iconKey);
  const palette = paletteFor(skill.color, skill.name);

  return (
    <button
      type="button"
      onClick={onOpen}
      className="group flex w-full items-center gap-3 px-4 py-2.5 text-left transition-colors hover:bg-accent/40 focus:outline-none"
    >
      <div
        className={cn(
          'inline-flex size-8 shrink-0 items-center justify-center rounded-lg',
          palette.iconBg,
          palette.iconText,
        )}
      >
        <Icon className="size-4" />
      </div>

      <div className="min-w-0 flex-1">
        <div className="flex items-baseline gap-2">
          <span className="truncate font-mono text-[13px] font-semibold text-foreground">
            /{skill.name}
          </span>
          {skill.category && (
            <span
              className={cn(
                'shrink-0 rounded-sm px-1.5 py-px text-[10px] font-medium uppercase tracking-wider',
                palette.chipBg,
                palette.chipText,
              )}
            >
              {skill.category}
            </span>
          )}
          {skill.has_attachments && (
            <Paperclip className="size-3 shrink-0 text-muted-foreground/60" />
          )}
        </div>
        <p className="mt-px truncate text-[12px] text-muted-foreground/85">{skill.description}</p>
      </div>

      <ChevronRight className="size-3.5 shrink-0 text-muted-foreground/50 transition-all group-hover:translate-x-px group-hover:text-foreground" />
    </button>
  );
}

/** Detail drawer for one skill — opens on card click, fetches the full
 *  SKILL.md body + attached files from `/api/skills/:name`, and renders
 *  the body as markdown. `name === null` closes the dialog; a name-swap
 *  transitions cleanly by keying the loader on `name`. */
export function SkillDetailDialog({ name, onClose }: { name: string | null; onClose: () => void }) {
  const [detail, setDetail] = useState<SkillDetail | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (name === null) {
      setDetail(null);
      setError(null);
      return;
    }
    let cancelled = false;
    setLoading(true);
    setError(null);
    getSkill(name)
      .then((d) => {
        if (cancelled) return;
        if (d) setDetail(d);
        else setError(`no skill named ${name}`);
      })
      .catch((e) => {
        if (!cancelled) setError(String((e as Error).message));
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [name]);

  const open = name !== null;

  return (
    <Dialog open={open} onOpenChange={(o) => !o && onClose()}>
      <DialogContent className="max-w-2xl p-0 gap-0 overflow-hidden">
        <div className="flex h-[560px] max-h-[80vh] flex-col">
          {/* Header: icon badge + name + tier chip. Mirrors the card
              styling so it feels like the card expanded rather than a
              separate view. */}
          {detail ? (
            <SkillDetailHeader detail={detail} />
          ) : (
            <div className="flex items-center gap-3 border-b border-border/60 px-5 py-4">
              <div className="size-9 shrink-0 rounded-lg bg-secondary/60" />
              <div className="flex min-w-0 flex-1 flex-col gap-1">
                <div className="h-4 w-32 rounded bg-secondary/60" />
                <div className="h-3 w-52 rounded bg-secondary/40" />
              </div>
            </div>
          )}

          <div className="flex-1 overflow-y-auto px-5 py-4">
            {loading && !detail && (
              <div className="text-[12.5px] text-muted-foreground">loading…</div>
            )}
            {error && (
              <div className="rounded-md border border-destructive/40 bg-destructive/[0.08] px-3 py-2 text-[12px] text-destructive">
                {error}
              </div>
            )}
            {detail && (
              <div className="flex flex-col gap-4">
                {/* Body — the SKILL.md markdown. The invoked skill sees
                    this exact text as a system-reminder, so showing it
                    verbatim doubles as documentation for what the model
                    is about to do. */}
                <div className="rounded-lg border border-border/50 bg-background/40 px-4 py-3">
                  {detail.body.trim().length === 0 ? (
                    <div className="text-[12px] italic text-muted-foreground">
                      (this skill has no body)
                    </div>
                  ) : (
                    <Markdown text={detail.body} />
                  )}
                </div>

                {detail.attachments.length > 0 && (
                  <div className="flex flex-col gap-1.5">
                    <div className="text-[10.5px] font-semibold uppercase tracking-wider text-muted-foreground">
                      Attached files ({detail.attachments.length})
                    </div>
                    <ul className="flex flex-col gap-1 rounded-md border border-border/50 bg-secondary/20 px-3 py-2">
                      {detail.attachments.map((f) => (
                        <li
                          key={f}
                          className="flex items-center gap-2 font-mono text-[11.5px] text-foreground/85"
                        >
                          <Paperclip className="size-3 shrink-0 text-muted-foreground" />
                          {f}
                        </li>
                      ))}
                    </ul>
                  </div>
                )}

                {detail.source && (
                  <div className="flex flex-col gap-1.5">
                    <div className="text-[10.5px] font-semibold uppercase tracking-wider text-muted-foreground">
                      Source
                    </div>
                    <div className="rounded-md border border-border/50 bg-secondary/20 px-3 py-2 font-mono text-[11.5px] text-foreground/80 break-all">
                      {detail.source}
                    </div>
                  </div>
                )}

                {!detail.source && (
                  <div className="text-[11.5px] text-muted-foreground">
                    Bundled skill — ships with the binary. Drop a file at{' '}
                    <code className="font-mono text-foreground/85">
                      ~/.mira/skills/{detail.name}/SKILL.md
                    </code>{' '}
                    to override.
                  </div>
                )}
              </div>
            )}
          </div>
        </div>
      </DialogContent>
    </Dialog>
  );
}

export function SkillDetailHeader({ detail }: { detail: SkillDetail }) {
  const iconKey = detail.icon ?? defaultIconKey(detail);
  const Icon = iconFor(iconKey);
  const palette = paletteFor(detail.color, detail.name);
  return (
    <div className="flex items-start gap-3 border-b border-border/60 px-5 py-4">
      <div
        className={cn(
          'inline-flex size-9 shrink-0 items-center justify-center rounded-lg',
          palette.iconBg,
          palette.iconText,
        )}
      >
        <Icon className="size-5" />
      </div>
      <div className="flex min-w-0 flex-1 flex-col gap-1">
        <div className="flex items-center gap-2">
          <span className="truncate font-mono text-[14px] font-semibold text-foreground">
            /{detail.name}
          </span>
          <span className="shrink-0 rounded-full border border-border/60 bg-secondary px-2 py-0.5 text-[9.5px] font-semibold uppercase tracking-wider text-muted-foreground">
            {tierLabel(detail.tier)}
          </span>
          {detail.category && (
            <span
              className={cn(
                'shrink-0 rounded-sm px-1.5 py-0.5 text-[10px] font-medium uppercase tracking-wider',
                palette.chipBg,
                palette.chipText,
              )}
            >
              {detail.category}
            </span>
          )}
        </div>
        <p className="text-[12.5px] leading-snug text-foreground/80">{detail.description}</p>
      </div>
    </div>
  );
}

export function EmptySkillsState() {
  return (
    <div className="flex flex-col items-center gap-3 rounded-xl border border-dashed border-border/50 bg-secondary/20 px-4 py-8 text-center">
      <div className="inline-flex size-10 items-center justify-center rounded-full bg-mira-blue/10 text-mira-blue">
        <Sparkle className="size-5" />
      </div>
      <div>
        <div className="text-[13px] font-semibold text-foreground">No skills loaded</div>
        <div className="mt-1 text-[12px] text-muted-foreground">
          Drop a <code className="font-mono text-foreground/85">SKILL.md</code> into{' '}
          <code className="font-mono text-foreground/85">~/.mira/skills/&lt;name&gt;/</code> and hit
          Reload — or ask mira via{' '}
          <code className="font-mono text-foreground/85">/skill-creator</code>.
        </div>
      </div>
    </div>
  );
}

/* ---------- skill icon + color mapping ---------- */

/** Kebab-case icon name → Lucide component. Curated so a user's
 *  frontmatter `icon: shield-check` picks up a matching component
 *  without every lucide icon getting bundled. Unknown names fall
 *  through to Sparkle. */
export function iconFor(name: string): React.ComponentType<{ className?: string }> {
  switch (name) {
    case 'shield-check':
      return ShieldCheck;
    case 'shield':
      return Shield;
    case 'magnifying-glass':
      return Search;
    case 'bug':
      return Bug;
    case 'folder-open':
      return FolderOpen;
    case 'file-text':
    case 'file':
      return FileText;
    case 'git-branch':
      return GitBranch;
    case 'git-commit':
      return GitCommit;
    case 'git-merge':
      return GitMerge;
    case 'git-pull-request':
      return GitPullRequest;
    case 'lightbulb':
      return Lightbulb;
    case 'target':
      return Target;
    case 'gear':
    case 'settings':
      return Cog;
    case 'wrench':
      return Wrench;
    case 'rocket':
      return Rocket;
    case 'package':
      return Package;
    case 'database':
      return Database;
    case 'terminal':
      return Terminal;
    case 'code':
      return Code;
    case 'brackets-curly':
      return Braces;
    case 'book':
      return Book;
    case 'book-open':
      return BookOpen;
    case 'eye':
      return Eye;
    case 'scan':
      return ScanFace;
    case 'note-pencil':
      return NotebookPen;
    case 'sparkle':
    default:
      return Sparkle;
  }
}

/** Pick a default icon key for a skill that didn't specify one. Bundled
 *  skills always ship an explicit `icon:`; this only fires for user-
 *  added ones. We nudge by category — a `git`-category skill without an
 *  icon still reads sensibly as a git-branch. Accepts either the list-
 *  view or the detail payload — both carry `category`. */
export function defaultIconKey(s: { category?: string }): string {
  const cat = (s.category ?? '').toLowerCase();
  switch (cat) {
    case 'git':
      return 'git-branch';
    case 'review':
      return 'magnifying-glass';
    case 'qa':
      return 'shield-check';
    case 'debug':
      return 'bug';
    case 'meta':
      return 'sparkle';
    case 'onboarding':
      return 'folder-open';
    case 'deploy':
      return 'rocket';
    default:
      return 'sparkle';
  }
}

export type Palette = {
  iconBg: string;
  iconText: string;
  chipBg: string;
  chipText: string;
};

/** Color name → tailwind classes.
 *
 *  Each hue's classes are HARD-CODED (not template-interpolated)
 *  because Tailwind's JIT scanner only ships classes it can see as
 *  literal strings. `` `bg-${color}-500/15` `` compiles fine but the
 *  class never gets emitted → the card renders unstyled. Every hue
 *  below lists all six slots explicitly.
 *
 *  Unknown/missing color names hash by skill name to a stable wheel
 *  pick, so a user-added skill with `color: whatever` still gets a
 *  coherent card rather than a bland fallback. */
export function paletteFor(name: string | undefined, fallbackSeed: string): Palette {
  const key = name?.toLowerCase() ?? hashPick(fallbackSeed);
  const normalized =
    key === 'green'
      ? 'emerald'
      : key === 'sky'
        ? 'blue'
        : key === 'purple'
          ? 'violet'
          : key === 'rose'
            ? 'pink'
            : key === 'yellow'
              ? 'amber'
              : key;
  switch (normalized) {
    case 'emerald':
      return {
        iconBg: 'bg-emerald-500/15',
        iconText: 'text-emerald-300',
        chipBg: 'bg-emerald-500/15',
        chipText: 'text-emerald-300',
      };
    case 'blue':
      return {
        iconBg: 'bg-blue-500/15',
        iconText: 'text-blue-300',
        chipBg: 'bg-blue-500/15',
        chipText: 'text-blue-300',
      };
    case 'indigo':
      return {
        iconBg: 'bg-indigo-500/15',
        iconText: 'text-indigo-300',
        chipBg: 'bg-indigo-500/15',
        chipText: 'text-indigo-300',
      };
    case 'violet':
      return {
        iconBg: 'bg-violet-500/15',
        iconText: 'text-violet-300',
        chipBg: 'bg-violet-500/15',
        chipText: 'text-violet-300',
      };
    case 'pink':
      return {
        iconBg: 'bg-pink-500/15',
        iconText: 'text-pink-300',
        chipBg: 'bg-pink-500/15',
        chipText: 'text-pink-300',
      };
    case 'amber':
      return {
        iconBg: 'bg-amber-500/15',
        iconText: 'text-amber-300',
        chipBg: 'bg-amber-500/15',
        chipText: 'text-amber-300',
      };
    case 'orange':
      return {
        iconBg: 'bg-orange-500/15',
        iconText: 'text-orange-300',
        chipBg: 'bg-orange-500/15',
        chipText: 'text-orange-300',
      };
    case 'red':
      return {
        iconBg: 'bg-red-500/15',
        iconText: 'text-red-300',
        chipBg: 'bg-red-500/15',
        chipText: 'text-red-300',
      };
    case 'teal':
      return {
        iconBg: 'bg-teal-500/15',
        iconText: 'text-teal-300',
        chipBg: 'bg-teal-500/15',
        chipText: 'text-teal-300',
      };
    case 'cyan':
      return {
        iconBg: 'bg-cyan-500/15',
        iconText: 'text-cyan-300',
        chipBg: 'bg-cyan-500/15',
        chipText: 'text-cyan-300',
      };
    default:
      return {
        iconBg: 'bg-blue-500/15',
        iconText: 'text-blue-300',
        chipBg: 'bg-blue-500/15',
        chipText: 'text-blue-300',
      };
  }
}

export const HASH_WHEEL = [
  'emerald',
  'blue',
  'indigo',
  'violet',
  'pink',
  'amber',
  'orange',
  'red',
  'teal',
  'cyan',
];

export function hashPick(seed: string): string {
  let h = 5381;
  for (let i = 0; i < seed.length; i++) {
    h = (h * 33) ^ seed.charCodeAt(i);
  }
  return HASH_WHEEL[Math.abs(h) % HASH_WHEEL.length];
}

export type Tier = 'bundled' | 'shared' | 'user' | 'project';

export function tierLabel(t: Tier): string {
  switch (t) {
    case 'bundled':
      return 'Bundled';
    case 'shared':
      return 'Shared (~/.agents/skills)';
    case 'user':
      return 'User (~/.mira/skills)';
    case 'project':
      return 'Project (.mira/skills)';
  }
}

export function tierDotClass(t: Tier): string {
  switch (t) {
    case 'bundled':
      return 'bg-mira-blue';
    case 'shared':
      return 'bg-amber-400';
    case 'user':
      return 'bg-emerald-400';
    case 'project':
      return 'bg-purple-400';
  }
}

export function timeAgoSecs(ts: number): string {
  const dt = Math.max(0, Math.floor((Date.now() - ts) / 1000));
  if (dt < 5) return 'just now';
  if (dt < 60) return `${dt}s ago`;
  if (dt < 3600) return `${Math.floor(dt / 60)}m ago`;
  return `${Math.floor(dt / 3600)}h ago`;
}
