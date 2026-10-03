/**
 * The last step: a short "here's how it works" before the app, then
 * onboarding is recorded as finished.
 */
import { useEffect, useState } from 'react';
import { Command, MessagesSquare, Sparkles } from 'lucide-react';
import { markOnboardingComplete } from './persist';
import type { Profile } from './OnboardingFlow';
import { SubagentFace, type FaceSpec } from '../SubagentFace';
import { ErrorText, PrimaryButton, StepFooter, tile } from './ui';
import { cn } from '@/lib/utils';

const TIPS = [
  {
    Icon: MessagesSquare,
    tint: 'text-mira-blue bg-mira-blue/12 border-mira-blue/25',
    title: 'Switch engines per chat',
    body: 'The model picker under the composer holds your providers and your external agents.',
  },
  {
    Icon: Sparkles,
    tint: 'text-mira-purple bg-mira-purple/12 border-mira-purple/25',
    title: 'Meet the subagents',
    body: 'Scout, Iris, Atlas and the rest take focused jobs. Make them yours in Settings.',
  },
  {
    Icon: Command,
    tint: 'text-teal-300 bg-teal-400/12 border-teal-400/25',
    title: 'Everything is ⌘K away',
    body: 'Chats, settings, the browser, the context window — press ⌘K and type.',
  },
];

const FACES: { id: string; face: FaceSpec }[] = [
  { id: 'explore', face: { color: '#2dd4bf', shape: 'round', eyes: 'wide', cheeks: true } },
  { id: 'coder', face: { color: '#a78bfa', shape: 'squircle', eyes: 'happy' } },
  { id: 'reviewer', face: { color: '#f59e0b', shape: 'squircle', eyes: 'visor' } },
];

export function DoneStep({
  userId,
  name,
  imported,
  onSaved,
  onBack,
}: {
  userId: string;
  /** First name, for the greeting. */
  name: string;
  /** Chats brought over in the previous step. */
  imported: number;
  onSaved: (profile: Profile) => void;
  onBack?: () => void;
}) {
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function finish() {
    if (pending) return;
    setPending(true);
    setError(null);
    try {
      onSaved(await markOnboardingComplete(userId));
    } catch (e) {
      setError((e as Error).message);
      setPending(false);
    }
  }

  // Return opens the app, the way the button says.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Enter' && !e.isComposing) void finish();
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  });

  return (
    <section className="flex flex-col gap-7">
      <div className="flex flex-col items-center gap-5 pt-2 text-center">
        <div className="flex items-end gap-2">
          {FACES.map((f, i) => (
            <span
              key={f.id}
              className={cn('onb-in', i === 1 && '-translate-y-1.5')}
              style={{ animationDelay: `${120 + i * 90}ms` }}
            >
              <SubagentFace id={f.id} face={f.face} size={i === 1 ? 44 : 36} />
            </span>
          ))}
        </div>
        <div className="flex flex-col gap-2">
          <h1 className="text-[28px] font-semibold tracking-[-0.022em] text-foreground">
            {name ? `You're all set, ${name}` : "You're all set"}
          </h1>
          <p className="mx-auto max-w-[46ch] text-[14px] leading-relaxed text-muted-foreground">
            {imported > 0
              ? `${imported} chat${imported === 1 ? ' is' : 's are'} waiting in your sidebar, ready to continue. A few things worth knowing:`
              : 'A few things worth knowing before you dive in:'}
          </p>
        </div>
      </div>

      <ul className="grid gap-3 sm:grid-cols-3">
        {TIPS.map(({ Icon, tint, title, body }, i) => (
          <li
            key={title}
            className={cn(tile, 'onb-in flex flex-col gap-3 p-4')}
            style={{ animationDelay: `${200 + i * 80}ms` }}
          >
            <span className={cn('grid size-9 place-items-center rounded-xl border', tint)}>
              <Icon className="size-[18px]" />
            </span>
            <span>
              <span className="block text-[13.5px] font-semibold text-foreground">{title}</span>
              <span className="mt-1 block text-[12.5px] leading-relaxed text-muted-foreground">{body}</span>
            </span>
          </li>
        ))}
      </ul>

      {error && <ErrorText>{error}</ErrorText>}

      <StepFooter
        onBack={onBack}
        status={
          <span className="hidden sm:inline">
            Press <kbd className="rounded-md border border-fg/[0.1] bg-fg/[0.05] px-1.5 py-0.5 font-mono text-[11px]">↵</kbd> to start
          </span>
        }
      >
        <PrimaryButton pending={pending} onClick={() => void finish()}>
          {pending ? 'Opening…' : 'Open Mira'}
        </PrimaryButton>
      </StepFooter>
    </section>
  );
}
