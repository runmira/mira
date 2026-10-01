/**
 * A subagent's face — the little character that stands for it.
 *
 * Soft, round 2D characters in the spirit of the "agents as companions"
 * wave: each subagent has its own body shape, color and eyes, and the face
 * shows what it's doing — blinking while idle, bobbing and glancing around
 * while working, beaming when done, worried when something failed.
 *
 * Pure SVG + CSS, no assets. A subagent without a configured face gets a
 * stable one derived from its id, so every subagent has a recognisable
 * face from the first time it appears.
 */
import { useId } from 'react';
import { cn } from '@/lib/utils';

export type FaceSpec = {
  color?: string | null;
  shape?: string | null;
  eyes?: string | null;
  cheeks?: boolean | null;
};

export type FaceState = 'idle' | 'working' | 'waiting' | 'done' | 'error';

export const FACE_COLORS = [
  '#2dd4bf', '#60a5fa', '#a78bfa', '#f472b6', '#fb7185',
  '#f59e0b', '#a3e635', '#34d399', '#38bdf8', '#e879f9',
];
export const FACE_SHAPES = ['round', 'squircle', 'blob', 'drop', 'ghost', 'star'] as const;
export const FACE_EYES = ['dot', 'happy', 'wide', 'sleepy', 'wink', 'visor'] as const;

function hash(s: string): number {
  let h = 2166136261;
  for (let i = 0; i < s.length; i++) {
    h ^= s.charCodeAt(i);
    h = Math.imul(h, 16777619);
  }
  return h >>> 0;
}

/** The face to draw: the configured one, gaps filled from the id. */
export function resolveFace(id: string, face?: FaceSpec | null): Required<{ [K in keyof FaceSpec]: NonNullable<FaceSpec[K]> }> {
  const h = hash(id || 'agent');
  return {
    color: face?.color || FACE_COLORS[h % FACE_COLORS.length],
    shape: face?.shape || FACE_SHAPES[(h >>> 4) % FACE_SHAPES.length],
    eyes: face?.eyes || FACE_EYES[(h >>> 8) % FACE_EYES.length],
    cheeks: face?.cheeks ?? (h >>> 12) % 2 === 0,
  };
}

const BODY: Record<string, string> = {
  round: 'M32 8c13.3 0 24 10.7 24 24s-10.7 24-24 24S8 45.3 8 32 18.7 8 32 8z',
  squircle: 'M32 8c17 0 24 7 24 24s-7 24-24 24S8 49 8 32 15 8 32 8z',
  blob: 'M33 7c12 0 23 8 23 21 0 9-3 14-6 19-5 7-11 10-19 10C18 57 8 48 8 34 8 20 19 7 33 7z',
  drop: 'M32 5c4 7 24 21 24 35 0 11-10 18-24 18S8 51 8 40C8 26 28 12 32 5z',
  ghost: 'M32 7c13 0 23 10 23 23v24c0 3-3 4-5 2l-3-3-4 4-4-4-4 4-3-3-4 4-4-4-3 3c-2 2-5 1-5-2V30C9 17 19 7 32 7z',
  star: 'M32 6c4 0 6 6 9 9s10 1 12 5-3 8-3 12 6 8 4 12-9 3-12 6-5 9-10 9-7-6-10-9-10-2-12-6 4-8 4-12-6-8-4-12 9-2 12-5 6-9 10-9z',
};

const INK = '#1e1b2e';

function Eyes({ kind, state, cx = 32, y = 30 }: { kind: string; state: FaceState; cx?: number; y?: number }) {
  // State overrides: a finished face beams, a failed one worries.
  if (state === 'done') kind = 'happy';
  const l = cx - 8;
  const r = cx + 8;
  switch (kind) {
    case 'happy':
      return (
        <g stroke={INK} strokeWidth="2.6" strokeLinecap="round" fill="none">
          <path d={`M${l - 3.5} ${y + 1.5}q3.5-4.5 7 0`} />
          <path d={`M${r - 3.5} ${y + 1.5}q3.5-4.5 7 0`} />
        </g>
      );
    case 'sleepy':
      return (
        <g stroke={INK} strokeWidth="2.6" strokeLinecap="round" fill="none">
          <path d={`M${l - 3.5} ${y}q3.5 3 7 0`} />
          <path d={`M${r - 3.5} ${y}q3.5 3 7 0`} />
        </g>
      );
    case 'wink':
      return (
        <g className="mf-blink">
          <circle cx={l} cy={y} r="3.3" fill={INK} />
          <path d={`M${r - 3.5} ${y + 1}q3.5-4 7 0`} stroke={INK} strokeWidth="2.6" strokeLinecap="round" fill="none" />
        </g>
      );
    case 'wide':
      return (
        <g>
          <circle cx={l} cy={y} r="5.4" fill="#fff" />
          <circle cx={r} cy={y} r="5.4" fill="#fff" />
          <g className={state === 'working' ? 'mf-look' : state === 'waiting' ? 'mf-up' : undefined}>
            <circle cx={l + 0.6} cy={y + 0.6} r="2.7" fill={INK} />
            <circle cx={r + 0.6} cy={y + 0.6} r="2.7" fill={INK} />
          </g>
        </g>
      );
    case 'visor':
      return (
        <g>
          <rect x={l - 7} y={y - 5} width={r - l + 14} height="10" rx="5" fill={INK} />
          <g className={state === 'working' ? 'mf-scan' : undefined}>
            <circle cx={l} cy={y} r="2" fill="#a5f3fc" />
            <circle cx={r} cy={y} r="2" fill="#a5f3fc" />
          </g>
        </g>
      );
    default:
      return (
        <g className={cn('mf-blink', state === 'working' && 'mf-look')}>
          <circle cx={l} cy={y} r="3.3" fill={INK} />
          <circle cx={r} cy={y} r="3.3" fill={INK} />
        </g>
      );
  }
}

function Mouth({ state, cx = 32, y = 41 }: { state: FaceState; cx?: number; y?: number }) {
  if (state === 'error') {
    return <path d={`M${cx - 4} ${y + 1.5}q4-3.5 8 0`} stroke={INK} strokeWidth="2.2" strokeLinecap="round" fill="none" />;
  }
  if (state === 'working') {
    return <ellipse cx={cx} cy={y} rx="2.4" ry="1.9" fill={INK} />;
  }
  const wide = state === 'done' ? 5 : 3.5;
  return <path d={`M${cx - wide} ${y - 1}q${wide} ${state === 'done' ? 5 : 3.5} ${wide * 2} 0`} stroke={INK} strokeWidth="2.2" strokeLinecap="round" fill="none" />;
}

export function SubagentFace({
  id,
  face,
  size = 28,
  state = 'idle',
  className,
  animate = true,
}: {
  /** The subagent's id — seeds the face when none is configured. */
  id: string;
  face?: FaceSpec | null;
  size?: number;
  state?: FaceState;
  className?: string;
  /** Off in dense lists, where many moving faces would be noise. */
  animate?: boolean;
}) {
  ensureFaceStyles();
  const f = resolveFace(id, face);
  const gid = useId().replace(/:/g, '');
  const body = BODY[f.shape] ?? BODY.round;
  return (
    <span
      className={cn('relative inline-block shrink-0', className)}
      style={{ width: size, height: size }}
      aria-hidden
    >
      <svg
        viewBox="0 0 64 64"
        width={size}
        height={size}
        className={cn(
          'overflow-visible',
          animate && (state === 'working' ? 'mf-bob-fast' : state === 'idle' || state === 'waiting' ? 'mf-float' : undefined),
        )}
      >
        <defs>
          <radialGradient id={`g${gid}`} cx="35%" cy="28%" r="80%">
            <stop offset="0%" stopColor="#ffffff" stopOpacity="0.55" />
            <stop offset="45%" stopColor={f.color} stopOpacity="1" />
            <stop offset="100%" stopColor={f.color} stopOpacity="1" />
          </radialGradient>
        </defs>
        <ellipse cx="32" cy="60" rx="15" ry="2.6" fill="#000" opacity="0.18" />
        <path
          d={body}
          fill={`url(#g${gid})`}
          stroke="rgba(0,0,0,0.12)"
          strokeWidth="1"
          style={state === 'error' ? { filter: 'saturate(0.55)' } : undefined}
        />
        <Eyes kind={f.eyes} state={state} />
        {f.cheeks && (
          <g fill="#ff6b9a" opacity="0.35">
            <ellipse cx="19" cy="38" rx="4" ry="2.4" />
            <ellipse cx="45" cy="38" rx="4" ry="2.4" />
          </g>
        )}
        <Mouth state={state} />
        {state === 'error' && (
          <g stroke={INK} strokeWidth="2" strokeLinecap="round">
            <path d="M20 21l7 2.5" />
            <path d="M44 21l-7 2.5" />
          </g>
        )}
      </svg>
      {state === 'waiting' && (
        <span className="absolute -right-1 -top-1 grid size-3 place-items-center rounded-full bg-amber-400 text-[8px] font-bold leading-none text-black">
          ?
        </span>
      )}
    </span>
  );
}

const FACE_CSS = `
      @keyframes mf-float { 0%,100% { transform: translateY(0) } 50% { transform: translateY(-1.5px) } }
      @keyframes mf-bob { 0%,100% { transform: translateY(0) rotate(0) } 25% { transform: translateY(-2px) rotate(-3deg) } 75% { transform: translateY(-1px) rotate(3deg) } }
      @keyframes mf-blink { 0%,92%,100% { transform: scaleY(1) } 95% { transform: scaleY(0.1) } }
      @keyframes mf-look { 0%,100% { transform: translateX(0) } 30% { transform: translateX(-2px) } 65% { transform: translateX(2px) } }
      @keyframes mf-scan { 0%,100% { transform: translateX(-3px) } 50% { transform: translateX(3px) } }
      .mf-float { animation: mf-float 3.2s ease-in-out infinite; }
      .mf-bob-fast { animation: mf-bob 0.9s ease-in-out infinite; transform-origin: 50% 90%; }
      .mf-blink { animation: mf-blink 4.6s ease-in-out infinite; transform-box: fill-box; transform-origin: center; }
      .mf-look { animation: mf-look 1.8s ease-in-out infinite; }
      .mf-scan { animation: mf-scan 1.2s ease-in-out infinite; }
      .mf-up { transform: translateY(-1.6px); }
      @media (prefers-reduced-motion: reduce) {
        .mf-float, .mf-bob-fast, .mf-blink, .mf-look, .mf-scan { animation: none; }
      }
    `;

/** The face animations, added to the document once. Kept out of the
 *  component tree so they outlive whichever face rendered first. */
function ensureFaceStyles() {
  if (typeof document === 'undefined' || document.getElementById('mira-face-styles')) return;
  const el = document.createElement('style');
  el.id = 'mira-face-styles';
  el.textContent = FACE_CSS;
  document.head.appendChild(el);
}
