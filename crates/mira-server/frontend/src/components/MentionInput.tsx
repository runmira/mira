import { useEffect, useRef, type KeyboardEvent, type ClipboardEvent } from 'react';
import { cn } from '@/lib/utils';
import { parseSkillMentions } from './SkillMention';
import type { PaletteSkill } from './commands';

/**
 * Codex-style composer input surface: a `contenteditable` div that
 * renders `@skill:<name>` mentions as inline chip elements while the
 * user types. Externally it behaves like a textarea — the parent tracks
 * a canonical text string with tokens, and typing / paste / imperative
 * inserts all funnel through `onChange(newText)`.
 *
 * ## Model
 *
 * The DOM is the source of truth for what's on screen; state is derived
 * from it. React does NOT re-render the innerHTML on every state
 * update — that would clobber the user's typing. Instead, the parent
 * seeds the initial content via `value`, and afterwards the ONLY paths
 * that mutate the DOM are:
 *
 *   1. User typing / paste / OS input (browser mutates the DOM directly)
 *   2. Imperative calls through `handleRef` (`.setText`, `.insertMention`,
 *      `.clear`) — used by palette picks and post-send cleanup.
 *
 * After every mutation we re-serialize the DOM to canonical text and
 * fire `onChange` so the parent's state stays in lockstep.
 *
 * ## Chip shape
 *
 * A mention is rendered as a non-editable `<span data-skill="name">`
 * containing an icon + display label. `contenteditable="false"` on
 * the span makes it atomic — backspace removes the whole chip, arrow
 * keys skip past it. Text-node siblings hold the surrounding prose.
 *
 * ## Placeholder + newlines
 *
 * `data-placeholder` + a CSS `:empty::before` rule (`.mention-input`
 * class → styles.css) show placeholder text when the field is empty.
 * `white-space: pre-wrap` on the container makes literal `\n` in text
 * nodes render as line breaks; Shift+Enter (handled by the parent's
 * `onKeyDown`) inserts `\n` via `document.execCommand('insertText',
 * false, '\n')`.
 */

export type MentionInputHandle = {
  /** Move focus into the editor. */
  focus: () => void;
  /** Replace the whole content with `text` (canonical form with
   *  `@skill:<name>` tokens). Fires `onChange`. Caret lands at end. */
  setText: (text: string) => void;
  /** Insert an `@skill:<name>` mention at the current caret. Adds a
   *  leading space when the char before the caret isn't whitespace and
   *  always trails a space so the user can keep typing. */
  insertMention: (name: string) => void;
  /** Wipe the editor and fire `onChange('')`. Cheaper than `setText('')`
   *  because it skips the parse+rebuild step. */
  clear: () => void;
};

type Props = {
  /** Initial canonical text. After mount, this is IGNORED — the
   *  editor is uncontrolled. Use `handleRef.current.setText(...)` to
   *  update programmatically. */
  value: string;
  onChange: (text: string) => void;
  onKeyDown?: (e: KeyboardEvent<HTMLDivElement>) => void;
  onPaste?: (e: ClipboardEvent<HTMLDivElement>) => void;
  /** Image files pasted from the clipboard (screenshots). When set, an
   *  image paste goes here instead of being dropped as plain text. */
  onPasteImages?: (files: File[]) => void;
  placeholder: string;
  disabled?: boolean;
  /** Skill roster — passed to `renderChip` so newly-inserted chips
   *  pick up the right color/label. Stale roster references remain
   *  functional (they still render); the label just won't get pretty
   *  until the next rebuild. */
  roster: PaletteSkill[];
  className?: string;
  ariaLabel?: string;
  /** Ref to the imperative API. Assigned on mount; safe to read from
   *  event handlers in the parent. */
  handleRef: React.MutableRefObject<MentionInputHandle | null>;
};

export function MentionInput({
  value,
  onChange,
  onKeyDown,
  onPaste,
  onPasteImages,
  placeholder,
  disabled = false,
  roster,
  className,
  ariaLabel,
  handleRef,
}: Props) {
  const elRef = useRef<HTMLDivElement | null>(null);
  const rosterRef = useRef(roster);
  useEffect(() => { rosterRef.current = roster; }, [roster]);

  // Initial DOM seed. Runs once so the field reflects `value` on mount,
  // e.g. when the parent restores draft text or re-mounts after a
  // route/session swap. Later updates flow through the imperative
  // handle so React never fights the browser for the caret.
  useEffect(() => {
    const el = elRef.current;
    if (!el) return;
    rebuild(el, value, rosterRef.current);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Install the imperative handle. Recreated only when the parent
  // swaps the ref target (should be never in practice).
  useEffect(() => {
    handleRef.current = {
      focus: () => elRef.current?.focus(),
      setText: (text) => {
        const el = elRef.current;
        if (!el) return;
        rebuild(el, text, rosterRef.current);
        caretToEnd(el);
        onChange(text);
      },
      insertMention: (name) => {
        const el = elRef.current;
        if (!el) return;
        insertMentionAtCaret(el, name, rosterRef.current);
        onChange(serialize(el));
      },
      clear: () => {
        const el = elRef.current;
        if (!el) return;
        el.innerHTML = '';
        onChange('');
      },
    };
    return () => { handleRef.current = null; };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [handleRef]);

  function handleInput() {
    const el = elRef.current;
    if (!el) return;
    onChange(serialize(el));
  }

  function defaultPaste(e: ClipboardEvent<HTMLDivElement>) {
    // Rich HTML paste breaks the "plain-text + chip nodes" invariant
    // (fonts, embedded images, nested contenteditable divs). Force
    // plaintext insertion so mentions in the pasted text still parse
    // correctly on next serialize.
    e.preventDefault();
    const images = Array.from(e.clipboardData.files).filter((f) => f.type.startsWith('image/'));
    if (images.length > 0 && onPasteImages) {
      onPasteImages(images);
      return;
    }
    const text = e.clipboardData.getData('text/plain');
    document.execCommand('insertText', false, text);
    // The insertText hits the DOM synchronously; onInput will fire and
    // sync state. No manual serialize call needed.
  }

  return (
    <div
      ref={elRef}
      role="textbox"
      aria-label={ariaLabel}
      aria-multiline="true"
      aria-disabled={disabled || undefined}
      contentEditable={!disabled}
      suppressContentEditableWarning
      spellCheck
      data-placeholder={placeholder}
      onInput={handleInput}
      onKeyDown={onKeyDown}
      onPaste={onPaste ?? defaultPaste}
      className={cn('mention-input', disabled && 'opacity-60', className)}
    />
  );
}

/* ---------- DOM ↔ canonical text ---------- */

/** Walk `root` and emit canonical text with `@skill:<name>` tokens.
 *  Chip elements return their skill name; `<br>` becomes `\n`; block-
 *  level containers (browsers occasionally insert `<div>` on Enter)
 *  are joined with a newline between siblings. */
function serialize(root: HTMLElement): string {
  let out = '';
  const visit = (n: Node) => {
    if (n.nodeType === Node.TEXT_NODE) {
      out += n.textContent ?? '';
      return;
    }
    if (!(n instanceof HTMLElement)) return;
    if (n.dataset.skill) {
      out += `@skill:${n.dataset.skill}`;
      return;
    }
    if (n.tagName === 'BR') {
      out += '\n';
      return;
    }
    const isBlock = n.tagName === 'DIV' || n.tagName === 'P';
    if (isBlock && out.length > 0 && !out.endsWith('\n')) out += '\n';
    for (const c of Array.from(n.childNodes)) visit(c);
  };
  for (const c of Array.from(root.childNodes)) visit(c);
  return out;
}

/** Replace `root`'s content with a rendering of `text`. Skill mentions
 *  become chip spans; runs of plain text (with embedded `\n`) become
 *  text nodes with explicit `<br>` for each line break so browsers
 *  don't collapse them. */
function rebuild(root: HTMLElement, text: string, roster: PaletteSkill[]) {
  root.innerHTML = '';
  for (const seg of parseSkillMentions(text)) {
    if (seg.kind === 'skill') {
      root.appendChild(buildChipEl(seg.name, roster));
      continue;
    }
    const parts = seg.text.split('\n');
    parts.forEach((line, i) => {
      if (i > 0) root.appendChild(document.createElement('br'));
      if (line.length > 0) root.appendChild(document.createTextNode(line));
    });
  }
}

/** Build one chip element. Inline-block, non-editable, colour taken
 *  from the skill's roster entry (or a hash-derived fallback). The
 *  visual is a leading sparkle icon + display label — no background,
 *  no border, just the accent-coloured icon and text so the chip reads
 *  as an inline word rather than a UI pill. Matches the sent-bubble
 *  `SkillChip` render so the composer and the transcript look
 *  identical for the same skill. */
function buildChipEl(name: string, roster: PaletteSkill[]): HTMLSpanElement {
  const meta = roster.find((s) => s.name.toLowerCase() === name.toLowerCase());
  const label = prettyName(meta?.name ?? name);
  const el = document.createElement('span');
  el.dataset.skill = name;
  el.contentEditable = 'false';
  el.className = 'mention-chip ' + paletteClasses(meta?.color, name);
  el.title = meta?.description ?? `Skill: ${name}`;
  el.appendChild(buildSparkleSvg());
  el.appendChild(document.createTextNode(label));
  return el;
}

/** Phosphor-style filled sparkle rendered as inline SVG so we can tint
 *  it via `color: currentColor` — no mask hackery. Matches the size /
 *  weight of the React `<Sparkle weight="fill">` used in
 *  SkillMention's `SkillChip`. */
function buildSparkleSvg(): SVGSVGElement {
  const ns = 'http://www.w3.org/2000/svg';
  const svg = document.createElementNS(ns, 'svg');
  svg.setAttribute('viewBox', '0 0 256 256');
  svg.setAttribute('fill', 'currentColor');
  svg.setAttribute('class', 'mention-chip-icon');
  svg.setAttribute('aria-hidden', 'true');
  const path = document.createElementNS(ns, 'path');
  path.setAttribute(
    'd',
    'M197.58,129.06,146,110.78,127.72,59.19a15.99,15.99,0,0,0-30.16,0L79.28,110.78,27.68,129.06a16,16,0,0,0,0,30.16l51.6,18.28,18.28,51.6a16,16,0,0,0,30.16,0l18.28-51.6,51.6-18.28a16,16,0,0,0,0-30.16ZM137.38,163.13a8,8,0,0,0-4.87,4.86l-19.9,56.16-19.91-56.16a8,8,0,0,0-4.86-4.86L32.51,144.12l55.13-19.53a8,8,0,0,0,4.87-4.86l19.9-56.16,19.91,56.16a8,8,0,0,0,4.86,4.86l55.13,19.53Z',
  );
  svg.appendChild(path);
  return svg;
}

/** Insert an `@skill:<name>` chip at the current caret. If there's a
 *  selection, it's replaced (matching how typing overwrites a
 *  highlighted range). Adds a leading space when the char before is
 *  non-whitespace and a trailing space so the user can keep typing. */
function insertMentionAtCaret(root: HTMLElement, name: string, roster: PaletteSkill[]) {
  root.focus();
  const sel = window.getSelection();
  if (!sel || sel.rangeCount === 0 || !root.contains(sel.anchorNode)) {
    // No caret in this input — anchor to the end.
    caretToEnd(root);
  }
  const currentSel = window.getSelection();
  if (!currentSel || currentSel.rangeCount === 0) return;
  const range = currentSel.getRangeAt(0);
  range.deleteContents();

  // Peek at the character immediately preceding the caret to decide
  // whether we need a leading space. Look one node back if the caret
  // sits at the very start of its container.
  const needsLead = precedingCharIsWord(range);
  const chip = buildChipEl(name, roster);
  const frag = document.createDocumentFragment();
  if (needsLead) frag.appendChild(document.createTextNode(' '));
  frag.appendChild(chip);
  frag.appendChild(document.createTextNode(' '));
  range.insertNode(frag);

  // Move caret to just after the trailing space.
  const newRange = document.createRange();
  newRange.setStartAfter(chip.nextSibling ?? chip);
  newRange.collapse(true);
  currentSel.removeAllRanges();
  currentSel.addRange(newRange);
}

function precedingCharIsWord(range: Range): boolean {
  const container = range.startContainer;
  const offset = range.startOffset;
  if (container.nodeType === Node.TEXT_NODE && offset > 0) {
    const ch = container.textContent?.[offset - 1];
    return !!ch && !/\s/.test(ch);
  }
  // Non-text container (e.g. right after a chip) — treat as needing a
  // separator so `[chip]@skill:foo` becomes `[chip] @skill:foo` when
  // serialized.
  if (offset === 0) return false;
  const prev = container.childNodes[offset - 1];
  if (!prev) return false;
  if (prev.nodeType === Node.TEXT_NODE) {
    const s = prev.textContent ?? '';
    return s.length > 0 && !/\s$/.test(s);
  }
  return true;
}

function caretToEnd(el: HTMLElement) {
  const range = document.createRange();
  range.selectNodeContents(el);
  range.collapse(false);
  const sel = window.getSelection();
  sel?.removeAllRanges();
  sel?.addRange(range);
}

/* ---------- chip palette ---------- */

/** Tailwind text-colour classes for a chip. Kept in sync with the
 *  palette wheel in SkillMention.tsx so the composer chip and the
 *  sent-bubble chip visually match. Only the text colour is emitted —
 *  the chip has no background or border, so `bg-*` / `border-*`
 *  variants are unnecessary. */
function paletteClasses(color: string | undefined, seed: string): string {
  const key = (color ?? '').toLowerCase();
  switch (key) {
    case 'emerald': case 'green':  return 'text-emerald-300';
    case 'blue': case 'sky':        return 'text-mira-blue';
    case 'violet': case 'purple':   return 'text-violet-300';
    case 'amber': case 'yellow':    return 'text-amber-300';
    case 'pink': case 'rose':       return 'text-pink-300';
  }
  const wheel = [
    'text-emerald-300',
    'text-mira-blue',
    'text-violet-300',
    'text-amber-300',
    'text-pink-300',
    'text-sky-300',
  ];
  let h = 0;
  for (let i = 0; i < seed.length; i++) h = (h * 33) ^ seed.charCodeAt(i);
  return wheel[Math.abs(h) % wheel.length];
}

function prettyName(name: string): string {
  return name
    .split(/[-_]/)
    .filter(Boolean)
    .map((w) => (w === w.toUpperCase() ? w : w[0].toUpperCase() + w.slice(1)))
    .join(' ');
}
