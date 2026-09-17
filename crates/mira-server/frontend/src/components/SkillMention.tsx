import { Sparkle } from '@phosphor-icons/react';
import { cn } from '@/lib/utils';
import type { PaletteSkill } from './commands';

/**
 * Inline `@skill:<name>` mentions.
 *
 * The composer inserts `@skill:<name> ` as plain text when the user picks
 * a skill from the palette. Rendering treats those tokens as pills —
 * both in the sent user bubble and in a small preview strip above the
 * composer.
 *
 * Wire format: raw text with `@skill:<name>` tokens. The model sees
 * them literally in the user message; the `Skill` tool's description
 * teaches it that `@skill:<name>` means "invoke that skill".
 */

/** Regex used to split text on `@skill:<name>` tokens. The identifier
 *  class matches what `SkillRegistry` accepts (kebab / snake case),
 *  slightly restrictive to avoid gobbling punctuation that trails the
 *  token. Case-insensitive so `@Skill:foo` still resolves. */
const SKILL_TOKEN = /@skill:([a-zA-Z0-9][a-zA-Z0-9_-]*)/gi;

export type MentionSegment =
  | { kind: 'text'; text: string }
  | { kind: 'skill'; name: string };

/** Split `text` into an ordered list of plain-text + skill-mention
 *  segments. Adjacent text segments are merged so consumers don't need
 *  to worry about empty runs between two mentions. */
export function parseSkillMentions(text: string): MentionSegment[] {
  const out: MentionSegment[] = [];
  let last = 0;
  for (const m of text.matchAll(SKILL_TOKEN)) {
    const start = m.index ?? 0;
    if (start > last) {
      out.push({ kind: 'text', text: text.slice(last, start) });
    }
    out.push({ kind: 'skill', name: m[1] });
    last = start + m[0].length;
  }
  if (last < text.length) {
    out.push({ kind: 'text', text: text.slice(last) });
  }
  return out;
}

/** All skill names mentioned in `text`, in encounter order + deduped.
 *  Powers the preview strip above the composer so the user sees which
 *  skills will fire on send. */
export function extractSkillMentions(text: string): string[] {
  const seen = new Set<string>();
  const out: string[] = [];
  for (const m of text.matchAll(SKILL_TOKEN)) {
    const name = m[1];
    const key = name.toLowerCase();
    if (seen.has(key)) continue;
    seen.add(key);
    out.push(name);
  }
  return out;
}

/** Insert a mention token at the caret of the given textarea. Returns
 *  the new text + the new caret position; the caller applies both. A
 *  trailing space so the user can immediately keep typing without
 *  gluing the next word onto the token. */
export function insertSkillMention(
  text: string,
  caret: number,
  name: string,
): { text: string; caret: number } {
  const before = text.slice(0, caret);
  const after = text.slice(caret);
  // Add a leading space when the previous char isn't already
  // whitespace, so `foo` + pick becomes `foo @skill:bar `, not
  // `foo@skill:bar `. Skip when we're at the very start of the field.
  const needsLead = before.length > 0 && !/\s$/.test(before);
  const insert = `${needsLead ? ' ' : ''}@skill:${name} `;
  const nextText = before + insert + after;
  return { text: nextText, caret: (before + insert).length };
}

/** Text a user sees vs. text the model sees are the same today — the
 *  `Skill` tool's spec description teaches the model to interpret
 *  `@skill:<name>` as an invocation directive. This helper exists as a
 *  seam in case we later want to strip / rewrite tokens before they
 *  hit the wire. */
export function renderForWire(text: string): string {
  return text;
}

type PaletteRoster = Pick<PaletteSkill, 'name' | 'description'> & {
  color?: string;
  icon?: string;
};

/** Naked "Sparkle Name" tag matching the sent-bubble look — just a
 *  filled sparkle in the skill's accent color followed by the label
 *  in the same color. No background, no border, no padding chrome:
 *  the chip reads as an inline word rather than a UI pill. Colour
 *  follows the same hash-derived wheel used for the settings card so
 *  a mention picks up the same accent as its card. */
export function SkillChip({
  name,
  roster,
}: {
  name: string;
  roster: PaletteRoster[];
  /** Kept as a prop for API stability, but the chip now inherits its
   *  font size from the surrounding text so it always aligns with
   *  whatever context it lives in — sent bubble at 14.5px, composer
   *  at 14px, panel at 13px, etc. */
  size?: 'sm' | 'md';
}) {
  const meta = roster.find((s) => s.name.toLowerCase() === name.toLowerCase());
  const label = prettyName(meta?.name ?? name);
  const palette = paletteFor(meta?.color, name);
  return (
    <span
      // Pure inline flow (no flexbox) so the chip's baseline is the
      // text baseline — critical for landing on the same line as the
      // surrounding prose. An `inline-flex` wrapper would introduce a
      // flex-container baseline that doesn't match the text next to
      // it, making the chip float up or down by a pixel or two. Font
      // size is intentionally not set: the chip inherits from the
      // parent so the label matches whatever text it's inline with,
      // and the sparkle scales relative to that via `size-[0.9em]`.
      className={cn('whitespace-nowrap font-medium', palette.text)}
      title={meta?.description ?? `Skill: ${name}`}
    >
      <Sparkle
        weight="fill"
        // `align-[-0.125em]` nudges the sparkle down from its default
        // baseline alignment so its optical centre lines up with the
        // x-height of the label, matching how the surrounding text
        // reads.
        className="mr-1 inline size-[0.9em] align-[-0.125em]"
      />
      {label}
    </span>
  );
}

/** Render text with `@skill:<name>` tokens replaced by chips. Preserves
 *  whitespace exactly so it can drop into a `whitespace-pre-wrap`
 *  container without ruining line breaks. */
export function SkillMentionText({
  text,
  roster,
  chipSize = 'md',
}: {
  text: string;
  roster: PaletteRoster[];
  chipSize?: 'sm' | 'md';
}) {
  const segs = parseSkillMentions(text);
  return (
    <>
      {segs.map((s, i) =>
        s.kind === 'text' ? (
          <span key={i}>{s.text}</span>
        ) : (
          <SkillChip key={i} name={s.name} roster={roster} size={chipSize} />
        ),
      )}
    </>
  );
}

/* ---------- palette ---------- */

type Palette = {
  /** Tailwind `text-<color>` class — colours both the sparkle icon
   *  (`fill: currentColor`) and the label. The chip no longer has a
   *  background or border, so those slots were dropped. */
  text: string;
};

const HASH_WHEEL: Palette[] = [
  { text: 'text-emerald-300' },
  { text: 'text-mira-blue'   },
  { text: 'text-violet-300'  },
  { text: 'text-amber-300'   },
  { text: 'text-pink-300'    },
  { text: 'text-sky-300'     },
];

function paletteFor(color: string | undefined, seed: string): Palette {
  if (color) {
    switch (color.toLowerCase()) {
      case 'emerald':
      case 'green':
        return HASH_WHEEL[0];
      case 'blue':
      case 'sky':
        return HASH_WHEEL[1];
      case 'violet':
      case 'purple':
        return HASH_WHEEL[2];
      case 'amber':
      case 'yellow':
        return HASH_WHEEL[3];
      case 'pink':
      case 'rose':
        return HASH_WHEEL[4];
    }
  }
  let h = 0;
  for (let i = 0; i < seed.length; i++) h = (h * 33) ^ seed.charCodeAt(i);
  return HASH_WHEEL[Math.abs(h) % HASH_WHEEL.length];
}

/** Turn a kebab-case skill name into a display label — `grill-me` →
 *  `Grill Me`, matching the Codex screenshot. Preserves acronyms
 *  someone typed in all-caps (`PR` stays `PR`). */
function prettyName(name: string): string {
  return name
    .split(/[-_]/)
    .filter(Boolean)
    .map((w) => (w === w.toUpperCase() ? w : w[0].toUpperCase() + w.slice(1)))
    .join(' ');
}
