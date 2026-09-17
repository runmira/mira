import { useState } from 'react';
import { markOnboardingComplete, upsertProfile } from './persist';
import type { Profile } from './OnboardingFlow';

/**
 * Step 2: display name. On personal accounts this is the last step, so we
 * mark onboarding complete before advancing. Team accounts continue on to
 * the invite step.
 */
export function YourNameStep({
  userId,
  initialProfile,
  fallbackName,
  onSaved,
}: {
  userId: string;
  initialProfile: Profile | null;
  fallbackName: string;
  onSaved: (profile: Profile) => void;
}) {
  const [name, setName] = useState<string>(initialProfile?.full_name || fallbackName);
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const isPersonal = initialProfile?.account_type === 'personal';

  async function submit() {
    const trimmed = name.trim();
    if (trimmed.length === 0) {
      setError('Give us something to call you.');
      return;
    }
    setError(null);
    setPending(true);
    try {
      const profile = await upsertProfile(userId, { full_name: trimmed });
      if (isPersonal) {
        const done = await markOnboardingComplete(userId);
        onSaved(done);
      } else {
        onSaved(profile);
      }
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setPending(false);
    }
  }

  return (
    <section className="flex flex-col gap-8">
      <header className="flex flex-col gap-2">
        <p className="text-[11px] uppercase tracking-wider text-muted-foreground">Step 2</p>
        <h1 className="text-2xl font-semibold tracking-tight text-foreground">
          What should we call you?
        </h1>
        <p className="text-[13px] text-muted-foreground">
          This is how your name appears in Mira. You can change it later.
        </p>
      </header>

      <label className="flex flex-col gap-2">
        <span className="text-[13px] font-medium text-foreground/90">Your name</span>
        <input
          value={name}
          onChange={(e) => setName(e.target.value)}
          placeholder="Ada Lovelace"
          className="rounded-md border border-border bg-card px-3 py-2 text-[14px] text-foreground outline-none focus:ring-2 focus:ring-mira-blue"
          autoFocus
          onKeyDown={(e) => {
            if (e.key === 'Enter') void submit();
          }}
        />
      </label>

      {error && <p className="text-[13px] text-destructive">{error}</p>}

      <div className="flex justify-end">
        <button
          type="button"
          onClick={() => void submit()}
          disabled={pending}
          className="rounded-md bg-mira-blue px-5 py-2.5 text-[13.5px] font-medium text-white shadow-sm transition hover:opacity-90 disabled:opacity-60"
        >
          {pending ? 'Saving…' : isPersonal ? 'Finish' : 'Continue'}
        </button>
      </div>
    </section>
  );
}
