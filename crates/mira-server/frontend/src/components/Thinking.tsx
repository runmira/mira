import { useEffect, useMemo, useState } from 'react';

/**
 * The "thinking…" indicator you see while the model is chewing on your
 * message but hasn't started streaming yet. Picks a random gerund per
 * mount (Claude Code-style), animates a spinner, and shows an elapsed
 * counter after the first second so long thinks feel intentional rather
 * than stuck.
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
    <div className="thinking" role="status" aria-live="polite">
      <span className="thinking-spinner" aria-hidden />
      <span className="thinking-verb">{verb}</span>
      <span className="thinking-dots" aria-hidden>
        <span>.</span><span>.</span><span>.</span>
      </span>
      {elapsed > 0 && <span className="thinking-elapsed">{formatElapsed(elapsed)}</span>}
    </div>
  );
}

function formatElapsed(secs: number): string {
  if (secs < 60) return `${secs}s`;
  const m = Math.floor(secs / 60);
  const s = secs % 60;
  return `${m}m ${s}s`;
}

/** Grab a random verb, biased towards the whimsical over the plain. */
function pickVerb(): string {
  return VERBS[Math.floor(Math.random() * VERBS.length)];
}

// 80+ gerunds. Kept mostly PG so a screenshot won't embarrass anyone,
// but leaning weird so the vibe is Claude Code, not Windows loading spinner.
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
