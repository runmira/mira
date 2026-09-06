import { useEffect, useMemo, useState } from 'react';
import { CircleNotch } from '@phosphor-icons/react';

/**
 * "Simmering… 4s" indicator shown while the model is thinking. Random
 * gerund per turn; live elapsed counter after 1s. Hides on first token.
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

  return (
    <div
      role="status"
      aria-live="polite"
      className="inline-flex max-w-max animate-fade-in items-center gap-2 rounded-full border border-mira-blue/25 bg-mira-blue/[0.07] px-3 py-1.5 text-[13px] text-mira-blue/90"
    >
      <CircleNotch className="size-3 shrink-0 animate-spin text-mira-blue" />
      <span className="font-medium text-foreground">{verb}</span>
      <span className="inline-flex gap-0.5 font-semibold text-mira-blue">
        <span className="animate-[thinking-bounce_1.2s_ease-in-out_infinite]">.</span>
        <span className="animate-[thinking-bounce_1.2s_ease-in-out_infinite_0.15s]">.</span>
        <span className="animate-[thinking-bounce_1.2s_ease-in-out_infinite_0.3s]">.</span>
      </span>
      {elapsed > 0 && (
        <span className="border-l border-white/10 pl-2 font-mono text-[11px] text-muted-foreground">
          {formatElapsed(elapsed)}
        </span>
      )}
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
