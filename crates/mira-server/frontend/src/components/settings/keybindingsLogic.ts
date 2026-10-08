/**
 * Keybindings settings logic (rows, conflicts, labels, capture).
 *
 * Row model over the resolved config: sources, default lookup, conflict
 * detection, command labels, and keyboard-event capture (bare single-key
 * bindings like y/n are allowed).
 */
import {
  DEFAULT_RESOLVED_KEYBINDINGS,
  isMacPlatform,
  MIRA_KEYBINDING_COMMANDS,
  parseKeybindingWhenExpression,
  shortcutKeyFromEvent,
  type KeybindingWhenNode,
  type MiraKeybindingCommand,
  type ResolvedKeybindingRule,
  type ResolvedKeybindingsConfig,
} from '@/lib/keybindings';

export type KeybindingSource = 'Default' | 'Custom';

export interface KeybindingRow {
  readonly id: string;
  readonly command: MiraKeybindingCommand;
  readonly key: string;
  readonly when: string;
  readonly source: KeybindingSource;
  readonly defaultKey: string | null;
  readonly defaultWhen: string;
  readonly binding: ResolvedKeybindingRule;
  readonly conflicts: ReadonlyArray<string>;
}

export type WhenVariableOption = string;
export type KeybindingCommandOption = MiraKeybindingCommand;

const CORE_WHEN_VARIABLES = [
  'terminalFocus',
  'approvalOpen',
  'reviewOpen',
  'settingsOpen',
  'editableFocus',
  'isWeb',
  'true',
  'false',
] as const;

const DEFAULT_WHEN_VARIABLES = new Set<string>(CORE_WHEN_VARIABLES);
for (const binding of DEFAULT_RESOLVED_KEYBINDINGS) {
  collectWhenIdentifiersFromNode(binding.whenAst, DEFAULT_WHEN_VARIABLES);
}

export const DEFAULT_WHEN_VARIABLE =
  [...DEFAULT_WHEN_VARIABLES].find(
    (identifier) => identifier !== 'true' && identifier !== 'false',
  ) ?? 'terminalFocus';
const KNOWN_WHEN_VARIABLES = new Set(DEFAULT_WHEN_VARIABLES);

export function shortcutToKeybindingInput(shortcut: {
  modKey: boolean;
  metaKey: boolean;
  ctrlKey: boolean;
  altKey: boolean;
  shiftKey: boolean;
  key: string;
}): string {
  const parts: string[] = [];
  if (shortcut.modKey) parts.push('mod');
  if (shortcut.metaKey) parts.push('meta');
  if (shortcut.ctrlKey) parts.push('ctrl');
  if (shortcut.altKey) parts.push('alt');
  if (shortcut.shiftKey) parts.push('shift');
  parts.push(shortcut.key === ' ' ? 'space' : shortcut.key === 'escape' ? 'esc' : shortcut.key);
  return parts.join('+');
}

export function whenAstToExpression(node: KeybindingWhenNode | undefined): string {
  if (!node) return '';
  switch (node.type) {
    case 'identifier':
      return node.name;
    case 'not':
      return `!${wrapWhenExpression(node.node)}`;
    case 'and':
      return `${wrapWhenExpression(node.left)} && ${wrapWhenExpression(node.right)}`;
    case 'or':
      return `${wrapWhenExpression(node.left)} || ${wrapWhenExpression(node.right)}`;
  }
}

export function whenNodeRemoveLabel(node: KeybindingWhenNode, depth: number): string {
  if (depth === 0) return 'Clear all conditions';
  if (node.type === 'identifier' || (node.type === 'not' && node.node.type === 'identifier')) {
    return 'Remove condition';
  }
  return 'Remove group and its conditions';
}

function wrapWhenExpression(node: KeybindingWhenNode): string {
  if (node.type === 'identifier' || node.type === 'not') return whenAstToExpression(node);
  return `(${whenAstToExpression(node)})`;
}

export function parseWhenExpressionDraft(
  expression: string,
): { ok: true; value: KeybindingWhenNode | undefined } | { ok: false; message: string } {
  const trimmed = expression.trim();
  if (trimmed.length === 0) return { ok: true, value: undefined };

  const ast = parseKeybindingWhenExpression(trimmed);
  if (!ast) {
    return {
      ok: false,
      message: 'Use variables with !, &&, ||, and parentheses.',
    };
  }

  return { ok: true, value: ast };
}

function sourceForBinding(binding: ResolvedKeybindingRule): KeybindingSource {
  const bindingKey = shortcutToKeybindingInput(binding.shortcut);
  const bindingWhen = whenAstToExpression(binding.whenAst);
  const isDefault = DEFAULT_RESOLVED_KEYBINDINGS.some(
    (entry) =>
      entry.command === binding.command &&
      shortcutToKeybindingInput(entry.shortcut) === bindingKey &&
      whenAstToExpression(entry.whenAst) === bindingWhen,
  );

  return isDefault ? 'Default' : 'Custom';
}

function defaultBindingForBinding(
  binding: ResolvedKeybindingRule,
): ResolvedKeybindingRule | undefined {
  const bindingKey = shortcutToKeybindingInput(binding.shortcut);
  const bindingWhen = whenAstToExpression(binding.whenAst);

  return (
    DEFAULT_RESOLVED_KEYBINDINGS.find(
      (entry) =>
        entry.command === binding.command &&
        shortcutToKeybindingInput(entry.shortcut) === bindingKey &&
        whenAstToExpression(entry.whenAst) === bindingWhen,
    ) ??
    DEFAULT_RESOLVED_KEYBINDINGS.find(
      (entry) =>
        entry.command === binding.command && whenAstToExpression(entry.whenAst) === bindingWhen,
    ) ??
    DEFAULT_RESOLVED_KEYBINDINGS.find((entry) => entry.command === binding.command)
  );
}

function keybindingRowId(command: MiraKeybindingCommand, key: string, when: string): string {
  return JSON.stringify([command, key, when]);
}

function conflictsWithWhen(leftWhen: string, rightWhen: string): boolean {
  return leftWhen.length === 0 || rightWhen.length === 0 || leftWhen === rightWhen;
}

export function keybindingConflictLabels(
  rows: ReadonlyArray<KeybindingRow>,
  input: { readonly rowId: string; readonly key: string; readonly when: string },
): ReadonlyArray<string> {
  if (input.key.trim().length === 0) return [];
  const conflicts: Array<string> = [];
  for (const candidate of rows) {
    if (
      candidate.id !== input.rowId &&
      candidate.key === input.key &&
      conflictsWithWhen(candidate.when, input.when)
    ) {
      conflicts.push(commandLabel(candidate.command));
    }
  }
  return [...new Set(conflicts)].sort();
}

export function buildKeybindingRows(
  keybindings: ResolvedKeybindingsConfig,
  query: string,
): ReadonlyArray<KeybindingRow> {
  const normalizedQuery = query.trim().toLowerCase();
  const rows = keybindings.map((binding, index) => {
    const defaultBinding = defaultBindingForBinding(binding);
    const key = shortcutToKeybindingInput(binding.shortcut);
    const when = whenAstToExpression(binding.whenAst);
    return {
      id: `${keybindingRowId(binding.command, key, when)}#${index}`,
      command: binding.command,
      key,
      when,
      source: sourceForBinding(binding),
      defaultKey: defaultBinding ? shortcutToKeybindingInput(defaultBinding.shortcut) : null,
      defaultWhen: whenAstToExpression(defaultBinding?.whenAst),
      binding,
      conflicts: [],
    } satisfies KeybindingRow;
  });

  const rowsWithConflicts = rows.map((row) => {
    const conflicts = keybindingConflictLabels(rows, {
      rowId: row.id,
      key: row.key,
      when: row.when,
    });
    return conflicts.length > 0
      ? Object.assign({}, row, { conflicts: [...new Set(conflicts)].sort() })
      : row;
  });

  rowsWithConflicts.sort((left, right) => {
    const commandCompare = commandLabel(left.command).localeCompare(
      commandLabel(right.command),
    );
    if (commandCompare !== 0) return commandCompare;
    return left.key.localeCompare(right.key);
  });

  if (normalizedQuery.length === 0) {
    return rowsWithConflicts;
  }

  return rowsWithConflicts.filter((row) => {
    return (
      row.command.toLowerCase().includes(normalizedQuery) ||
      commandLabel(row.command).toLowerCase().includes(normalizedQuery) ||
      row.key.toLowerCase().includes(normalizedQuery) ||
      row.when.toLowerCase().includes(normalizedQuery) ||
      row.source.toLowerCase().includes(normalizedQuery)
    );
  });
}

function collectWhenIdentifiersFromNode(
  node: KeybindingWhenNode | undefined,
  identifiers: Set<string>,
): void {
  if (!node) return;
  switch (node.type) {
    case 'identifier':
      identifiers.add(node.name);
      return;
    case 'not':
      collectWhenIdentifiersFromNode(node.node, identifiers);
      return;
    case 'and':
    case 'or':
      collectWhenIdentifiersFromNode(node.left, identifiers);
      collectWhenIdentifiersFromNode(node.right, identifiers);
      return;
  }
}

export function isKnownWhenVariable(identifier: string): boolean {
  return KNOWN_WHEN_VARIABLES.has(identifier);
}

export function unknownWhenVariables(node: KeybindingWhenNode | undefined): ReadonlyArray<string> {
  const identifiers = new Set<string>();
  collectWhenIdentifiersFromNode(node, identifiers);
  return [...identifiers].filter((identifier) => !isKnownWhenVariable(identifier)).sort();
}

export function buildWhenVariableOptions(): ReadonlyArray<WhenVariableOption> {
  return [...KNOWN_WHEN_VARIABLES].sort((left, right) => {
    const leftCoreIndex = (CORE_WHEN_VARIABLES as ReadonlyArray<string>).indexOf(left);
    const rightCoreIndex = (CORE_WHEN_VARIABLES as ReadonlyArray<string>).indexOf(right);
    if (leftCoreIndex !== -1 || rightCoreIndex !== -1) {
      return (
        (leftCoreIndex === -1 ? Number.MAX_SAFE_INTEGER : leftCoreIndex) -
        (rightCoreIndex === -1 ? Number.MAX_SAFE_INTEGER : rightCoreIndex)
      );
    }
    return left.localeCompare(right);
  });
}

export function buildKeybindingCommandOptions(
  keybindings: ResolvedKeybindingsConfig,
): ReadonlyArray<KeybindingCommandOption> {
  const commands = new Set<KeybindingCommandOption>(MIRA_KEYBINDING_COMMANDS);
  for (const binding of keybindings) {
    commands.add(binding.command);
  }
  return [...commands].sort((left, right) =>
    commandLabel(left).localeCompare(commandLabel(right)),
  );
}

const COMMAND_LABELS: Record<MiraKeybindingCommand, string> = {
  'chat.new': 'New chat',
  'panel.tests': 'Open tests',
  'panel.activity': 'Open activity',
  'panel.devices': 'Open device preview',
  'panel.whiteboard': 'Open whiteboard',
  'panel.aside': 'Ask aside',
  'panel.devtools': 'Open developer tools',
  'panel.next': 'Next panel tab',
  'panel.previous': 'Previous panel tab',
  'panel.closeTab': 'Close current panel tab',
  'chat.bottom': 'Jump to latest message',
  'chat.top': 'Jump to first message',
  'composer.attach': 'Attach files',

  'chat.slot1': 'Switch to chat 1',
  'chat.slot2': 'Switch to chat 2',
  'chat.slot3': 'Switch to chat 3',
  'chat.slot4': 'Switch to chat 4',
  'chat.slot5': 'Switch to chat 5',
  'chat.slot6': 'Switch to chat 6',
  'chat.slot7': 'Switch to chat 7',
  'chat.slot8': 'Switch to chat 8',
  'chat.slot9': 'Switch to chat 9',
  'panel.slot1': 'Switch to side panel tab 1',
  'panel.slot2': 'Switch to side panel tab 2',
  'panel.slot3': 'Switch to side panel tab 3',
  'panel.slot4': 'Switch to side panel tab 4',
  'panel.slot5': 'Switch to side panel tab 5',
  'panel.slot6': 'Switch to side panel tab 6',
  'panel.slot7': 'Switch to side panel tab 7',
  'panel.slot8': 'Switch to side panel tab 8',
  'panel.slot9': 'Switch to side panel tab 9',

  'composer.focus': 'Focus composer',
  'chat.stop': 'Stop response',
  'panel.files': 'Open files',
  'panel.processes': 'Open processes',
  'panel.browser': 'Open browser',
  'panel.close': 'Close side panel',
  'chat.copyResponse': 'Copy last response',
  'chat.copyCode': 'Copy last code block',
  'settings.shortcuts': 'Keyboard shortcuts',

  'terminal.toggle': 'Toggle terminal',
  'approval.accept': 'Approve request',
  'approval.reject': 'Reject request',
  'review.toggle': 'Review changes',
  'sidebar.toggle': 'Toggle sidebar',
  'settings.toggle': 'Open Settings',
  'palette.toggle': 'Command palette',
};

export function commandLabel(command: MiraKeybindingCommand): string {
  return COMMAND_LABELS[command] ?? command;
}

function normalizeShortcutKeyToken(key: string): string | null {
  const normalized = key.toLowerCase();
  if (
    normalized === 'meta' ||
    normalized === 'control' ||
    normalized === 'ctrl' ||
    normalized === 'shift' ||
    normalized === 'alt' ||
    normalized === 'option'
  ) {
    return null;
  }
  if (normalized === ' ') return 'space';
  if (normalized === 'escape') return 'esc';
  if (normalized === 'arrowup') return 'arrowup';
  if (normalized === 'arrowdown') return 'arrowdown';
  if (normalized === 'arrowleft') return 'arrowleft';
  if (normalized === 'arrowright') return 'arrowright';
  if (normalized.length === 1) return normalized;
  if (/^f\d{1,2}$/.test(normalized)) return normalized;
  if (normalized === 'enter' || normalized === 'tab' || normalized === 'backspace') {
    return normalized;
  }
  if (normalized === 'delete' || normalized === 'home' || normalized === 'end') {
    return normalized;
  }
  if (normalized === 'pageup' || normalized === 'pagedown') return normalized;
  return null;
}

/**
 * Key string (`mod+shift+o`) from a capture keydown. Bare single keys
 * (y/n) are accepted — Mira's approval shortcuts have no modifier.
 */
export function keybindingFromKeyboardEvent(
  event: Pick<KeyboardEvent, 'key' | 'code' | 'metaKey' | 'ctrlKey' | 'altKey' | 'shiftKey'>,
  platform: string,
): string | null {
  const keyToken = normalizeShortcutKeyToken(shortcutKeyFromEvent(event));
  if (!keyToken) return null;

  const parts: string[] = [];
  if (isMacPlatform(platform)) {
    if (event.metaKey) parts.push('mod');
    if (event.ctrlKey) parts.push('ctrl');
  } else {
    if (event.ctrlKey) parts.push('mod');
    if (event.metaKey) parts.push('meta');
  }
  if (event.altKey) parts.push('alt');
  if (event.shiftKey) parts.push('shift');
  parts.push(keyToken);
  return parts.join('+');
}