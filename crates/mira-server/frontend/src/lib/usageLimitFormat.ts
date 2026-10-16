/** Wording for Usage → Limits: window names and reset countdowns. */

/** `five_hour` → "5-hour", `seven_day_opus` → "Weekly · Opus". */
export function windowLabel(name: string): string {
  const parts = name.toLowerCase().split(/[_\s-]+/);
  const words: Record<string, string> = { one: '1', two: '2', three: '3', four: '4', five: '5', six: '6', seven: '7' };
  let base = name;
  let rest: string[] = [];
  if (parts[0] in words && (parts[1] === 'hour' || parts[1] === 'day')) {
    const n = words[parts[0]];
    base = parts[1] === 'day' && n === '7' ? 'Weekly' : parts[1] === 'day' && n === '1' ? 'Daily' : `${n}-${parts[1]}`;
    rest = parts.slice(2);
  } else {
    rest = parts;
    base = '';
  }
  const tail = rest.map((w) => w.charAt(0).toUpperCase() + w.slice(1)).join(' ');
  return base && tail ? `${base} · ${tail}` : base || tail || name;
}

/** "in 2h 14m", "in 3d 4h"; `null` once it has passed. */
export function resetsIn(resetsAt: number | null | undefined, nowSec: number): string | null {
  if (resetsAt == null) return null;
  const s = resetsAt - nowSec;
  if (s <= 0) return null;
  const d = Math.floor(s / 86400);
  const h = Math.floor((s % 86400) / 3600);
  const m = Math.floor((s % 3600) / 60);
  if (d > 0) return `in ${d}d ${h}h`;
  if (h > 0) return `in ${h}h ${m}m`;
  return `in ${Math.max(1, m)}m`;
}

const NUMBER_WORDS: Record<string, number> = { one: 1, two: 2, three: 3, four: 4, five: 5, six: 6, seven: 7, thirty: 30 };

/** How long a window lasts, in seconds, from its name (`five_hour`,
 *  `seven_day`, `5h`); `null` when the name doesn't say. */
export function windowSeconds(name: string): number | null {
  const parts = name.toLowerCase().split(/[_\s-]+/);
  const short = /^(\d+)([hd])$/.exec(parts[0] ?? '');
  if (short) return Number(short[1]) * (short[2] === 'h' ? 3600 : 86400);
  const n = NUMBER_WORDS[parts[0] ?? ''] ?? Number(parts[0]);
  if (!Number.isFinite(n) || n <= 0) return null;
  if (parts[1] === 'hour') return n * 3600;
  if (parts[1] === 'day') return n * 86400;
  if (parts[1] === 'week') return n * 7 * 86400;
  return null;
}

/** How far through the window we are, 0–1; `null` without its length or reset. */
export function elapsedShare(name: string, resetsAt: number | null | undefined, nowSec: number): number | null {
  const len = windowSeconds(name);
  if (len == null || resetsAt == null) return null;
  const left = resetsAt - nowSec;
  if (left <= 0) return null;
  return Math.min(1, Math.max(0, 1 - left / len));
}

export type Pace = 'ahead' | 'on' | 'behind';

/** Spending faster or slower than an even pace through the window. */
export function paceOf(used: number, elapsed: number | null): Pace | null {
  if (elapsed == null) return null;
  if (used > elapsed + 0.1) return 'ahead';
  if (used < elapsed - 0.1) return 'behind';
  return 'on';
}
