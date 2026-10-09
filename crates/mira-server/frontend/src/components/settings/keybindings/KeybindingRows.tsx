import {
  formatShortcutLabel,
  type KeybindingWhenNode,
  type MiraKeybindingCommand,
  type UpsertKeybindingInput,
} from '@/lib/keybindings';
import { cn } from '@/lib/utils';
import { ChevronDown, Ellipsis, X } from 'lucide-react';
import {
  useReducer,
  useState,
  type KeyboardEvent as ReactKeyboardEvent,
  type ReactNode,
} from 'react';
import { Button } from '../../ui/button';
import { SectionInput } from '../../ui/input';
import { Popover, PopoverContent, PopoverTrigger } from '../../ui/popover';
import { Select } from '../../ui/select';
import {
  commandLabel,
  keybindingConflictLabels,
  keybindingFromKeyboardEvent,
  whenAstToExpression,
  type KeybindingCommandOption,
  type KeybindingRow,
  type WhenVariableOption,
} from '../keybindingsLogic';
import { KeybindingConflictWarning, WhenExpressionBuilder } from './WhenExpressionBuilder';
export function KeybindingPill({ value }: { value: string }) {
  // Keys dedupe repeated parts; a literal "+" in a shortcut splits into empty strings.
  const isMac = /mac|iphone|ipad/i.test(navigator.platform);
  const seenParts = new Map<string, number>();
  const parts = value.split('+').map((part) => {
    const seen = seenParts.get(part) ?? 0;
    seenParts.set(part, seen + 1);
    return { part, key: seen === 0 ? part : `${part}-${seen}` };
  });
  return (
    <span className="inline-flex items-center gap-1">
      {parts.map(({ part, key }) => (
        <kbd
          key={key}
          className="rounded border border-border/70 bg-background/60 px-1.5 py-0.5 font-mono text-[11px] leading-none text-foreground"
        >
          {part === 'mod'
            ? isMac
              ? '⌘'
              : 'Ctrl'
            : part === 'shift'
              ? '⇧'
              : part === 'alt'
                ? isMac
                  ? '⌥'
                  : 'Alt'
                : part === 'ctrl'
                  ? '⌃'
                  : part.length === 1
                    ? part.toUpperCase()
                    : part}
        </kbd>
      ))}
    </span>
  );
}

export function IconButton({
  title,
  onClick,
  disabled,
  children,
  className,
}: {
  title: string;
  onClick?: () => void;
  disabled?: boolean;
  children: ReactNode;
  className?: string;
}) {
  return (
    <button
      type="button"
      title={title}
      aria-label={title}
      disabled={disabled}
      onClick={onClick}
      className={cn(
        'rounded-md p-1.5 text-muted-foreground transition-colors hover:bg-accent hover:text-foreground disabled:opacity-40',
        className,
      )}
    >
      {children}
    </button>
  );
}

export type KeybindingRowDraftState = {
  keyDraft: string;
  whenDraft: KeybindingWhenNode | undefined;
  isRecording: boolean;
  isWhenDraftValid: boolean;
};

export function createKeybindingRowDraft(row: KeybindingRow): KeybindingRowDraftState {
  return {
    keyDraft: row.key,
    whenDraft: row.binding.whenAst,
    isRecording: false,
    isWhenDraftValid: true,
  };
}

export function keybindingRowDraftReducer(
  state: KeybindingRowDraftState,
  patch: Partial<KeybindingRowDraftState>,
): KeybindingRowDraftState {
  return { ...state, ...patch };
}

export function rowKeybindingTarget(row: KeybindingRow): {
  command: MiraKeybindingCommand;
  key: string;
  when?: string;
} {
  return {
    command: row.command,
    key: row.key,
    ...(row.when.trim().length > 0 ? { when: row.when } : {}),
  };
}

/** Draft state and actions for editing one existing binding; layouts decide how to render it. */
export function useKeybindingRowEditor({
  row,
  allRows,
  onSave,
}: {
  row: KeybindingRow;
  allRows: ReadonlyArray<KeybindingRow>;
  onSave: (input: UpsertKeybindingInput) => void;
}) {
  const [draft, setDraft] = useReducer(keybindingRowDraftReducer, row, createKeybindingRowDraft);
  const { keyDraft, whenDraft, isRecording, isWhenDraftValid } = draft;
  const whenDraftExpression = whenAstToExpression(whenDraft);
  const isDirty = keyDraft !== row.key || whenDraftExpression !== row.when;
  const conflictLabels = keybindingConflictLabels(allRows, {
    rowId: row.id,
    key: keyDraft,
    when: whenDraftExpression,
  });

  const save = () => {
    onSave({
      command: row.command,
      key: keyDraft,
      when: whenDraftExpression.trim().length > 0 ? whenDraftExpression : undefined,
      replace: rowKeybindingTarget(row),
    });
  };

  const captureKeybinding = (event: ReactKeyboardEvent<HTMLInputElement>) => {
    if (event.key === 'Tab') return;
    event.preventDefault();
    if (event.key === 'Escape') {
      setDraft({ keyDraft: row.key, isRecording: false });
      return;
    }
    const next = keybindingFromKeyboardEvent(event.nativeEvent, navigator.platform);
    if (!next) return;
    setDraft({ keyDraft: next, isRecording: false });
  };

  return {
    keyDraft,
    whenDraft,
    isRecording,
    isWhenDraftValid,
    whenDraftExpression,
    isDirty,
    conflictLabels,
    setDraft,
    save,
    captureKeybinding,
  };
}

export type KeybindingRowEditor = ReturnType<typeof useKeybindingRowEditor>;

export interface KeybindingRowActions {
  allRows: ReadonlyArray<KeybindingRow>;
  variables: ReadonlyArray<WhenVariableOption>;
  onSave: (input: UpsertKeybindingInput) => void;
  onReset: (row: KeybindingRow) => void;
  onRemove: (row: KeybindingRow) => void;
}

export type KeybindingRowProps = KeybindingRowActions & {
  row: KeybindingRow;
};

/** Shortcut pill that turns into a capture input when clicked, plus Save once the draft changes. */
export function KeybindingKeyControl({
  row,
  editor,
  pillClassName,
}: {
  row: KeybindingRow;
  editor: KeybindingRowEditor;
  pillClassName?: string | undefined;
}) {
  const { keyDraft, isRecording, isDirty, isWhenDraftValid, setDraft, save, captureKeybinding } =
    editor;
  const showPill = !isRecording && keyDraft === row.key && row.key.length > 0 && !isDirty;

  return (
    <>
      {isDirty ? (
        <Button
          size="sm"
          disabled={keyDraft.trim().length === 0 || !isWhenDraftValid}
          onClick={save}
        >
          Save
        </Button>
      ) : null}
      {showPill ? (
        <button
          type="button"
          onClick={() => setDraft({ isRecording: true })}
          aria-label={`Edit shortcut for ${commandLabel(row.command)}: ${formatShortcutLabel(row.binding.shortcut)}`}
          className={cn(
            'inline-flex h-8 cursor-pointer items-center rounded-md border border-transparent px-1.5 outline-none transition-colors hover:border-border/70 hover:bg-accent focus-visible:border-ring sm:h-7',
            pillClassName,
          )}
        >
          <KeybindingPill value={row.key} />
        </button>
      ) : (
        <SectionInput
          data-keybinding-capture=""
          autoFocus={isRecording}
          aria-label={`Keybinding for ${commandLabel(row.command)}`}
          value={isRecording ? '' : keyDraft}
          placeholder={isRecording ? 'Press shortcut' : 'Unassigned'}
          className="h-8 w-44 font-mono text-[12.5px]"
          onFocus={() => setDraft({ isRecording: true })}
          onBlur={() => setDraft({ isRecording: false })}
          onChange={(event) => setDraft({ keyDraft: event.currentTarget.value })}
          onKeyDown={captureKeybinding}
        />
      )}
    </>
  );
}

/** Quiet inline trigger showing the when clause; opens the expression builder. */
export function WhenClauseControl({
  label,
  expression,
  value,
  variables,
  onChange,
  onValidityChange,
}: {
  label: string;
  expression: string;
  value: KeybindingWhenNode | undefined;
  variables: ReadonlyArray<WhenVariableOption>;
  onChange: (value: KeybindingWhenNode | undefined) => void;
  onValidityChange: (valid: boolean) => void;
}) {
  return (
    <Popover>
      <PopoverTrigger asChild>
        <button
          type="button"
          aria-label={`Edit when clause for ${label}`}
          className="flex min-w-0 shrink items-center gap-1 rounded-md px-1.5 py-1 text-[12px] text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
        >
          <span className="truncate font-mono">{expression || 'Always'}</span>
          <ChevronDown className="size-3.5 shrink-0 opacity-60" />
        </button>
      </PopoverTrigger>
      <PopoverContent align="start" sideOffset={6} className="w-auto p-3">
        <WhenExpressionBuilder
          value={value}
          variables={variables}
          onChange={onChange}
          onValidityChange={onValidityChange}
        />
      </PopoverContent>
    </Popover>
  );
}

export function KeybindingRowMenu({
  row,
  onReset,
  onRemove,
}: {
  row: KeybindingRow;
  onReset: (row: KeybindingRow) => void;
  onRemove: (row: KeybindingRow) => void;
}) {
  const canReset = row.source === 'Custom' && row.defaultKey !== null;
  const canRemove = row.source !== 'Default';
  if (!canReset && !canRemove) return null;

  return (
    <Popover>
      <PopoverTrigger asChild>
        <button
          type="button"
          aria-label={`Actions for ${commandLabel(row.command)}`}
          className="rounded-md p-1.5 text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
        >
          <Ellipsis className="size-3.5" />
        </button>
      </PopoverTrigger>
      <PopoverContent align="end" className="w-44">
        {canReset ? (
          <button
            type="button"
            onClick={() => onReset(row)}
            className="flex w-full items-center gap-2 rounded-md px-2.5 py-1.5 text-left text-[13px] text-foreground/85 transition-colors hover:bg-accent hover:text-foreground"
          >
            Reset to default
          </button>
        ) : null}
        {canRemove ? (
          <button
            type="button"
            onClick={() => onRemove(row)}
            className="flex w-full items-center gap-2 rounded-md px-2.5 py-1.5 text-left text-[13px] text-destructive transition-colors hover:bg-destructive/10"
          >
            Remove
          </button>
        ) : null}
      </PopoverContent>
    </Popover>
  );
}

export function KeybindingSourceBadge({ source }: { source: KeybindingRow['source'] }) {
  if (source === 'Default') return null;
  return (
    <span className="rounded-full border border-border/70 px-1.5 py-px text-[10px] font-medium uppercase tracking-wider text-muted-foreground">
      {source}
    </span>
  );
}

export function KeybindingRowTitle({ row }: { row: KeybindingRow }) {
  return (
    <span className="flex items-center gap-2" title={row.command}>
      {commandLabel(row.command)}
      <KeybindingSourceBadge source={row.source} />
    </span>
  );
}

export function KeybindingRowWhen({
  row,
  editor,
  variables,
}: {
  row: KeybindingRow;
  editor: KeybindingRowEditor;
  variables: ReadonlyArray<WhenVariableOption>;
}) {
  return (
    <span className="flex h-6 items-center gap-1.5">
      <span className="text-xs leading-none text-muted-foreground/70">When</span>
      <WhenClauseControl
        label={commandLabel(row.command)}
        expression={editor.whenDraftExpression}
        value={editor.whenDraft}
        variables={variables}
        onChange={(whenDraft) => editor.setDraft({ whenDraft })}
        onValidityChange={(isWhenDraftValid) => editor.setDraft({ isWhenDraftValid })}
      />
    </span>
  );
}

/** Row actions that stay hidden until the row is hovered or holds focus. */
export function KeybindingHoverRowMenu(props: {
  row: KeybindingRow;
  onReset: (row: KeybindingRow) => void;
  onRemove: (row: KeybindingRow) => void;
}) {
  return (
    <span className="flex items-center opacity-0 transition-opacity focus-within:opacity-100 hover:opacity-100 group-hover/row:opacity-100 group-focus-within/row:opacity-100 max-sm:opacity-100">
      <KeybindingRowMenu {...props} />
    </span>
  );
}

/** One binding as a settings row: pills flush right, actions fading in beside them on hover. */
export function KeybindingSettingsRow(props: KeybindingRowProps) {
  const { row, allRows, variables, onSave, onReset, onRemove } = props;
  const editor = useKeybindingRowEditor({ row, allRows, onSave });

  return (
    <div className="group/row flex items-center gap-3 px-4 py-2.5">
      <div className="min-w-0 flex-1">
        <div className="truncate text-[13px] font-medium text-foreground">
          <KeybindingRowTitle row={row} />
        </div>
        <div className="mt-0.5">
          <KeybindingRowWhen row={row} editor={editor} variables={variables} />
        </div>
      </div>
      <div className="flex shrink-0 flex-wrap items-center justify-end gap-1.5">
        <KeybindingConflictWarning labels={editor.conflictLabels} />
        <KeybindingHoverRowMenu row={row} onReset={onReset} onRemove={onRemove} />
        <KeybindingKeyControl row={row} editor={editor} pillClassName="-mr-1.5" />
      </div>
    </div>
  );
}

/** Draft state for a binding that does not exist yet. */
export function useNewKeybindingDraft({
  allRows,
  onSave,
}: {
  allRows: ReadonlyArray<KeybindingRow>;
  onSave: (input: UpsertKeybindingInput) => void;
}) {
  const [commandDraft, setCommandDraft] = useState<MiraKeybindingCommand | ''>('');
  const [draft, setDraft] = useReducer(keybindingRowDraftReducer, {
    keyDraft: '',
    whenDraft: undefined,
    isRecording: false,
    isWhenDraftValid: true,
  });
  const { keyDraft, whenDraft, isRecording, isWhenDraftValid } = draft;
  const whenDraftExpression = whenAstToExpression(whenDraft);
  const conflictLabels = keybindingConflictLabels(allRows, {
    rowId: 'new',
    key: keyDraft,
    when: whenDraftExpression,
  });
  const commandLabelText = commandDraft ? commandLabel(commandDraft) : 'new keybinding';
  const canSave = Boolean(commandDraft) && keyDraft.trim().length > 0 && isWhenDraftValid;

  const save = () => {
    if (!commandDraft) return;
    onSave({
      command: commandDraft,
      key: keyDraft,
      ...(whenDraftExpression.trim().length > 0 ? { when: whenDraftExpression } : {}),
    });
  };

  const captureKeybinding = (event: ReactKeyboardEvent<HTMLInputElement>) => {
    if (event.key === 'Tab') return;
    event.preventDefault();
    if (event.key === 'Escape') {
      setDraft({ keyDraft: '', isRecording: false });
      return;
    }
    const next = keybindingFromKeyboardEvent(event.nativeEvent, navigator.platform);
    if (!next) return;
    setDraft({ keyDraft: next, isRecording: false });
  };

  return {
    commandDraft,
    setCommandDraft,
    keyDraft,
    whenDraft,
    whenDraftExpression,
    isRecording,
    conflictLabels,
    commandLabelText,
    canSave,
    setDraft,
    save,
    captureKeybinding,
  };
}

export type NewKeybindingDraft = ReturnType<typeof useNewKeybindingDraft>;

export interface NewKeybindingProps {
  commandOptions: ReadonlyArray<KeybindingCommandOption>;
  allRows: ReadonlyArray<KeybindingRow>;
  variables: ReadonlyArray<WhenVariableOption>;
  onSave: (input: UpsertKeybindingInput) => void;
  onCancel: () => void;
}

export function NewKeybindingCommandSelect({
  draft,
  commandOptions,
  className,
}: {
  draft: NewKeybindingDraft;
  commandOptions: ReadonlyArray<KeybindingCommandOption>;
  className?: string | undefined;
}) {
  return (
    <Select
      value={draft.commandDraft}
      onChange={(value) => draft.setCommandDraft(value as MiraKeybindingCommand)}
      options={commandOptions.map((command) => ({ value: command, label: commandLabel(command) }))}
      placeholder="Command"
      className={className}
    />
  );
}

export function NewKeybindingKeyInput({
  draft,
  autoFocus = false,
  className,
}: {
  draft: NewKeybindingDraft;
  autoFocus?: boolean;
  className?: string | undefined;
}) {
  return (
    <SectionInput
      data-keybinding-capture=""
      autoFocus={autoFocus}
      aria-label={`Keybinding for ${draft.commandLabelText}`}
      value={draft.isRecording ? '' : draft.keyDraft}
      placeholder={draft.isRecording ? 'Press shortcut' : 'Unassigned'}
      className={cn('font-mono text-[12.5px]', className)}
      onFocus={() => draft.setDraft({ isRecording: true })}
      onBlur={() => draft.setDraft({ isRecording: false })}
      onChange={(event) => draft.setDraft({ keyDraft: event.currentTarget.value })}
      onKeyDown={draft.captureKeybinding}
    />
  );
}

export function NewKeybindingWhen({
  draft,
  variables,
}: {
  draft: NewKeybindingDraft;
  variables: ReadonlyArray<WhenVariableOption>;
}) {
  return (
    <WhenClauseControl
      label={draft.commandLabelText}
      expression={draft.whenDraftExpression}
      value={draft.whenDraft}
      variables={variables}
      onChange={(whenDraft) => draft.setDraft({ whenDraft })}
      onValidityChange={(isWhenDraftValid) => draft.setDraft({ isWhenDraftValid })}
    />
  );
}

/** Add-binding form shaped like the binding rows below it. */
export function NewKeybindingSettingsRow(props: NewKeybindingProps) {
  const { commandOptions, allRows, variables, onSave, onCancel } = props;
  const draft = useNewKeybindingDraft({ allRows, onSave });

  return (
    <div className="bg-muted/15 px-4 py-2.5">
      <div className="flex flex-wrap items-center gap-3">
        <div className="min-w-0 flex-1">
          <div className="text-[13px] font-medium text-foreground">New keybinding</div>
          <span className="mt-0.5 flex h-6 items-center gap-1.5">
            <span className="text-xs leading-none text-muted-foreground/70">When</span>
            <NewKeybindingWhen draft={draft} variables={variables} />
          </span>
        </div>
        <div className="flex shrink-0 flex-wrap items-center gap-2">
          <NewKeybindingCommandSelect
            draft={draft}
            commandOptions={commandOptions}
            className="h-8 w-56 text-[12.5px]"
          />
          <KeybindingConflictWarning labels={draft.conflictLabels} />
          <NewKeybindingKeyInput draft={draft} className="h-8 w-44" />
          <Button size="sm" disabled={!draft.canSave} onClick={draft.save}>
            Save
          </Button>
          <IconButton title="Cancel new keybinding" onClick={onCancel}>
            <X className="size-3.5" />
          </IconButton>
        </div>
      </div>
    </div>
  );
}
