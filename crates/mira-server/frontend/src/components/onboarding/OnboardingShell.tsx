/**
 * The frame around every onboarding step.
 *
 * It paints its own opaque ground. In the desktop app the window is
 * transparent so macOS vibrancy can show through the sidebar, which means
 * anything drawn straight on it sits on the user's wallpaper. The app
 * proper keeps its content on opaque panels; onboarding has no panels, so
 * the shell is one.
 *
 * Layout: a draggable top bar (clearing the traffic lights), a step rail
 * on wide windows, and the step itself on a raised card.
 */
import type { ReactNode } from 'react';
import { Check } from 'lucide-react';
import miraLogo from '../../assets/mira-logo.png';
import { desktopChannel, hasHiddenTitleBar, TRAFFIC_LIGHT_INSET } from '../../lib/desktop';
import { cn } from '@/lib/utils';

export type RailStep = { slug: string; label: string; hint: string };

export function OnboardingShell({
  steps,
  active,
  wide,
  children,
}: {
  steps: RailStep[];
  active: number;
  /** Steps with lists and side-by-side cards get a wider card. */
  wide: boolean;
  children: ReactNode;
}) {
  const inset = hasHiddenTitleBar();
  const channel = desktopChannel();
  return (
    <div className="fixed inset-0 flex flex-col overflow-hidden bg-[#08090b] text-foreground">
      <Ambient />

      <div
        data-tauri-drag-region
        className="relative z-10 flex h-14 shrink-0 items-center gap-3 pr-5"
        style={{ paddingLeft: inset ? TRAFFIC_LIGHT_INSET.left + 8 : 24 }}
      >
        <img src={miraLogo} alt="" draggable={false} className="pointer-events-none size-6 rounded-full" />
        <span className="pointer-events-none text-[14px] font-semibold tracking-tight">Mira</span>
        {channel !== 'stable' && (
          <span
            className={cn(
              'pointer-events-none rounded-full border px-2 py-px text-[10.5px] font-semibold uppercase tracking-[0.08em]',
              channel === 'alpha' ? 'border-rose-400/30 bg-rose-400/10 text-rose-300' : 'border-amber-300/30 bg-amber-300/10 text-amber-200',
            )}
          >
            {channel}
          </span>
        )}
        <span className="pointer-events-none flex-1" />
        <span className="pointer-events-none text-[12px] tabular-nums text-muted-foreground">
          {active + 1} of {steps.length}
        </span>
      </div>

      {/* Compact progress where the rail doesn't fit. */}
      <div className="relative z-10 flex gap-1.5 px-6 xl:hidden">
        {steps.map((s, i) => (
          <span
            key={s.slug}
            className={cn(
              'h-1 flex-1 rounded-full transition-colors duration-500',
              i < active ? 'bg-mira-blue/70' : i === active ? 'bg-mira-blue' : 'bg-white/[0.08]',
            )}
          />
        ))}
      </div>

      <div className="relative z-10 flex min-h-0 flex-1 justify-center gap-10 overflow-y-auto px-6 pb-10 pt-8 xl:pt-14">
        <Rail steps={steps} active={active} />
        <main
          className={cn(
            'w-full self-start transition-[max-width] duration-500 ease-out',
            wide ? 'max-w-[920px]' : 'max-w-[560px]',
          )}
        >
          <div className="relative rounded-[24px] border border-white/[0.08] bg-[#111215]/95 p-7 shadow-[0_40px_120px_-30px_rgba(0,0,0,0.9),0_0_0_1px_rgba(0,0,0,0.6)] sm:p-9">
            {/* Light catching the top edge. */}
            <span className="pointer-events-none absolute inset-x-10 top-0 h-px bg-gradient-to-r from-transparent via-white/25 to-transparent" />
            <div key={steps[active]?.slug} className="onb-in">
              {children}
            </div>
          </div>
        </main>
      </div>
    </div>
  );
}

function Rail({ steps, active }: { steps: RailStep[]; active: number }) {
  return (
    <nav aria-label="Setup steps" className="sticky top-0 hidden w-[210px] shrink-0 self-start pt-3 xl:block">
      <ol className="relative flex flex-col gap-1">
        {/* The spine the step dots sit on. */}
        <span className="absolute bottom-5 left-[13px] top-5 w-px bg-white/[0.07]" />
        <span
          className="absolute left-[13px] top-5 w-px bg-mira-blue/60 transition-[height] duration-500"
          style={{ height: `calc((100% - 40px) * ${steps.length > 1 ? active / (steps.length - 1) : 0})` }}
        />
        {steps.map((s, i) => {
          const done = i < active;
          const now = i === active;
          return (
            <li key={s.slug} className="relative flex items-start gap-3 rounded-xl px-0 py-2">
              <span
                className={cn(
                  'relative z-10 grid size-[27px] shrink-0 place-items-center rounded-full border text-[11.5px] font-semibold tabular-nums transition-colors duration-300',
                  done && 'border-mira-blue/60 bg-mira-blue/15 text-mira-blue',
                  now && 'border-mira-blue bg-mira-blue text-[#0b0d14] shadow-[0_0_0_4px_rgba(122,162,247,0.15)]',
                  !done && !now && 'border-white/[0.1] bg-[#0e0f12] text-muted-foreground/70',
                )}
              >
                {done ? <Check className="size-3.5" strokeWidth={3} /> : i + 1}
              </span>
              <span className="min-w-0 pt-[3px]">
                <span
                  className={cn(
                    'block text-[13px] font-medium transition-colors',
                    now ? 'text-foreground' : done ? 'text-foreground/70' : 'text-muted-foreground/70',
                  )}
                >
                  {s.label}
                </span>
                <span
                  className={cn(
                    'block text-[11.5px] leading-snug transition-colors',
                    now ? 'text-muted-foreground' : 'text-muted-foreground/45',
                  )}
                >
                  {s.hint}
                </span>
              </span>
            </li>
          );
        })}
      </ol>
    </nav>
  );
}

/** Soft colour in the corners and a faint dot grid, fading out to the edges. */
function Ambient() {
  return (
    <div aria-hidden className="pointer-events-none absolute inset-0">
      <div className="absolute -left-40 -top-48 h-[620px] w-[620px] rounded-full bg-[radial-gradient(closest-side,rgba(122,162,247,0.16),transparent)]" />
      <div className="absolute -bottom-56 -right-40 h-[640px] w-[640px] rounded-full bg-[radial-gradient(closest-side,rgba(45,212,191,0.09),transparent)]" />
      <div className="absolute right-[18%] top-[10%] h-[380px] w-[380px] rounded-full bg-[radial-gradient(closest-side,rgba(187,154,247,0.07),transparent)]" />
      <div
        className="absolute inset-0 opacity-[0.5]"
        style={{
          backgroundImage: 'radial-gradient(rgba(255,255,255,0.07) 1px, transparent 1px)',
          backgroundSize: '22px 22px',
          maskImage: 'radial-gradient(ellipse 70% 60% at 50% 40%, black, transparent)',
          WebkitMaskImage: 'radial-gradient(ellipse 70% 60% at 50% 40%, black, transparent)',
        }}
      />
    </div>
  );
}
