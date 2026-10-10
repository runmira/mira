import {
  getCustomKeybindingRules,
  importCustomKeybindingRules,
  removeKeybinding,
  resetKeybindingCommand,
  upsertKeybinding,
  useKeybindings,
  type UpsertKeybindingInput,
} from '@/lib/keybindings';
import {
  Check,
  Download,
  Keyboard,
  Plus,
  RotateCcw,
  Search,
  TriangleAlert,
  Upload,
  X,
} from 'lucide-react';
import { useEffect, useMemo, useRef, useState, type ReactNode, type Ref } from 'react';
import { SectionInput } from '../ui/input';
import {
  IconButton,
  KeybindingRowActions,
  KeybindingSettingsRow,
  NewKeybindingProps,
  NewKeybindingSettingsRow,
  rowKeybindingTarget,
} from './keybindings/KeybindingRows';
import {
  buildKeybindingCommandOptions,
  buildKeybindingRows,
  buildWhenVariableOptions,
  type KeybindingCommandOption,
  type KeybindingRow,
} from './keybindingsLogic';
function ExpandableHeaderSearch({
  query,
  onChange,
  isOpen,
  onOpenChange,
  inputRef,
  collapsedAccessory,
}: {
  query: string;
  onChange: (next: string) => void;
  isOpen: boolean;
  onOpenChange: (next: boolean) => void;
  inputRef?: Ref<HTMLInputElement>;
  collapsedAccessory?: ReactNode;
}) {
  if (!isOpen) {
    return (
      <>
        {collapsedAccessory}
        <IconButton title="Search keybindings" onClick={() => onOpenChange(true)}>
          <Search className="size-3.5" />
        </IconButton>
      </>
    );
  }

  return (
    <div className="relative w-44">
      <Search
        aria-hidden
        className="pointer-events-none absolute left-2.5 top-1/2 size-3 -translate-y-1/2 text-muted-foreground"
      />
      <SectionInput
        ref={inputRef}
        autoFocus
        type="search"
        value={query}
        onChange={(event) => onChange(event.currentTarget.value)}
        onBlur={() => {
          if (query.length === 0) onOpenChange(false);
        }}
        onKeyDown={(event) => {
          if (event.key === 'Escape') {
            event.preventDefault();
            onChange('');
            onOpenChange(false);
          }
        }}
        placeholder="Search keybindings"
        aria-label="Search keybindings"
        className="h-8 pl-8 text-[12.5px]"
      />
    </div>
  );
}

interface KeybindingsListProps extends KeybindingRowActions {
  rows: ReadonlyArray<KeybindingRow>;
  commandOptions: ReadonlyArray<KeybindingCommandOption>;
  isAddingBinding: boolean;
  onCancelAdd: () => void;
}

/** The add-binding row, one settings row per binding, and the empty state. */
function KeybindingsList(props: KeybindingsListProps) {
  const { rows, commandOptions, isAddingBinding, onCancelAdd, ...rowActions } = props;
  const newProps: NewKeybindingProps = {
    commandOptions,
    allRows: rows,
    variables: rowActions.variables,
    onSave: rowActions.onSave,
    onCancel: onCancelAdd,
  };
  return (
    <div className="divide-y divide-border/30">
      {isAddingBinding ? <NewKeybindingSettingsRow {...newProps} /> : null}
      {rows.map((row) => (
        <KeybindingSettingsRow key={row.id} row={row} {...rowActions} />
      ))}
      {rows.length === 0 && !isAddingBinding ? (
        <div className="px-4 py-12 text-center text-sm text-muted-foreground">
          No keybindings match your search.
        </div>
      ) : null}
    </div>
  );
}

export function AdvancedKeybindingsSection() {
  const keybindings = useKeybindings();
  const [query, setQuery] = useState('');
  const [isSearchOpen, setIsSearchOpen] = useState(false);
  const searchInputRef = useRef<HTMLInputElement>(null);
  const [isAddingBinding, setIsAddingBinding] = useState(false);
  const [importError, setImportError] = useState<string | null>(null);
  const fileInputRef = useRef<HTMLInputElement>(null);
  const rows = useMemo(() => buildKeybindingRows(keybindings, query), [keybindings, query]);
  const commandOptions = useMemo(() => buildKeybindingCommandOptions(keybindings), [keybindings]);
  const whenVariables = useMemo(() => buildWhenVariableOptions(), []);

  useEffect(() => {
    const handleKeyDown = (event: globalThis.KeyboardEvent) => {
      const isMod = event.metaKey || event.ctrlKey;
      if (!isMod || event.altKey || event.key.toLowerCase() !== 'f') return;

      const target = event.target;
      if (
        target !== searchInputRef.current &&
        target instanceof HTMLElement &&
        (target.tagName === 'INPUT' || target.tagName === 'TEXTAREA' || target.isContentEditable)
      ) {
        return;
      }

      event.preventDefault();
      setIsSearchOpen(true);
      requestAnimationFrame(() => {
        searchInputRef.current?.focus();
        searchInputRef.current?.select();
      });
    };
    window.addEventListener('keydown', handleKeyDown);
    return () => window.removeEventListener('keydown', handleKeyDown);
  }, []);

  const saveKeybinding = (input: UpsertKeybindingInput) => {
    upsertKeybinding({
      command: input.command,
      key: input.key.trim(),
      ...(input.when?.trim() ? { when: input.when.trim() } : {}),
      ...(input.replace ? { replace: input.replace } : {}),
    });
    setIsAddingBinding(false);
  };

  const removeRow = (row: KeybindingRow) => {
    removeKeybinding(rowKeybindingTarget(row));
  };

  const resetRow = (row: KeybindingRow) => {
    if (!row.defaultKey) return;
    // Drop the custom override, then restore the default binding.
    removeKeybinding(rowKeybindingTarget(row));
    upsertKeybinding({
      command: row.command,
      key: row.defaultKey,
      ...(row.defaultWhen.trim().length > 0 ? { when: row.defaultWhen } : {}),
    });
  };

  const resetAll = () => {
    for (const command of new Set(rows.map((r) => r.command))) {
      resetKeybindingCommand(command);
    }
  };

  const exportJson = () => {
    const blob = new Blob([JSON.stringify(getCustomKeybindingRules(), null, 2)], {
      type: 'application/json',
    });
    const url = URL.createObjectURL(blob);
    const a = document.createElement('a');
    a.href = url;
    a.download = 'mira-keybindings.json';
    a.click();
    URL.revokeObjectURL(url);
  };

  const importJson = (file: File | undefined) => {
    if (!file) return;
    setImportError(null);
    void file.text().then((text) => {
      const err = importCustomKeybindingRules(text);
      setImportError(err);
    });
  };

  const bindingsCount = (
    <span className="text-[11px] text-muted-foreground">
      {rows.length + (isAddingBinding ? 1 : 0)}{' '}
      {rows.length + (isAddingBinding ? 1 : 0) === 1 ? 'binding' : 'bindings'}
    </span>
  );

  return (
    <div className="flex flex-col gap-2.5">
      <section className="flex flex-col gap-2.5">
        <div className="flex min-h-7 items-start justify-between gap-4 px-1">
          <div className="min-w-0">
            <h2 className="flex min-h-7 items-center gap-2 text-[13px] font-medium text-muted-foreground">
              <Keyboard className="size-3.5" />
              Keybindings
            </h2>
            <p className="mt-1 max-w-xl text-[12px] leading-relaxed text-muted-foreground/75">
              Click a shortcut to re-record it. Bindings save to this browser automatically.
            </p>
          </div>
          <div className="flex min-h-7 shrink-0 items-center gap-1">
            <ExpandableHeaderSearch
              query={query}
              onChange={setQuery}
              isOpen={isSearchOpen}
              onOpenChange={setIsSearchOpen}
              inputRef={searchInputRef}
              collapsedAccessory={bindingsCount}
            />
            <IconButton title="Add keybinding" onClick={() => setIsAddingBinding(true)}>
              <Plus className="size-3.5" />
            </IconButton>
            <IconButton title="Reset all to defaults" onClick={resetAll}>
              <RotateCcw className="size-3.5" />
            </IconButton>
            <IconButton title="Export custom bindings (JSON)" onClick={exportJson}>
              <Download className="size-3.5" />
            </IconButton>
            <IconButton
              title="Import bindings (JSON)"
              onClick={() => fileInputRef.current?.click()}
            >
              <Upload className="size-3.5" />
            </IconButton>
            <input
              ref={fileInputRef}
              type="file"
              accept="application/json,.json"
              className="hidden"
              onChange={(e) => {
                importJson(e.target.files?.[0]);
                e.target.value = '';
              }}
            />
          </div>
        </div>

        <div className="overflow-hidden rounded-xl border border-border/60 bg-card/40">
          <div className="flex items-center gap-2 border-b border-border/50 px-4 py-2 text-xs leading-normal text-muted-foreground">
            <TriangleAlert className="size-3.5 shrink-0 text-amber-400/80" aria-hidden />
            <span>
              Some shortcuts may be claimed by the browser before Mira sees them (new-tab,
              new-window and devtools chords can&apos;t be overridden).
            </span>
          </div>
          {importError ? (
            <div className="flex items-center gap-2 border-b border-destructive/30 bg-destructive/10 px-4 py-2 text-xs text-destructive">
              <X className="size-3.5 shrink-0" aria-hidden />
              <span>{importError}</span>
              <button
                type="button"
                onClick={() => setImportError(null)}
                className="ml-auto flex items-center gap-1 rounded px-1.5 py-0.5 hover:bg-destructive/20"
              >
                <Check className="size-3" /> Dismiss
              </button>
            </div>
          ) : null}

          <KeybindingsList
            rows={rows}
            allRows={rows}
            commandOptions={commandOptions}
            variables={whenVariables}
            isAddingBinding={isAddingBinding}
            onCancelAdd={() => setIsAddingBinding(false)}
            onSave={saveKeybinding}
            onReset={resetRow}
            onRemove={removeRow}
          />
        </div>
      </section>
    </div>
  );
}

export { KeyboardShortcutsSettings as KeybindingsSection } from './KeyboardShortcutsSettings';
