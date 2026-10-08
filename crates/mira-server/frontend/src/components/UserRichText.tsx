import { useMemo } from 'react';
import { GithubRef } from './GithubRef';
import { LinkRef } from './RichRefs';
import { SkillChip } from './SkillMention';
import { parseGithubRef } from '../lib/githubRefs';

/**
 * The user's own message, rendered with the same inline richness the
 * agent's messages get — links and PR references only, for now.
 *
 * Why not just run the text through `Markdown`? Because this is a
 * *sent* message, not model output: it is plain text, never a
 * document. Handing it to a Markdown pipeline would silently turn a
 * pasted `#` or a stray `<` into structure the user never wrote. So the
 * bubble gets a tokeniser instead of a parser — it recognises exactly
 * three things it knows the user meant:
 *
 *   - `@skill:<name>`     → the skill chip (already existed here)
 *   - `owner/repo#123`    → the PR/issue chip, with its state popover
 *   - a bare `http(s)://` → the link chip, with its unfurl hover card
 *
 * Everything else is text, byte for byte. The bubble is
 * `whitespace-pre-wrap`, so segments must preserve whitespace exactly —
 * the tokeniser never trims, and trailing punctuation on a URL is left
 * outside the link where it belongs.
 */

/** One recognised run of a sent message. */
export type UserSegment =
  | { kind: 'text'; text: string }
  | { kind: 'skill'; name: string }
  | { kind: 'link'; href: string }
  | { kind: 'github'; href: string };

/** `@skill:<name>` — kept in sync with `SkillMention`'s own token. */
const SKILL_TOKEN = /@skill:([a-zA-Z0-9][a-zA-Z0-9_-]*)/i;
/** `owner/repo#123`, the shorthand for a PR or issue. */
const GH_REF = /[A-Za-z0-9][\w-]*\/[\w.-]+#\d+\b/;
/** A bare URL. Stops at whitespace and the characters that, in prose,
 *  always belong to the sentence rather than to the link. */
const URL = /https?:\/\/[^\s<>()[\]"'`]+/;

/**
 * Code is off-limits. A fenced block or an inline span is the user
 * quoting something, and a URL inside it is content, not a link —
 * same rule the Markdown path applies with `linkifyGithubRefs`.
 */
const CODE_SPLIT = /(```[\s\S]*?```|`[^`\n]*`)/g;

/** Punctuation that follows a URL in a sentence, not part of it. */
const TRAILING = /[.,;:!?]+$/;

function pushText(out: UserSegment[], text: string) {
  if (!text) return;
  const last = out[out.length - 1];
  // Merge adjacent text runs so the output has no empty gaps for
  // consumers to think about.
  if (last && last.kind === 'text') last.text += text;
  else out.push({ kind: 'text', text });
}

/**
 * Tokenise one run of prose (no code spans).
 *
 * Walks the *original* string with `matchAll` and tracks a cursor,
 * rather than re-slicing and re-running `exec`. The slicing version is
 * the obvious one and it is wrong: a global regex carries `lastIndex`
 * across calls, so after the first match the cursor lands mid-string
 * and every later link is silently swallowed. Group numbering comes
 * from the wrapper parens: 2 = skill name, 3 = PR ref, 4 = URL.
 */
function tokenizeProse(prose: string, out: UserSegment[]) {
  const token = new RegExp(
    `(${SKILL_TOKEN.source})|(${GH_REF.source})|(${URL.source})`,
    'g',
  );
  let last = 0;
  for (const m of prose.matchAll(token)) {
    const start = m.index ?? 0;
    pushText(out, prose.slice(last, start));
    if (m[2]) {
      out.push({ kind: 'skill', name: m[2] });
    } else if (m[3]) {
      // GitHub resolves `/pull/<n>` to the issue when the number is
      // one, so this is the same target the Markdown path builds.
      out.push({ kind: 'github', href: `https://github.com/${m[3].replace('#', '/pull/')}` });
    } else if (m[4]) {
      // Hand back the trailing sentence punctuation: `(see
      // https://example.com).` links the URL, not the period.
      const trimmed = m[4].replace(TRAILING, '');
      const tail = m[4].slice(trimmed.length);
      if (trimmed) out.push({ kind: 'link', href: trimmed });
      pushText(out, tail);
    }
    last = start + m[0].length;
  }
  pushText(out, prose.slice(last));
}

/** Split a sent message into renderable segments. Pure — no DOM, no
 *  React — so the rules above can be read (and tested) on their own. */
export function segmentUserText(text: string): UserSegment[] {
  const out: UserSegment[] = [];
  if (!text) return out;
  // Odd indices are the captured code spans; even ones are prose.
  text.split(CODE_SPLIT).forEach((part, i) => {
    if (i % 2 === 1) pushText(out, part);
    else tokenizeProse(part, out);
  });
  return out;
}

/**
 * Render a sent user message. `roster` is the palette roster, used only
 * to colour the `@skill:` chips; everything else is self-contained.
 *
 * The roster type is inferred from `SkillChip` rather than re-declared,
 * so this component can't drift from the chip it hands text to.
 */
export function UserRichText({
  text,
  roster,
}: {
  text: string;
  roster: Parameters<typeof SkillChip>[0]['roster'];
}) {
  const segments = useMemo(() => segmentUserText(text), [text]);
  return (
    <>
      {segments.map((seg, i) => {
        switch (seg.kind) {
          case 'skill':
            // An unknown skill still renders as a chip: the token is
            // what fires the skill, whether or not the roster knows it.
            return <SkillChip key={i} name={seg.name} roster={roster} />;
          case 'link':
            return <LinkRef key={i} href={seg.href}>{seg.href}</LinkRef>;
          case 'github': {
            const target = parseGithubRef(seg.href);
            if (!target) return <span key={i}>{seg.href}</span>;
            return (
              <GithubRef key={i} target={target} href={seg.href}>
                {`${target.owner}/${target.repo}#${target.number}`}
              </GithubRef>
            );
          }
          default:
            return <span key={i}>{seg.text}</span>;
        }
      })}
    </>
  );
}
