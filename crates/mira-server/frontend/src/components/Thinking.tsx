import { useEffect, useMemo, useState } from 'react';
import miraLogo from '../assets/mira-logo.png';

/**
 * "Simmering 4s" indicator shown while the model is thinking.
 *
 * Mira logo rotating alongside a random gerund with a slow
 * left-to-right shimmer sweep. The logo spins at 3s / revolution —
 * fast enough to signal life, slow enough not to be dizzying next to
 * quiet prose. Live elapsed counter after 1s. Hides on first token /
 * tool_start.
 */
export function Thinking() {
  const verb = useMemo(() => pickVerb(), []);
  const startedAt = useMemo(() => Date.now(), []);
  const [elapsed, setElapsed] = useState(0);

  useEffect(() => {
    const id = setInterval(() => {
      setElapsed(Math.floor((Date.now() - startedAt) / 1000));
    }, 250);
    return () => clearInterval(id);
  }, [startedAt]);

  const label = elapsed > 0 ? `${verb} ${formatElapsed(elapsed)}` : verb;

  return (
    <div
      role="status"
      aria-live="polite"
      className="flex animate-fade-in items-center gap-2 text-[13.5px] font-medium tracking-tight"
    >
      {/* Slow rotation — Tailwind's built-in `animate-spin` is 1s
          linear, which reads as urgent/jittery next to prose. 3s
          linear keeps the motion present but calm, matching the
          shimmer sweep timing. */}
      <img
        src={miraLogo}
        alt=""
        aria-hidden="true"
        draggable={false}
        className="size-4 shrink-0 rounded-full object-contain animate-[spin_3s_linear_infinite]"
      />
      <span
        // Gradient text: a bright band travels through the muted-grey base.
        // `bg-clip-text` + `text-transparent` lets the moving gradient show
        // through the glyphs. `background-size: 200%` gives the shimmer
        // enough runway to feel like a sweep, not a flash.
        className="animate-text-shimmer inline-block bg-clip-text text-transparent"
        style={{
          backgroundImage:
            'linear-gradient(90deg, hsl(0 0% 100% / 0.28) 0%, hsl(0 0% 100% / 0.28) 40%, hsl(0 0% 100% / 0.95) 50%, hsl(0 0% 100% / 0.28) 60%, hsl(0 0% 100% / 0.28) 100%)',
          backgroundSize: '200% 100%',
        }}
      >
        {label}
      </span>
    </div>
  );
}

function formatElapsed(secs: number): string {
  if (secs < 60) return `${secs}s`;
  const m = Math.floor(secs / 60);
  const s = secs % 60;
  return `${m}m ${s}s`;
}

function pickVerb(): string {
  return VERBS[Math.floor(Math.random() * VERBS.length)];
}

const VERBS: readonly string[] = [
  'Thinking', 'Pondering', 'Musing', 'Ruminating', 'Contemplating',
  'Deliberating', 'Cerebrating', 'Cogitating', 'Reflecting', 'Considering',
  'Puzzling', 'Brainstorming', 'Weighing', 'Analyzing', 'Processing',
  'Computing', 'Calculating', 'Working', 'Fidgeting', 'Fussing',
  'Puttering', 'Tinkering', 'Rummaging', 'Scheming', 'Plotting',
  'Concocting', 'Devising', 'Formulating', 'Crafting', 'Weaving',
  'Spinning', 'Conjuring', 'Fabricating', 'Assembling', 'Constructing',
  'Building', 'Forging', 'Sculpting', 'Molding', 'Distilling',
  'Refining', 'Polishing', 'Curating', 'Simmering', 'Percolating',
  'Brewing', 'Steeping', 'Marinating', 'Kneading', 'Mulling',
  'Wondering', 'Noodling', 'Untangling', 'Deducing', 'Investigating',
  'Sleuthing', 'Excavating', 'Spelunking', 'Charting', 'Mapping',
  'Divining', 'Foraging', 'Gathering', 'Sifting', 'Piecing',
  'Threading', 'Stitching', 'Grokking', 'Vibing', 'Cooking',
  'Chopping', 'Whisking', 'Stirring', 'Loading', 'Warming up',
  'Spinning up', 'Firing up', 'Rifling', 'Skimming', 'Doodling',
  'Hatching', 'Sketching', 'Whittling', 'Chiseling', 'Buffing',
  'Fettling', 'Dithering',
];
