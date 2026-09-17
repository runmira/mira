import { useMemo, useState } from 'react';
import { AccountTypeStep } from './AccountTypeStep';
import { YourNameStep } from './YourNameStep';
import { InviteTeamStep } from './InviteTeamStep';
import { cn } from '@/lib/utils';

export type AccountType = 'personal' | 'team';

export type Profile = {
  id: string;
  account_type: AccountType | null;
  full_name: string | null;
  onboarding_completed_at: string | null;
};

type StepSlug = 'account-type' | 'your-name' | 'invite-team';

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
];

function applicable(steps: Step[], type: AccountType | null): Step[] {
  return steps.filter((s) => s.appliesTo(type));
}

function firstIncomplete(profile: Profile | null): StepSlug {
  if (!profile?.account_type) return 'account-type';
  if (!profile.full_name || profile.full_name.trim().length === 0) return 'your-name';
  return profile.account_type === 'team' ? 'invite-team' : 'your-name';
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

  const steps = useMemo(() => applicable(STEPS, profile?.account_type ?? null), [profile?.account_type]);
  const activeIndex = Math.max(0, steps.findIndex((s) => s.slug === current));

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

  return (
    <div className="mx-auto flex min-h-screen w-full max-w-lg flex-col px-6 py-10">
      <div className="flex items-center gap-2">
        {steps.map((s, i) => (
          <div
            key={s.slug}
            className={cn(
              'h-1 flex-1 rounded-full transition-colors',
              i <= activeIndex ? 'bg-mira-blue' : 'bg-border',
            )}
          />
        ))}
      </div>

      <div className="mt-10 flex-1">
        {current === 'account-type' && (
          <AccountTypeStep
            userId={userId}
            initialProfile={profile}
            onSaved={(next) => goNext(next)}
          />
        )}
        {current === 'your-name' && (
          <YourNameStep
            userId={userId}
            initialProfile={profile}
            fallbackName={oauthName(userMetadata)}
            onSaved={(next) => goNext(next)}
          />
        )}
        {current === 'invite-team' && (
          <InviteTeamStep
            userId={userId}
            onSaved={(next) => goNext(next)}
          />
        )}
      </div>
    </div>
  );
}

function oauthName(meta: Record<string, unknown>): string {
  const raw =
    (meta.full_name as string | undefined) ||
    (meta.name as string | undefined) ||
    (meta.user_name as string | undefined);
  return raw && raw.trim().length > 0 ? raw.trim() : '';
}
