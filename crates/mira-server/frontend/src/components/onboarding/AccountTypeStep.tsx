import { useState } from 'react';
import type { LucideIcon } from 'lucide-react';
import { Check, User, Users } from 'lucide-react';
import { getSupabase } from '../../lib/supabase';
import { upsertProfile } from './persist';
import type { AccountType, Profile } from './OnboardingFlow';
import { cn } from '@/lib/utils';
import { ErrorText, PrimaryButton, StepFooter, StepHeader, field } from './ui';

export function AccountTypeStep({
  userId,
  initialProfile,
  onSaved,
}: {
  userId: string;
  initialProfile: Profile | null;
  onSaved: (profile: Profile) => void;
}) {
  const [selected, setSelected] = useState<AccountType | null>(
    initialProfile?.account_type ?? null,
  );
  const [teamName, setTeamName] = useState('');
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function submit() {
    if (!selected) {
      setError('Pick one to continue.');
      return;
    }
    if (selected === 'team' && teamName.trim().length === 0) {
      setError('Give your team a name.');
      return;
    }
    setError(null);
    setPending(true);
    try {
      const profile = await upsertProfile(userId, { account_type: selected });

      if (selected === 'team') {
        const supabase = getSupabase();
        if (!supabase) throw new Error('Supabase not configured');
        const { data: existing } = await supabase
          .from('teams')
          .select('id')
          .eq('owner_id', userId)
          .limit(1)
          .maybeSingle();

        if (!existing) {
          const { data: team, error: teamErr } = await supabase
            .from('teams')
            .insert({ name: teamName.trim(), owner_id: userId })
            .select('id')
            .single();
          if (teamErr) throw new Error(teamErr.message);
          const { error: memberErr } = await supabase
            .from('team_members')
            .insert({ team_id: team.id, user_id: userId, role: 'owner' });
          if (memberErr) throw new Error(memberErr.message);
        } else {
          await supabase.from('teams').update({ name: teamName.trim() }).eq('id', existing.id);
        }
      }

      onSaved(profile);
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setPending(false);
    }
  }

  return (
    <section className="flex flex-col gap-7">
      <StepHeader eyebrow="Welcome to Mira" title="Who are you setting Mira up for?">
        This shapes your workspace. You can add a team later either way.
      </StepHeader>

      <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
        <Option
          Icon={User}
          title="Just me"
          description="My own projects, on my own machine."
          selected={selected === 'personal'}
          onSelect={() => setSelected('personal')}
        />
        <Option
          Icon={Users}
          title="My team"
          description="A company, a group, or a shared codebase."
          selected={selected === 'team'}
          onSelect={() => setSelected('team')}
        />
      </div>

      {selected === 'team' && (
        <label className="onb-in flex flex-col gap-2">
          <span className="text-[13px] font-medium text-foreground/90">Team name</span>
          <input
            value={teamName}
            onChange={(e) => setTeamName(e.target.value)}
            onKeyDown={(e) => e.key === 'Enter' && void submit()}
            placeholder="Acme, Inc."
            className={field}
            autoFocus
          />
        </label>
      )}

      {error && <ErrorText>{error}</ErrorText>}

      <StepFooter>
        <PrimaryButton pending={pending} disabled={!selected} onClick={() => void submit()}>
          {pending ? 'Saving…' : 'Continue'}
        </PrimaryButton>
      </StepFooter>
    </section>
  );
}

function Option({
  Icon,
  title,
  description,
  selected,
  onSelect,
}: {
  Icon: LucideIcon;
  title: string;
  description: string;
  selected: boolean;
  onSelect: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onSelect}
      aria-pressed={selected}
      className={cn(
        'group relative flex flex-col items-start gap-4 rounded-2xl border p-5 text-left transition-[background-color,border-color,box-shadow] duration-200',
        selected
          ? 'border-mira-blue/70 bg-mira-blue/[0.08] shadow-[0_0_0_4px_rgba(122,162,247,0.12)]'
          : 'border-fg/[0.08] bg-fg/[0.035] hover:border-fg/[0.16] hover:bg-fg/[0.06]',
      )}
    >
      <span
        className={cn(
          'grid size-10 place-items-center rounded-xl border transition-colors',
          selected ? 'border-mira-blue/40 bg-mira-blue/15 text-mira-blue' : 'border-fg/[0.08] bg-fg/[0.04] text-foreground/70',
        )}
      >
        <Icon className="size-5" />
      </span>
      <span>
        <span className="block text-[15px] font-semibold text-foreground">{title}</span>
        <span className="mt-1 block text-[12.5px] leading-relaxed text-muted-foreground">{description}</span>
      </span>
      <span
        className={cn(
          'absolute right-4 top-4 grid size-5 place-items-center rounded-full border transition-colors',
          selected ? 'border-mira-blue bg-mira-blue text-mira-on-accent' : 'border-fg/[0.15]',
        )}
      >
        {selected && <Check className="size-3" strokeWidth={3.5} />}
      </span>
    </button>
  );
}
