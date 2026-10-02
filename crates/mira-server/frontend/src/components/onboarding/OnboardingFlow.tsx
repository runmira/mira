import { useMemo, useState } from 'react';
import { AccountTypeStep } from './AccountTypeStep';
import { YourNameStep } from './YourNameStep';
import { InviteTeamStep } from './InviteTeamStep';
import { RunStep } from './RunStep';
import { DoneStep } from './DoneStep';
import { ImportChats } from '../ImportChats';
import { OnboardingShell, type RailStep } from './OnboardingShell';
import { StepHeader } from './ui';

export type AccountType = 'personal' | 'team';

export type Profile = {
  id: string;
  account_type: AccountType | null;
  full_name: string | null;
  onboarding_completed_at: string | null;
};

type StepSlug = 'account-type' | 'your-name' | 'invite-team' | 'run' | 'import' | 'done';

/**
 * Client-side onboarding state machine. The step registry is a plain array
 * with an `appliesTo` predicate so team-only steps (invite-team) are
 * skipped for personal accounts without extra branching in the render.
 */
type Step = {
  slug: StepSlug;
  appliesTo: (t: AccountType | null) => boolean;
};

const STEPS: Step[] = [
  { slug: 'account-type', appliesTo: () => true },
  { slug: 'your-name', appliesTo: () => true },
  { slug: 'invite-team', appliesTo: (t) => t === 'team' },
  { slug: 'run', appliesTo: () => true },
  { slug: 'import', appliesTo: () => true },
  { slug: 'done', appliesTo: () => true },
];

/** What the step rail calls each step. */
const RAIL: Record<StepSlug, Omit<RailStep, 'slug'>> = {
  'account-type': { label: 'Workspace', hint: 'Just you, or a team' },
  'your-name': { label: 'Profile', hint: 'What to call you' },
  'invite-team': { label: 'Team', hint: 'Bring your people' },
  run: { label: 'Engines', hint: 'Agents and providers' },
  import: { label: 'Chats', hint: 'From Claude Code & Codex' },
  done: { label: 'Ready', hint: 'Into the app' },
};

/** Steps that need more room than a single column. */
const WIDE: ReadonlySet<StepSlug> = new Set(['run', 'import']);

function applicable(steps: Step[], type: AccountType | null): Step[] {
  return steps.filter((s) => s.appliesTo(type));
}

function firstIncomplete(profile: Profile | null): StepSlug {
  if (!profile?.account_type) return 'account-type';
  if (!profile.full_name || profile.full_name.trim().length === 0) return 'your-name';
  // Name saved but not finished: pick up at setup (invites are optional
  // and were offered already).
  return 'run';
}

export function OnboardingFlow({
  userId,
  initialProfile,
  userMetadata,
  onDone,
}: {
  userId: string;
  initialProfile: Profile | null;
  userMetadata: Record<string, unknown>;
  onDone: () => void;
}) {
  const [profile, setProfile] = useState<Profile | null>(initialProfile);
  const [current, setCurrent] = useState<StepSlug>(() => firstIncomplete(initialProfile));
  const [imported, setImported] = useState(0);

  const steps = useMemo(() => applicable(STEPS, profile?.account_type ?? null), [profile?.account_type]);
  const activeIndex = Math.max(0, steps.findIndex((s) => s.slug === current));

  /** Advance from a step that saves nothing to the profile. */
  function advance() {
    const list = applicable(STEPS, profile?.account_type ?? null);
    const idx = list.findIndex((s) => s.slug === current);
    const next = list[idx + 1];
    if (next) setCurrent(next.slug);
  }
  function back() {
    const list = applicable(STEPS, profile?.account_type ?? null);
    const idx = list.findIndex((s) => s.slug === current);
    if (idx > 0) setCurrent(list[idx - 1].slug);
  }

  function goNext(nextProfile: Profile) {
    setProfile(nextProfile);
    if (nextProfile.onboarding_completed_at) {
      onDone();
      return;
    }
    const list = applicable(STEPS, nextProfile.account_type ?? null);
    const idx = list.findIndex((s) => s.slug === current);
    const next = list[idx + 1];
    setCurrent(next ? next.slug : list[list.length - 1].slug);
  }

  const rail = steps.map((s) => ({ slug: s.slug, ...RAIL[s.slug] }));
  const firstName = (profile?.full_name ?? '').trim().split(/\s+/)[0] ?? '';

  return (
    <OnboardingShell steps={rail} active={activeIndex} wide={WIDE.has(current)}>
      {current === 'account-type' && (
        <AccountTypeStep userId={userId} initialProfile={profile} onSaved={(next) => goNext(next)} />
      )}
      {current === 'your-name' && (
        <YourNameStep
          userId={userId}
          initialProfile={profile}
          fallbackName={oauthName(userMetadata)}
          onSaved={(next) => goNext(next)}
          onBack={back}
        />
      )}
      {current === 'invite-team' && <InviteTeamStep userId={userId} onSaved={(next) => goNext(next)} onBack={back} />}
      {current === 'run' && <RunStep onNext={advance} onBack={back} />}
      {current === 'import' && (
        <section className="flex flex-col gap-7">
          <StepHeader eyebrow="Bring your chats" title="Pick up where you left off">
            Your Claude Code and Codex conversations, grouped by project. Imported chats open on the same agent and
            continue the original session — the agent still remembers everything. Nothing leaves your computer.
          </StepHeader>
          <ImportChats
            doneLabel="Continue"
            onBack={back}
            onDone={(n) => {
              setImported(n);
              advance();
            }}
          />
        </section>
      )}
      {current === 'done' && (
        <DoneStep userId={userId} name={firstName} imported={imported} onSaved={(next) => goNext(next)} onBack={back} />
      )}
    </OnboardingShell>
  );
}

function oauthName(meta: Record<string, unknown>): string {
  const raw =
    (meta.full_name as string | undefined) ||
    (meta.name as string | undefined) ||
    (meta.user_name as string | undefined);
  return raw && raw.trim().length > 0 ? raw.trim() : '';
}
