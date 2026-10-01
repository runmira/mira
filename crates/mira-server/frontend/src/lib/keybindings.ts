/**
 * Configurable keybindings engine (localStorage-backed).
 *
 * `mod+shift+o` key syntax, `when`-clause conditions, most-recent-wins
 * matching. Persistence is localStorage (`mira.keybindings.v1`).
 * Bare single-key bindings (y/n) are allowed. Web-only: `isDesktop` is
 * always false, `isWeb` always true.
 */
import { useSyncExternalStore } from 'react';

export const MAX_KEYBINDING_VALUE_LENGTH = 64;
const MAX_KEYBINDING_WHEN_LENGTH = 256;
export const MAX_WHEN_EXPRESSION_DEPTH = 64;
export const MAX_KEYBINDINGS_COUNT = 256;

/* ------------------------------------------------------------------ */
/* Commands                                                              */
/* ------------------------------------------------------------------ */

export const MIRA_KEYBINDING_COMMANDS = [
  'chat.new',
  'terminal.toggle',
  'approval.accept',
  'approval.reject',
  'review.toggle',
  'sidebar.toggle',
  'settings.toggle',
  'palette.toggle',
] as const;
export type MiraKeybindingCommand = (typeof MIRA_KEYBINDING_COMMANDS)[number];

/* ------------------------------------------------------------------ */
/* Types                                                                 */
/* ------------------------------------------------------------------ */

export interface KeybindingShortcut {
  key: string;
  metaKey: boolean;
  ctrlKey: boolean;
  shiftKey: boolean;
  altKey: boolean;
  modKey: boolean;
}

export type KeybindingWhenNode =
  | { type: 'identifier'; name: string }
  | { type: 'not'; node: KeybindingWhenNode }
  | { type: 'and'; left: KeybindingWhenNode; right: KeybindingWhenNode }
  | { type: 'or'; left: KeybindingWhenNode; right: KeybindingWhenNode };

export interface KeybindingRule {
  key: string;
  command: MiraKeybindingCommand;
  when?: string;
}

export interface ResolvedKeybindingRule {
  command: MiraKeybindingCommand;
  shortcut: KeybindingShortcut;
  whenAst?: KeybindingWhenNode;
}

export type ResolvedKeybindingsConfig = ResolvedKeybindingRule[];

export interface UpsertKeybindingInput {
  command: MiraKeybindingCommand;
  key: string;
  when?: string;
  replace?: { command: MiraKeybindingCommand; key: string; when?: string };
}

export interface RemoveKeybindingInput {
  command: MiraKeybindingCommand;
  key: string;
  when?: string;
}

/* ------------------------------------------------------------------ */
/* Platform                                                              */
/* ------------------------------------------------------------------ */

export function isMacPlatform(platform = navigator.platform): boolean {
  return /mac|iphone|ipad/i.test(platform);
}

/* ------------------------------------------------------------------ */
/* Shortcut parsing (from `mod+shift+o` text to structured shortcut)     */
/* ------------------------------------------------------------------ */

function normalizeKeyToken(token: string): string {
  if (token === 'space') return ' ';
  if (token === 'esc') return 'escape';
  return token;
}

export function parseKeybindingShortcut(value: string): KeybindingShortcut | null {
  const rawTokens = value
    .toLowerCase()
    .split('+')
    .map((token) => token.trim());
  const tokens = [...rawTokens];
  let trailingEmptyCount = 0;
  while (tokens[tokens.length - 1] === '') {
    trailingEmptyCount += 1;
    tokens.pop();
  }
  if (trailingEmptyCount > 0) {
    tokens.push('+');
  }
  if (tokens.some((token) => token.length === 0)) {
    return null;
  }
  if (tokens.length === 0) return null;

  let key: string | null = null;
  let metaKey = false;
  let ctrlKey = false;
  let shiftKey = false;
  let altKey = false;
  let modKey = false;

  for (const token of tokens) {
    switch (token) {
      case 'cmd':
      case 'meta':
        metaKey = true;
        break;
      case 'ctrl':
      case 'control':
        ctrlKey = true;
        break;
      case 'shift':
        shiftKey = true;
        break;
      case 'alt':
      case 'option':
        altKey = true;
        break;
      case 'mod':
        modKey = true;
        break;
      default: {
        if (key !== null) return null;
        key = normalizeKeyToken(token);
      }
    }
  }

  if (key === null) return null;
  return { key, metaKey, ctrlKey, shiftKey, altKey, modKey };
}

/* ------------------------------------------------------------------ */
/* When-expression parsing (`a && !b || (c)`)                            */
/* ------------------------------------------------------------------ */

type WhenToken =
  | { type: 'identifier'; value: string }
  | { type: 'not' }
  | { type: 'and' }
  | { type: 'or' }
  | { type: 'lparen' }
  | { type: 'rparen' };

function tokenizeWhenExpression(expression: string): WhenToken[] | null {
  const tokens: WhenToken[] = [];
  let index = 0;

  while (index < expression.length) {
    const current = expression[index];
    if (!current) break;

    if (/\s/.test(current)) {
      index += 1;
      continue;
    }
    if (expression.startsWith('&&', index)) {
      tokens.push({ type: 'and' });
      index += 2;
      continue;
    }
    if (expression.startsWith('||', index)) {
      tokens.push({ type: 'or' });
      index += 2;
      continue;
    }
    if (current === '!') {
      tokens.push({ type: 'not' });
      index += 1;
      continue;
    }
    if (current === '(') {
      tokens.push({ type: 'lparen' });
      index += 1;
      continue;
    }
    if (current === ')') {
      tokens.push({ type: 'rparen' });
      index += 1;
      continue;
    }

    const identifier = /^[A-Za-z_][A-Za-z0-9_.-]*/.exec(expression.slice(index));
    if (!identifier) {
      return null;
    }
    tokens.push({ type: 'identifier', value: identifier[0] });
    index += identifier[0].length;
  }

  return tokens;
}

export function parseKeybindingWhenExpression(expression: string): KeybindingWhenNode | null {
  const tokens = tokenizeWhenExpression(expression);
  if (!tokens || tokens.length === 0) return null;
  let index = 0;

  const parsePrimary = (depth: number): KeybindingWhenNode | null => {
    if (depth > MAX_WHEN_EXPRESSION_DEPTH) {
      return null;
    }
    const token = tokens[index];
    if (!token) return null;

    if (token.type === 'identifier') {
      index += 1;
      return { type: 'identifier', name: token.value };
    }

    if (token.type === 'lparen') {
      index += 1;
      const expressionNode = parseOr(depth + 1);
      const closeToken = tokens[index];
      if (!expressionNode || !closeToken || closeToken.type !== 'rparen') {
        return null;
      }
      index += 1;
      return expressionNode;
    }

    return null;
  };

  const parseUnary = (depth: number): KeybindingWhenNode | null => {
    let notCount = 0;
    while (tokens[index]?.type === 'not') {
      index += 1;
      notCount += 1;
      if (notCount > MAX_WHEN_EXPRESSION_DEPTH) {
        return null;
      }
    }

    let node = parsePrimary(depth);
    if (!node) return null;

    while (notCount > 0) {
      node = { type: 'not', node };
      notCount -= 1;
    }

    return node;
  };

  const parseAnd = (depth: number): KeybindingWhenNode | null => {
    let left = parseUnary(depth);
    if (!left) return null;

    while (tokens[index]?.type === 'and') {
      index += 1;
      const right = parseUnary(depth);
      if (!right) return null;
      left = { type: 'and', left, right };
    }

    return left;
  };

  const parseOr = (depth: number): KeybindingWhenNode | null => {
    let left = parseAnd(depth);
    if (!left) return null;

    while (tokens[index]?.type === 'or') {
      index += 1;
      const right = parseAnd(depth);
      if (!right) return null;
      left = { type: 'or', left, right };
    }

    return left;
  };

  const ast = parseOr(0);
  if (!ast || index !== tokens.length) return null;
  return ast;
}

export function compileResolvedKeybindingRule(rule: KeybindingRule): ResolvedKeybindingRule | null {
  if (rule.key.length < 1 || rule.key.length > MAX_KEYBINDING_VALUE_LENGTH) return null;
  if (rule.when !== undefined && (rule.when.length < 1 || rule.when.length > MAX_KEYBINDING_WHEN_LENGTH)) {
    return null;
  }
  const shortcut = parseKeybindingShortcut(rule.key);
  if (!shortcut) return null;

  if (rule.when !== undefined) {
    const whenAst = parseKeybindingWhenExpression(rule.when);
    if (!whenAst) return null;
    return { command: rule.command, shortcut, whenAst };
  }

  return { command: rule.command, shortcut };
}

export function compileResolvedKeybindingsConfig(
  config: ReadonlyArray<KeybindingRule>,
): ResolvedKeybindingsConfig {
  const compiled: ResolvedKeybindingRule[] = [];
  for (const rule of config) {
    const result = compileResolvedKeybindingRule(rule);
    if (result) {
      compiled.push(result);
    }
  }
  return compiled.slice(-MAX_KEYBINDINGS_COUNT);
}

/* ------------------------------------------------------------------ */
/* Defaults + merge                                                      */
/* ------------------------------------------------------------------ */

export const DEFAULT_KEYBINDINGS: ReadonlyArray<KeybindingRule> = [
  { key: 'mod+shift+o', command: 'chat.new', when: '!terminalFocus' },
  // NOTE: `mod+j` is listed last so labels prefer it over `ctrl+`` —
  // most-recent match wins for display; both still dispatch.
  { key: 'ctrl+`', command: 'terminal.toggle' },
  { key: 'mod+j', command: 'terminal.toggle' },
  { key: 'y', command: 'approval.accept', when: 'approvalOpen && !editableFocus' },
  { key: 'n', command: 'approval.reject', when: 'approvalOpen && !editableFocus' },
  { key: 'mod+d', command: 'review.toggle', when: '!terminalFocus' },
  { key: 'mod+b', command: 'sidebar.toggle', when: '!terminalFocus' },
  { key: 'mod+,', command: 'settings.toggle', when: '!terminalFocus' },
  { key: 'mod+k', command: 'palette.toggle', when: '!terminalFocus' },
];

export const DEFAULT_RESOLVED_KEYBINDINGS =
  compileResolvedKeybindingsConfig(DEFAULT_KEYBINDINGS);

export function mergeWithDefaultKeybindings(
  custom: ResolvedKeybindingsConfig,
): ResolvedKeybindingsConfig {
  if (custom.length === 0) {
    return [...DEFAULT_RESOLVED_KEYBINDINGS];
  }

  const overriddenCommands = new Set(custom.map((binding) => binding.command));
  const retainedDefaults = DEFAULT_RESOLVED_KEYBINDINGS.filter(
    (binding) => !overriddenCommands.has(binding.command),
  );
  const merged = [...retainedDefaults, ...custom];

  if (merged.length <= MAX_KEYBINDINGS_COUNT) {
    return merged;
  }

  return merged.slice(-MAX_KEYBINDINGS_COUNT);
}

/* ------------------------------------------------------------------ */
/* Runtime matching                                                      */
/* ------------------------------------------------------------------ */

export interface ShortcutEventLike {
  getModifierState?: (key: 'AltGraph') => boolean;
  type?: string;
  code?: string;
  key: string;
  metaKey: boolean;
  ctrlKey: boolean;
  shiftKey: boolean;
  altKey: boolean;
}

export interface ShortcutModifierStateLike {
  metaKey: boolean;
  ctrlKey: boolean;
  shiftKey: boolean;
  altKey: boolean;
}

export interface ShortcutMatchContext {
  terminalFocus: boolean;
  approvalOpen: boolean;
  reviewOpen: boolean;
  settingsOpen: boolean;
  isWeb: boolean;
  isDesktop: boolean;
  /** A text field, textarea, select or rich-text editor owns the keyboard. */
  editableFocus?: boolean;
  [key: string]: boolean | undefined;
}

interface ShortcutMatchOptions {
  platform?: string;
  context?: Partial<ShortcutMatchContext>;
}

const EVENT_CODE_SHORTCUT_KEYS: Readonly<Record<string, string>> = {
  Backquote: '`',
  Backslash: '\\',
  BracketLeft: '[',
  BracketRight: ']',
  Comma: ',',
  Digit0: '0',
  Digit1: '1',
  Digit2: '2',
  Digit3: '3',
  Digit4: '4',
  Digit5: '5',
  Digit6: '6',
  Digit7: '7',
  Digit8: '8',
  Digit9: '9',
  Equal: '=',
  Minus: '-',
  Period: '.',
  Quote: "'",
  Semicolon: ';',
  Slash: '/',
};

function normalizeEventKey(key: string): string {
  const normalized = key.toLowerCase();
  if (normalized === 'esc') return 'escape';
  return normalized;
}

export function shortcutKeyFromEvent(event: Pick<ShortcutEventLike, 'key' | 'code'>): string {
  const layoutKey = normalizeEventKey(event.key);
  if (/^[a-z]$/.test(layoutKey)) return layoutKey;
  const physicalKey = event.code ? EVENT_CODE_SHORTCUT_KEYS[event.code] : undefined;
  return physicalKey ?? layoutKey;
}

function resolveEventKeys(event: ShortcutEventLike): Set<string> {
  const layoutKey = normalizeEventKey(event.key);
  const keys = new Set([layoutKey]);
  // Physical-position fallback for layouts that type non-Latin letters and
  // for Option-modified symbols on macOS. When the layout already produces a
  // Latin letter, match on it alone.
  const letterCode = event.code?.match(/^Key([A-Z])$/)?.[1];
  if (letterCode && !/^[a-z]$/.test(layoutKey)) {
    keys.add(letterCode.toLowerCase());
  }
  keys.add(shortcutKeyFromEvent(event));
  return keys;
}

function matchesShortcutModifiers(
  event: ShortcutModifierStateLike,
  shortcut: KeybindingShortcut,
  platform = navigator.platform,
): boolean {
  const useMetaForMod = isMacPlatform(platform);
  const expectedMeta = shortcut.metaKey || (shortcut.modKey && useMetaForMod);
  const expectedCtrl = shortcut.ctrlKey || (shortcut.modKey && !useMetaForMod);
  return (
    event.metaKey === expectedMeta &&
    event.ctrlKey === expectedCtrl &&
    event.shiftKey === shortcut.shiftKey &&
    event.altKey === shortcut.altKey
  );
}

function matchesShortcut(
  event: ShortcutEventLike,
  shortcut: KeybindingShortcut,
  platform = navigator.platform,
): boolean {
  if (
    !isMacPlatform(platform) &&
    event.getModifierState?.('AltGraph') &&
    !/^[a-z0-9]$/i.test(event.key)
  )
    return false;
  if (!matchesShortcutModifiers(event, shortcut, platform)) return false;
  return resolveEventKeys(event).has(shortcut.key);
}

function resolvePlatform(options: ShortcutMatchOptions | undefined): string {
  return options?.platform ?? navigator.platform;
}

function resolveContext(options: ShortcutMatchOptions | undefined): ShortcutMatchContext {
  return {
    terminalFocus: false,
    approvalOpen: false,
    reviewOpen: false,
    settingsOpen: false,
    isWeb: true,
    isDesktop: false,
    editableFocus: false,
    ...options?.context,
  };
}

function evaluateWhenNode(node: KeybindingWhenNode, context: ShortcutMatchContext): boolean {
  switch (node.type) {
    case 'identifier':
      if (node.name === 'true') return true;
      if (node.name === 'false') return false;
      return Boolean(context[node.name]);
    case 'not':
      return !evaluateWhenNode(node.node, context);
    case 'and':
      return evaluateWhenNode(node.left, context) && evaluateWhenNode(node.right, context);
    case 'or':
      return evaluateWhenNode(node.left, context) || evaluateWhenNode(node.right, context);
  }
}

function matchesWhenClause(
  whenAst: KeybindingWhenNode | undefined,
  context: ShortcutMatchContext,
): boolean {
  if (!whenAst) return true;
  return evaluateWhenNode(whenAst, context);
}

export function shortcutConflictKey(
  shortcut: KeybindingShortcut,
  platform = navigator.platform,
): string {
  const useMetaForMod = isMacPlatform(platform);
  const metaKey = shortcut.metaKey || (shortcut.modKey && useMetaForMod);
  const ctrlKey = shortcut.ctrlKey || (shortcut.modKey && !useMetaForMod);
  return [
    shortcut.key,
    metaKey ? 'meta' : '',
    ctrlKey ? 'ctrl' : '',
    shortcut.shiftKey ? 'shift' : '',
    shortcut.altKey ? 'alt' : '',
  ].join('|');
}

function findEffectiveShortcutForCommand(
  keybindings: ResolvedKeybindingsConfig,
  command: MiraKeybindingCommand,
  options?: ShortcutMatchOptions,
): KeybindingShortcut | null {
  const platform = resolvePlatform(options);
  const context = resolveContext(options);
  const claimedShortcuts = new Set<string>();

  for (let index = keybindings.length - 1; index >= 0; index -= 1) {
    const binding = keybindings[index];
    if (!binding) continue;
    if (!matchesWhenClause(binding.whenAst, context)) continue;

    const conflictKey = shortcutConflictKey(binding.shortcut, platform);
    if (claimedShortcuts.has(conflictKey)) {
      continue;
    }

    claimedShortcuts.add(conflictKey);
    if (binding.command === command) {
      return binding.shortcut;
    }
  }

  return null;
}

export function matchesCommandShortcut(
  event: ShortcutEventLike,
  keybindings: ResolvedKeybindingsConfig,
  command: MiraKeybindingCommand,
  options?: ShortcutMatchOptions,
): boolean {
  return resolveShortcutCommand(event, keybindings, options) === command;
}

/** Most-recent matching binding wins — custom bindings shadow defaults. */
export function resolveShortcutCommand(
  event: ShortcutEventLike,
  keybindings: ResolvedKeybindingsConfig,
  options?: ShortcutMatchOptions,
): MiraKeybindingCommand | null {
  const platform = resolvePlatform(options);
  const context = resolveContext(options);

  for (let index = keybindings.length - 1; index >= 0; index -= 1) {
    const binding = keybindings[index];
    if (!binding) continue;
    if (!matchesWhenClause(binding.whenAst, context)) continue;
    if (!matchesShortcut(event, binding.shortcut, platform)) continue;
    return binding.command;
  }
  return null;
}

export function formatShortcutKeyLabel(key: string): string {
  if (key === ' ') return 'Space';
  if (key.length === 1) return key.toUpperCase();
  if (key === 'escape') return 'Esc';
  if (key === 'arrowup') return 'Up';
  if (key === 'arrowdown') return 'Down';
  if (key === 'arrowleft') return 'Left';
  if (key === 'arrowright') return 'Right';
  return key.slice(0, 1).toUpperCase() + key.slice(1);
}

export function formatShortcutLabel(
  shortcut: KeybindingShortcut,
  platform = navigator.platform,
): string {
  const keyLabel = formatShortcutKeyLabel(shortcut.key);
  const useMetaForMod = isMacPlatform(platform);
  const showMeta = shortcut.metaKey || (shortcut.modKey && useMetaForMod);
  const showCtrl = shortcut.ctrlKey || (shortcut.modKey && !useMetaForMod);
  const showAlt = shortcut.altKey;
  const showShift = shortcut.shiftKey;

  if (useMetaForMod) {
    return `${showCtrl ? '⌃' : ''}${showAlt ? '⌥' : ''}${showShift ? '⇧' : ''}${showMeta ? '⌘' : ''}${keyLabel}`;
  }

  const parts: string[] = [];
  if (showCtrl) parts.push('Ctrl');
  if (showAlt) parts.push('Alt');
  if (showShift) parts.push('Shift');
  if (showMeta) parts.push('Meta');
  parts.push(keyLabel);
  return parts.join('+');
}

export function shortcutLabelForCommand(
  keybindings: ResolvedKeybindingsConfig,
  command: MiraKeybindingCommand | null,
  options?: ShortcutMatchOptions,
): string | null {
  if (command === null) return null;
  const shortcut = findEffectiveShortcutForCommand(keybindings, command, options);
  return shortcut ? formatShortcutLabel(shortcut, resolvePlatform(options)) : null;
}

/* ------------------------------------------------------------------ */
/* localStorage store                                                    */
/* ------------------------------------------------------------------ */

const STORAGE_KEY = 'mira.keybindings.v1';

function readCustomRules(): KeybindingRule[] {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (!raw) return [];
    const parsed: unknown = JSON.parse(raw);
    if (!Array.isArray(parsed)) return [];
    const out: KeybindingRule[] = [];
    for (const entry of parsed) {
      if (
        typeof entry === 'object' &&
        entry !== null &&
        typeof (entry as { key?: unknown }).key === 'string' &&
        typeof (entry as { command?: unknown }).command === 'string' &&
        (MIRA_KEYBINDING_COMMANDS as ReadonlyArray<string>).includes(
          (entry as { command: string }).command,
        ) &&
        ((entry as { when?: unknown }).when === undefined ||
          typeof (entry as { when?: unknown }).when === 'string')
      ) {
        out.push(entry as KeybindingRule);
      }
    }
    return out;
  } catch {
    return [];
  }
}

function customKeyOf(rule: { command: string; key: string; when?: string }): string {
  return `${rule.command}\u0000${rule.key}\u0000${rule.when?.trim() ?? ''}`;
}

type Listener = () => void;
const listeners = new Set<Listener>();
let cachedCustom: KeybindingRule[] | null = null;
let cachedResolved: ResolvedKeybindingsConfig | null = null;

function customRules(): KeybindingRule[] {
  if (!cachedCustom) cachedCustom = readCustomRules();
  return cachedCustom;
}

function persistCustom(rules: KeybindingRule[]) {
  cachedCustom = [...rules];
  cachedResolved = null;
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(rules));
  } catch {
    /* private mode — keep in-memory */
  }
  listeners.forEach((l) => l());
}

function snapshot(): ResolvedKeybindingsConfig {
  if (!cachedResolved) {
    cachedResolved = mergeWithDefaultKeybindings(compileResolvedKeybindingsConfig(customRules()));
  }
  return cachedResolved;
}

function subscribe(listener: Listener): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

/** Live resolved config (defaults + custom). Re-renders on every edit. */
export function useKeybindings(): ResolvedKeybindingsConfig {
  return useSyncExternalStore(subscribe, snapshot, snapshot);
}

/** Raw custom rules (for export). */
export function getCustomKeybindingRules(): KeybindingRule[] {
  return [...customRules()];
}

/** Replace the whole custom set (for import). Returns an error message or null. */
export function importCustomKeybindingRules(json: string): string | null {
  let parsed: unknown;
  try {
    parsed = JSON.parse(json);
  } catch {
    return 'That file is not valid JSON.';
  }
  if (!Array.isArray(parsed)) return 'Expected a JSON array of { key, command, when? }.';
  const rules: KeybindingRule[] = [];
  for (const entry of parsed) {
    if (
      typeof entry !== 'object' ||
      entry === null ||
      typeof (entry as { key?: unknown }).key !== 'string' ||
      typeof (entry as { command?: unknown }).command !== 'string'
    ) {
      return 'Each entry needs a string "key" and "command".';
    }
    const e = entry as { key: string; command: string; when?: unknown };
    if (!(MIRA_KEYBINDING_COMMANDS as ReadonlyArray<string>).includes(e.command)) {
      return `Unknown command: ${e.command}.`;
    }
    if (e.when !== undefined && typeof e.when !== 'string') {
      return 'The "when" clause must be a string.';
    }
    const compiled = compileResolvedKeybindingRule({
      key: e.key,
      command: e.command as MiraKeybindingCommand,
      ...(typeof e.when === 'string' ? { when: e.when } : {}),
    });
    if (!compiled) return `Could not parse binding "${e.key}" → ${e.command}.`;
    rules.push({
      key: e.key,
      command: e.command as MiraKeybindingCommand,
      ...(typeof e.when === 'string' ? { when: e.when } : {}),
    });
  }
  if (rules.length > MAX_KEYBINDINGS_COUNT) {
    return `Too many bindings (max ${MAX_KEYBINDINGS_COUNT}).`;
  }
  persistCustom(rules);
  return null;
}

export function upsertKeybinding(input: UpsertKeybindingInput): void {
  const key = input.key.trim();
  if (!key) return;
  const when = input.when?.trim() ? input.when.trim() : undefined;
  const next = customRules().filter((rule) => {
    // Drop the replaced binding…
    if (
      input.replace &&
      rule.command === input.replace.command &&
      rule.key === input.replace.key &&
      (rule.when?.trim() ?? '') === (input.replace.when?.trim() ?? '')
    ) {
      return false;
    }
    // …and any custom rule this upsert shadows (same command+key+when).
    return (
      customKeyOf({ command: rule.command, key: rule.key, when: rule.when }) !==
      customKeyOf({ command: input.command, key, when })
    );
  });
  next.push({ command: input.command, key, ...(when ? { when } : {}) });
  persistCustom(next.slice(-MAX_KEYBINDINGS_COUNT));
}

export function removeKeybinding(input: RemoveKeybindingInput): void {
  persistCustom(
    customRules().filter(
      (rule) =>
        !(
          rule.command === input.command &&
          rule.key === input.key &&
          (rule.when?.trim() ?? '') === (input.when?.trim() ?? '')
        ),
    ),
  );
}

/** Reset one command to defaults: drop every custom rule for it. */
export function resetKeybindingCommand(command: MiraKeybindingCommand): void {
  persistCustom(customRules().filter((rule) => rule.command !== command));
}
