import { type KeybindingWhenNode } from '@/lib/keybindings';
import { cn } from '@/lib/utils';
import { Minus, Plus, TriangleAlert, X } from 'lucide-react';
import { useMemo, useState, type ReactNode } from 'react';
import { Button } from '../../ui/button';
import { SectionInput } from '../../ui/input';
import { Select } from '../../ui/select';
import {
  DEFAULT_WHEN_VARIABLE,
  isKnownWhenVariable,
  parseWhenExpressionDraft,
  unknownWhenVariables,
  whenAstToExpression,
  whenNodeRemoveLabel,
  type WhenVariableOption,
} from '../keybindingsLogic';
import { IconButton } from './KeybindingRows';
export type BooleanOperator = 'and' | 'or';

export function flattenWhenChildren(
  node: KeybindingWhenNode,
  operator: BooleanOperator,
): KeybindingWhenNode[] {
  if (node.type !== operator) return [node];
  return [
    ...flattenWhenChildren(node.left, operator),
    ...flattenWhenChildren(node.right, operator),
  ];
}

export function buildWhenExpressionGroup(
  children: readonly KeybindingWhenNode[],
  operator: BooleanOperator,
): KeybindingWhenNode | undefined {
  const first = children[0];
  if (!first) return undefined;
  return children.slice(1).reduce<KeybindingWhenNode>(
    (left, right) => ({
      type: operator,
      left,
      right,
    }),
    first,
  );
}

export function conditionParts(
  node: KeybindingWhenNode,
): { identifier: string; negated: boolean } | null {
  if (node.type === 'identifier') return { identifier: node.name, negated: false };
  if (node.type === 'not' && node.node.type === 'identifier') {
    return { identifier: node.node.name, negated: true };
  }
  return null;
}

export function setConditionIdentifier(
  node: KeybindingWhenNode,
  identifier: string,
): KeybindingWhenNode {
  const parts = conditionParts(node);
  if (!parts) return node;
  const next: KeybindingWhenNode = { type: 'identifier', name: identifier };
  return parts.negated ? { type: 'not', node: next } : next;
}

export function setConditionNegated(
  node: KeybindingWhenNode,
  negated: boolean,
): KeybindingWhenNode {
  const parts = conditionParts(node);
  if (!parts) return negated ? { type: 'not', node } : node;
  const identifier: KeybindingWhenNode = { type: 'identifier', name: parts.identifier };
  return negated ? { type: 'not', node: identifier } : identifier;
}

export function defaultWhenCondition(): KeybindingWhenNode {
  return { type: 'identifier', name: DEFAULT_WHEN_VARIABLE };
}

export function defaultWhenGroup(operator: BooleanOperator = 'and'): KeybindingWhenNode {
  return {
    type: operator,
    left: defaultWhenCondition(),
    right: { type: 'not', node: defaultWhenCondition() },
  };
}

export function NotToggle({
  pressed,
  onChange,
  label,
}: {
  pressed: boolean;
  onChange: (pressed: boolean) => void;
  label: string;
}) {
  return (
    <button
      type="button"
      aria-pressed={pressed}
      aria-label={label}
      onClick={() => onChange(!pressed)}
      className={cn(
        'min-w-10 shrink-0 rounded-md border px-2 py-1 text-[12px] font-medium transition-colors',
        pressed
          ? 'border-mira-blue/50 bg-mira-blue/15 text-mira-blue'
          : 'border-border/70 text-muted-foreground hover:bg-accent hover:text-foreground',
      )}
    >
      Not
    </button>
  );
}

/** Warning glyph with a hover explanation; the one owner of that affordance here. */
export function WarningIcon({ label, children }: { label: string; children: ReactNode }) {
  return (
    <span className="group/warn relative inline-flex shrink-0 items-center justify-center">
      <span
        tabIndex={0}
        aria-label={label}
        className="inline-flex size-5 items-center justify-center rounded-sm text-amber-400 outline-none transition-colors hover:bg-amber-400/10 focus-visible:ring-2 focus-visible:ring-amber-400/25"
      >
        <TriangleAlert className="size-3.5" />
      </span>
      <span className="tooltip pointer-events-none absolute bottom-full left-1/2 z-50 mb-1.5 hidden w-56 -translate-x-1/2 group-hover/warn:block">
        {children}
      </span>
    </span>
  );
}

export function UnknownWhenVariableWarning({
  identifiers,
}: {
  identifiers: ReadonlyArray<string>;
}) {
  if (identifiers.length === 0) return null;
  const label =
    identifiers.length === 1
      ? `Unknown condition: ${identifiers[0]}`
      : `Unknown conditions: ${identifiers.join(', ')}`;

  return (
    <WarningIcon label={label}>
      Mira does not recognize this condition yet. It can still be saved, but it may not match unless
      the runtime provides it.
    </WarningIcon>
  );
}

export function KeybindingConflictWarning({ labels }: { labels: ReadonlyArray<string> }) {
  if (labels.length === 0) return null;
  const description =
    labels.length === 1
      ? `Conflicts with ${labels[0]}.`
      : `Conflicts with ${labels.slice(0, 3).join(', ')}${labels.length > 3 ? ', and more' : ''}.`;

  return (
    <WarningIcon label={description}>
      {description} The most recent matching binding wins when both conditions can apply.
    </WarningIcon>
  );
}

export function WhenVariableSelect({
  value,
  variables,
  unknownIdentifiers,
  onChange,
}: {
  value: string;
  variables: ReadonlyArray<WhenVariableOption>;
  unknownIdentifiers?: ReadonlyArray<string>;
  onChange: (value: string) => void;
}) {
  const options = variables.some((option) => option === value)
    ? variables.map((v) => ({ value: v, label: v }))
    : [{ value, label: value }, ...variables.map((v) => ({ value: v, label: v }))];

  return (
    <div className="flex min-w-0 flex-1 items-center gap-1.5">
      <Select
        value={value}
        onChange={(nextValue) => nextValue && onChange(nextValue)}
        options={options}
        placeholder="Condition"
        className="h-8 text-[12.5px]"
      />
      {unknownIdentifiers && unknownIdentifiers.length > 0 ? (
        <UnknownWhenVariableWarning identifiers={unknownIdentifiers} />
      ) : null}
    </div>
  );
}

export function WhenExpressionRemoveButton({
  label,
  className,
  onRemove,
}: {
  label: string;
  className?: string | undefined;
  onRemove: () => void;
}) {
  return (
    <IconButton title={label} onClick={onRemove} className={cn('size-7', className)}>
      <Minus aria-hidden className="size-3.5" />
    </IconButton>
  );
}

export function WhenExpressionNodeEditor({
  node,
  variables,
  depth = 0,
  onChange,
  onRemove,
}: {
  node: KeybindingWhenNode;
  variables: ReadonlyArray<WhenVariableOption>;
  depth?: number;
  onChange: (node: KeybindingWhenNode) => void;
  onRemove?: () => void;
}) {
  const condition = conditionParts(node);

  if (condition) {
    const unknownIdentifiers = isKnownWhenVariable(condition.identifier)
      ? []
      : [condition.identifier];

    return (
      <div className="flex items-center gap-2 rounded-md border border-border/70 bg-background/60 px-2 py-2">
        <NotToggle
          pressed={condition.negated}
          onChange={(pressed) => onChange(setConditionNegated(node, pressed))}
          label={`Negate ${condition.identifier}`}
        />
        <WhenVariableSelect
          value={condition.identifier}
          variables={variables}
          unknownIdentifiers={unknownIdentifiers}
          onChange={(value) => onChange(setConditionIdentifier(node, value))}
        />
        {onRemove ? (
          <WhenExpressionRemoveButton
            label={whenNodeRemoveLabel(node, depth)}
            onRemove={onRemove}
          />
        ) : null}
      </div>
    );
  }

  if (node.type === 'not') {
    return (
      <div
        className={cn(
          'space-y-2 rounded-lg border border-border/70 bg-muted/20 p-2',
          depth > 0 && 'border-border/50 bg-background/50',
        )}
      >
        <div className="flex items-center gap-2">
          <NotToggle
            pressed
            onChange={(pressed) => onChange(pressed ? node : node.node)}
            label="Negate group"
          />
          {onRemove ? (
            <WhenExpressionRemoveButton
              label={whenNodeRemoveLabel(node, depth)}
              className="ml-auto"
              onRemove={onRemove}
            />
          ) : null}
        </div>
        <div className="relative pl-4">
          <span className="absolute bottom-0 left-1.5 top-0 w-px bg-border/70" aria-hidden />
          <span className="absolute left-1.5 top-4 h-px w-2.5 bg-border/70" aria-hidden />
          <WhenExpressionNodeEditor
            node={node.node}
            variables={variables}
            depth={depth + 1}
            onChange={(next) => onChange({ type: 'not', node: next })}
          />
        </div>
      </div>
    );
  }

  const operator: BooleanOperator = node.type === 'or' ? 'or' : 'and';
  const children = flattenWhenChildren(node, operator);
  const childKeyCounts = new Map<string, number>();
  const childEntries = children.map((child) => {
    const baseKey = `${child.type}-${whenAstToExpression(child)}`;
    const count = childKeyCounts.get(baseKey) ?? 0;
    childKeyCounts.set(baseKey, count + 1);
    return { child, key: count === 0 ? baseKey : `${baseKey}-${count}` };
  });

  const updateChild = (target: KeybindingWhenNode, next: KeybindingWhenNode) => {
    let didUpdate = false;
    const nextChildren = children.map((child) => {
      if (!didUpdate && child === target) {
        didUpdate = true;
        return next;
      }
      return child;
    });
    const nextNode = buildWhenExpressionGroup(nextChildren, operator);
    if (nextNode) onChange(nextNode);
  };

  const removeChild = (target: KeybindingWhenNode) => {
    let didRemove = false;
    const nextChildren = children.filter((child) => {
      if (!didRemove && child === target) {
        didRemove = true;
        return false;
      }
      return true;
    });
    const nextNode = buildWhenExpressionGroup(nextChildren, operator);
    if (nextNode) {
      onChange(nextNode);
    } else {
      onChange(defaultWhenCondition());
    }
  };

  const setOperator = (nextOperator: BooleanOperator) => {
    if (nextOperator === operator) return;
    const nextNode = buildWhenExpressionGroup(children, nextOperator);
    if (nextNode) onChange(nextNode);
  };

  const addCondition = () => {
    const nextNode = buildWhenExpressionGroup([...children, defaultWhenCondition()], operator);
    if (nextNode) onChange(nextNode);
  };

  const addGroup = () => {
    const nestedOperator: BooleanOperator = operator === 'and' ? 'or' : 'and';
    const group: KeybindingWhenNode = {
      type: nestedOperator,
      left: defaultWhenCondition(),
      right: { type: 'not', node: defaultWhenCondition() },
    };
    const nextNode = buildWhenExpressionGroup([...children, group], operator);
    if (nextNode) onChange(nextNode);
  };

  return (
    <div
      className={cn(
        'space-y-2 rounded-lg border border-border/60 bg-muted/10 p-2',
        depth > 0 && 'border-border/70 bg-background/55',
      )}
    >
      <div className="flex flex-wrap items-center gap-2">
        <Select
          value={operator}
          onChange={(value) => setOperator(value as BooleanOperator)}
          options={[
            { value: 'and', label: 'and' },
            { value: 'or', label: 'or' },
          ]}
          className="h-8 w-24 text-[12.5px]"
        />
        <Button type="button" variant="outline" size="sm" className="h-8" onClick={addCondition}>
          <Plus className="size-3.5" />
          Condition
        </Button>
        <Button type="button" variant="outline" size="sm" className="h-8" onClick={addGroup}>
          <Plus className="size-3.5" />
          Group
        </Button>
        {onRemove ? (
          <WhenExpressionRemoveButton
            label={whenNodeRemoveLabel(node, depth)}
            className="ml-auto"
            onRemove={onRemove}
          />
        ) : null}
      </div>
      <div className="space-y-2">
        {childEntries.map(({ child, key }) => (
          <div key={key} className="relative pl-4">
            <span
              className={cn(
                'absolute bottom-0 left-1.5 top-0 w-px',
                depth === 0 ? 'bg-border' : 'bg-border/70',
              )}
              aria-hidden
            />
            <span
              className={cn(
                'absolute left-1.5 top-4 h-px w-2.5',
                depth === 0 ? 'bg-border' : 'bg-border/70',
              )}
              aria-hidden
            />
            <WhenExpressionNodeEditor
              node={child}
              variables={variables}
              depth={depth + 1}
              onChange={(next) => updateChild(child, next)}
              onRemove={() => removeChild(child)}
            />
          </div>
        ))}
      </div>
    </div>
  );
}

export function WhenExpressionBuilder({
  value,
  variables,
  onChange,
  onValidityChange,
}: {
  value: KeybindingWhenNode | undefined;
  variables: ReadonlyArray<WhenVariableOption>;
  onChange: (value: KeybindingWhenNode | undefined) => void;
  onValidityChange?: (valid: boolean) => void;
}) {
  const expression = whenAstToExpression(value);
  const [expressionDraft, setExpressionDraft] = useState(expression);
  const parseResult = useMemo(() => parseWhenExpressionDraft(expressionDraft), [expressionDraft]);
  const parseError = parseResult.ok ? null : parseResult.message;
  const unknownIdentifiers = parseResult.ok ? unknownWhenVariables(parseResult.value) : [];

  const updateExpressionDraft = (nextExpression: string) => {
    setExpressionDraft(nextExpression);
    const nextResult = parseWhenExpressionDraft(nextExpression);
    onValidityChange?.(nextResult.ok);
    if (nextResult.ok) {
      onChange(nextResult.value);
    }
  };

  const updateExpressionValue = (nextValue: KeybindingWhenNode | undefined) => {
    setExpressionDraft(whenAstToExpression(nextValue));
    onValidityChange?.(true);
    onChange(nextValue);
  };

  const addRootCondition = () => {
    if (!value) {
      updateExpressionValue(defaultWhenCondition());
      return;
    }
    updateExpressionValue({ type: 'and', left: value, right: defaultWhenCondition() });
  };

  const addRootGroup = () => {
    const group = defaultWhenGroup('or');
    if (!value) {
      updateExpressionValue(group);
      return;
    }
    updateExpressionValue({ type: 'and', left: value, right: group });
  };

  return (
    <div className="w-[min(34rem,calc(100vw-2rem))] space-y-3">
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0">
          <div className="text-sm font-medium text-foreground">When</div>
        </div>
        <div className="flex shrink-0 items-center gap-2">
          <Button
            type="button"
            variant="outline"
            size="sm"
            className="h-8"
            onClick={addRootCondition}
          >
            <Plus className="size-3.5" />
            Condition
          </Button>
          <Button type="button" variant="outline" size="sm" className="h-8" onClick={addRootGroup}>
            <Plus className="size-3.5" />
            Group
          </Button>
        </div>
      </div>

      <div className="space-y-1.5">
        <div className="relative">
          <SectionInput
            value={expressionDraft}
            onChange={(event) => updateExpressionDraft(event.currentTarget.value)}
            placeholder="Always"
            aria-invalid={Boolean(parseError)}
            aria-label="When expression"
            className="h-8 font-mono text-[12.5px]"
          />
          {unknownIdentifiers.length > 0 ? (
            <span className="absolute right-2 top-1/2 -translate-y-1/2">
              <UnknownWhenVariableWarning identifiers={unknownIdentifiers} />
            </span>
          ) : null}
        </div>
        {parseError ? (
          <div className="flex items-center gap-1.5 text-[11px] text-destructive">
            <X className="size-3.5" />
            {parseError}
          </div>
        ) : null}
      </div>

      <div className="relative">
        {value ? (
          <WhenExpressionNodeEditor
            node={value}
            variables={variables}
            onChange={updateExpressionValue}
            onRemove={() => updateExpressionValue(undefined)}
          />
        ) : (
          <div className="rounded-md border border-dashed border-border/80 bg-muted/15 p-3">
            <div className="flex flex-wrap gap-2">
              <Button type="button" size="sm" className="h-8" onClick={addRootCondition}>
                <Plus className="size-3.5" />
                Condition
              </Button>
              <Button
                type="button"
                variant="outline"
                size="sm"
                className="h-8"
                onClick={addRootGroup}
              >
                <Plus className="size-3.5" />
                Group
              </Button>
            </div>
          </div>
        )}
        {parseError ? (
          <div className="pointer-events-none absolute inset-0 flex items-center justify-center rounded-lg border border-destructive/30 bg-background/75 p-4 text-center text-xs text-destructive">
            Fix the expression above to continue editing visually.
          </div>
        ) : null}
      </div>
    </div>
  );
}
