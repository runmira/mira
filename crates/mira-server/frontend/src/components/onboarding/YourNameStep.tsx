import { useState } from 'react';
import { upsertProfile } from './persist';
import type { Profile } from './OnboardingFlow';
import { ErrorText, PrimaryButton, StepFooter, StepHeader, field } from './ui';

/**
 * Display name. Team accounts go on to invite teammates; everyone then
 * sets up how Mira runs and can bring their chats over.
 */
export function YourNameStep({
  userId,
  initialProfile,
  fallbackName,
  onSaved,
  onBack,
}: {
  userId: string;
  initialProfile: Profile | null;
  fallbackName: string;
  onSaved: (profile: Profile) => void;
  onBack?: () => void;
}) {
  const [name, setName] = useState<string>(initialProfile?.full_name || fallbackName);
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function submit() {
    const trimmed = name.trim();
    if (trimmed.length === 0) {
      setError('Give us something to call you.');
      return;
    }
    setError(null);
    setPending(true);
    try {
      // Not the last step any more: setting up how Mira runs and
      // bringing chats over come next, for every account.
      onSaved(await upsertProfile(userId, { full_name: trimmed }));
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setPending(false);
    }
  }

  const initials = name
    .trim()
    .split(/\s+/)
    .slice(0, 2)
    .map((w) => w[0]?.toUpperCase() ?? '')
    .join('');

  return (
    <section className="flex flex-col gap-7">
      <StepHeader eyebrow="Your profile" title="What should we call you?">
        This is how your name appears in Mira and to your teammates. You can change it later.
      </StepHeader>

      <div className="flex items-center gap-4">
        <span
          aria-hidden
          className="grid size-14 shrink-0 place-items-center rounded-2xl bg-gradient-to-br from-mira-blue via-[#8b9df9] to-mira-purple text-[19px] font-semibold text-[#0b0d14] shadow-[0_10px_30px_-10px_rgba(122,162,247,0.7)]"
        >
          {initials || '·'}
        </span>
        <label className="flex min-w-0 flex-1 flex-col gap-2">
          <span className="text-[13px] font-medium text-foreground/90">Your name</span>
          <input
            value={name}
            onChange={(e) => setName(e.target.value)}
            placeholder="Ada Lovelace"
            className={field}
            autoFocus
            onKeyDown={(e) => {
              if (e.key === 'Enter') void submit();
            }}
          />
        </label>
      </div>

      {error && <ErrorText>{error}</ErrorText>}

      <StepFooter onBack={onBack}>
        <PrimaryButton pending={pending} onClick={() => void submit()}>
          {pending ? 'Saving…' : 'Continue'}
        </PrimaryButton>
      </StepFooter>
    </section>
  );
}
