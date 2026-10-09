import { shortcutLabelForCommand, useKeybindings } from '@/lib/keybindings';
import { cn } from '@/lib/utils';
import { useEffect, useMemo, useRef, useState } from 'react';
import { agentRequestHeadline, agentRequestOf } from '../../lib/agentRequest';
import { partsNeedingApproval, splitShellCommand } from '../../lib/shellParts';
import type { ApprovalScope, DiffLine } from '../../types';
import { ApprovalChoices } from '../ApprovalDialog';
import { infoFor } from '../tools/toolInfo';
import { PendingApproval } from './types';
export function ApprovalDetails({
  tool,
  args,
  fallback,
}: {
  tool: string;
  args: Record<string, unknown> | null;
  fallback: string;
}) {
  const command = typeof args?.command === 'string' ? args.command : null;
  if (command) {
    return (
      <pre className="m-0 max-h-[22vh] overflow-auto rounded-md border border-border/35 bg-mira-elev1/45 px-3 py-2 font-mono text-[12px] leading-relaxed">
        {command
          .replace(/\\n/g, '\n')
          .split('\n')
          .map((line, i) => (
            <div key={i} className="flex gap-2 whitespace-pre-wrap break-all">
              <span className="shrink-0 select-none text-muted-foreground/60">
                {i === 0 ? '$' : ' '}
              </span>
              <span className="text-foreground/90">{line || ' '}</span>
            </div>
          ))}
      </pre>
    );
  }
  const rows = approvalRows(tool, args);
  if (rows.length > 0) {
    return (
      <div className="rounded-md border border-border/35 bg-mira-elev1/45 px-3 py-2 text-[12.5px]">
        <dl className="grid grid-cols-[auto_minmax(0,1fr)] gap-x-3 gap-y-1.5">
          {rows.map((row) => (
            <div key={row.label} className="contents">
              <dt className="select-none text-muted-foreground/65">{row.label}</dt>
              <dd className="min-w-0 break-words font-mono text-foreground/85">{row.value}</dd>
            </div>
          ))}
        </dl>
      </div>
    );
  }
  return (
    <pre className="m-0 max-h-[22vh] overflow-auto whitespace-pre-wrap rounded-md bg-background/55 px-3 py-2 font-mono text-xs text-muted-foreground">
      {fallback}
    </pre>
  );
}

export function approvalRows(
  tool: string,
  args: Record<string, unknown> | null,
): { label: string; value: string }[] {
  if (!args) return [];
  const rows: { label: string; value: string }[] = [];
  const add = (label: string, value: unknown) => {
    if (typeof value !== 'string' && typeof value !== 'number' && typeof value !== 'boolean')
      return;
    const text = String(value).trim();
    if (text) rows.push({ label, value: text.length > 160 ? `${text.slice(0, 160)}…` : text });
  };
  const lower = tool.toLowerCase();
  if (lower.includes('edit') || lower.includes('write') || lower.includes('file')) {
    add('file', args.path ?? args.file_path ?? args.filePath ?? args.file);
    return rows;
  }
  if (lower.includes('fetch')) add('url', args.url);
  if (lower.includes('search') || lower.includes('grep')) add('query', args.query ?? args.pattern);
  add('target', args.path ?? args.file_path ?? args.url ?? args.query ?? args.pattern);
  return rows;
}

export function safeJson(text: string): Record<string, unknown> | null {
  try {
    const parsed = JSON.parse(text || '{}');
    return parsed && typeof parsed === 'object' && !Array.isArray(parsed) ? parsed : null;
  } catch {
    return null;
  }
}

export function ApprovalQueue({
  approvals,
  activeId,
  onFocus,
  onDecide,
}: {
  approvals: PendingApproval[];
  activeId?: string;
  onFocus: (id: string) => void;
  onDecide: (id: string, allow: boolean) => void;
}) {
  const [excluded, setExcluded] = useState<Set<string>>(new Set());
  const [now, setNow] = useState(Date.now());
  const firstSeen = useRef<Record<string, number>>({});
  useEffect(() => {
    const timer = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(timer);
  }, []);
  const groups = new Map<string, PendingApproval[]>();
  for (const approval of approvals) {
    firstSeen.current[approval.callId] ??= Date.now();
    const agent = agentRequestOf(approval.call);
    const args = agent?.input ?? safeJson(approval.call.function.arguments) ?? {};
    const tool = agent?.tool ?? approval.call.function.name;
    const path =
      typeof args.path === 'string'
        ? args.path
        : typeof args.file_path === 'string'
          ? args.file_path
          : '';
    const folder = path.includes('/') ? path.slice(0, path.lastIndexOf('/') + 1) : '';
    const key = `${tool}${folder ? ` in ${folder}` : ''}`;
    groups.set(key, [...(groups.get(key) ?? []), approval]);
  }
  const selected = approvals.filter((approval) => !excluded.has(approval.callId));
  return (
    <section
      aria-label="Approval queue"
      className="mb-2 rounded-xl border border-border/70 bg-background/40 p-2"
    >
      <div className="mb-2 flex items-center justify-between gap-2">
        <span className="text-xs font-medium">{approvals.length} pending approvals</span>
        <button
          type="button"
          onClick={() =>
            setExcluded(
              selected.length === approvals.length
                ? new Set(approvals.map((item) => item.callId))
                : new Set(),
            )
          }
          className="text-[11px] text-muted-foreground"
        >
          {selected.length === approvals.length ? 'Deselect all' : 'Select all'}
        </button>
      </div>
      <div className="max-h-48 overflow-auto space-y-2">
        {Array.from(groups, ([label, items]) => (
          <div key={label}>
            <p className="px-1 text-[11px] text-muted-foreground">
              {items.length} × {label}
            </p>
            {items.map((item) => {
              const seconds = Math.max(
                0,
                Math.floor((now - (item.startedAt ?? firstSeen.current[item.callId])) / 1000),
              );
              const agent = agentRequestOf(item.call);
              const args = agent?.input ?? safeJson(item.call.function.arguments) ?? {};
              const summary = String(
                args.command ??
                  args.path ??
                  args.file_path ??
                  args.query ??
                  item.call.function.name,
              );
              return (
                <div
                  key={item.callId}
                  className={cn(
                    'flex items-center gap-2 rounded-lg px-2 py-1.5',
                    activeId === item.callId && 'bg-mira-blue/10',
                  )}
                >
                  <input
                    type="checkbox"
                    data-approval-queue="true"
                    onFocus={() => onFocus(item.callId)}
                    aria-label={`Select ${summary}`}
                    checked={!excluded.has(item.callId)}
                    onChange={(event) =>
                      setExcluded((previous) => {
                        const next = new Set(previous);
                        if (event.target.checked) next.delete(item.callId);
                        else next.add(item.callId);
                        return next;
                      })
                    }
                  />
                  <button
                    type="button"
                    aria-pressed={activeId === item.callId}
                    onClick={() => onFocus(item.callId)}
                    onFocus={() => onFocus(item.callId)}
                    className="min-w-0 flex-1 truncate text-left font-mono text-[11.5px]"
                    title={summary}
                  >
                    {summary}
                  </button>
                  <span className="shrink-0 text-[10px] tabular-nums text-muted-foreground">
                    {seconds < 60 ? `${seconds}s` : `${Math.floor(seconds / 60)}m ${seconds % 60}s`}
                  </span>
                </div>
              );
            })}
          </div>
        ))}
      </div>
      <div className="mt-2 flex justify-end gap-2">
        <button
          type="button"
          disabled={!selected.length}
          onClick={() => selected.forEach((item) => onDecide(item.callId, false))}
          className="rounded-full border border-border px-4 py-2 text-xs font-medium disabled:opacity-40"
        >
          Deny {selected.length === approvals.length ? 'all' : 'selected'} ({selected.length})
        </button>
        <button
          type="button"
          disabled={!selected.length}
          onClick={() => selected.forEach((item) => onDecide(item.callId, true))}
          className="rounded-full bg-mira-blue px-4 py-2 text-xs font-medium text-mira-on-accent disabled:opacity-40"
        >
          Approve {selected.length === approvals.length ? 'all' : 'selected'} ({selected.length})
        </button>
      </div>
    </section>
  );
}

/** Tool approval rendered directly inside the Composer. */
export function EmbeddedApprovalCard({
  approval,
  openRuleEditor,
  onRuleEditorCancel,
  onDecide,
  queued = 1,
  position = 1,
}: {
  approval: PendingApproval;
  openRuleEditor?: boolean;
  onRuleEditorCancel?: () => void;
  onDecide: (allow: boolean, scope?: ApprovalScope, rules?: string[]) => void;
  /** Requests waiting, this one included. */
  queued?: number;
  position?: number;
  onAllowAll?: () => void;
}) {
  const { call, preview } = approval;
  const [editingRule, setEditingRule] = useState(false);
  useEffect(() => {
    if (openRuleEditor) setEditingRule(true);
  }, [openRuleEditor]);
  const [ruleDraft, setRuleDraft] = useState<string | null>(null);
  const rules = (ruleDraft ?? approval.rulePreview?.rules.join('\n') ?? '')
    .split('\n')
    .map((rule) => rule.trim())
    .filter(Boolean);
  // The keys App listens for, as the user has them bound.
  const keybindings = useKeybindings();
  const allowKey =
    shortcutLabelForCommand(keybindings, 'approval.accept', { context: { approvalOpen: true } }) ??
    'Y';
  const denyKey =
    shortcutLabelForCommand(keybindings, 'approval.reject', { context: { approvalOpen: true } }) ??
    'N';
  // One card for every approval. An external agent's request only differs
  // in how it is described: its own tool name and input, and why it asks.
  const agent = useMemo(() => agentRequestOf(call), [call]);
  const info = infoFor(agent ? agentToolAsMira(agent.tool) : call.function.name);
  const Icon = info.Icon;
  const isDiffTool = call.function.name === 'write_file' || call.function.name === 'edit_file';
  const prettyArgs = useMemo(() => {
    if (agent) {
      const headline = agentRequestHeadline(agent);
      return headline ?? JSON.stringify(agent.input, null, 2);
    }
    try {
      return JSON.stringify(JSON.parse(call.function.arguments), null, 2);
    } catch {
      return call.function.arguments;
    }
  }, [call.function.arguments, agent]);
  // A compound shell command, as the operations it is made of — one long
  // line hides which part is the one worth a second look.
  const command = useMemo(() => {
    if (agent)
      return agent.tool === 'Bash' && typeof agent.input.command === 'string'
        ? agent.input.command
        : null;
    if (call.function.name !== 'bash') return null;
    try {
      const c = JSON.parse(call.function.arguments)?.command;
      return typeof c === 'string' ? c : null;
    } catch {
      return null;
    }
  }, [agent, call]);
  const parts = useMemo(() => (command ? splitShellCommand(command) : []), [command]);
  // Which parts are the reason for asking: the agent says so in its
  // reason; for Mira's own commands the server's policy names them.
  const flagged = useMemo(
    () => (agent ? partsNeedingApproval(agent.reason) : (approval.needs ?? [])),
    [agent, approval.needs],
  );

  return (
    <div className="flex flex-col">
      <div className="flex items-center gap-2 px-1.5 pt-1 pb-2 font-mono text-[12.5px]">
        <span className="shrink-0 text-mira-tool">
          <Icon className="size-3.5" />
        </span>
        <span className="font-medium text-foreground truncate min-w-0">
          {agent ? agent.tool : info.verbCont}{' '}
          <span className="font-normal text-muted-foreground">
            {isDiffTool && preview?.path ? preview.path.split('/').slice(-2).join('/') : ''}
          </span>
        </span>
        {preview && (
          <span className="rounded-full border border-border bg-background px-1.5 py-0.5 text-[10.5px] uppercase tracking-wider text-muted-foreground shrink-0">
            {preview.kind}
          </span>
        )}
        <span className="ml-auto shrink-0 rounded-full bg-secondary px-2 py-0.5 text-[10.5px] font-semibold uppercase tracking-wider text-muted-foreground">
          {queued > 1 ? `${position} of ${queued} awaiting` : 'awaiting approval'}
        </span>
      </div>

      <div className="max-h-[35vh] overflow-auto">
        {preview ? (
          <div className="diff">
            {preview.lines.map((line: DiffLine, i: number) => {
              if (line.tag === 'hunkgap')
                return (
                  <div key={i} className="diff-line hunk">
                    ···
                  </div>
                );
              const cls = line.tag === 'add' ? 'add' : line.tag === 'del' ? 'del' : 'ctx';
              const prefix = line.tag === 'add' ? '+' : line.tag === 'del' ? '-' : ' ';
              return (
                <div key={i} className={`diff-line ${cls}`}>
                  <span className="prefix">{prefix}</span>
                  <span className="text">{line.text}</span>
                </div>
              );
            })}
          </div>
        ) : parts.length > 1 ? (
          <ol className="m-0 max-h-[22vh] list-none space-y-0.5 overflow-auto rounded-md bg-background/60 px-2 py-1.5 font-mono text-xs">
            {parts.map((p, i) => {
              const hot = flagged.some((f) => f.includes(p.text) || p.text.includes(f));
              return (
                <li key={i} className="flex items-start gap-2">
                  <span className="w-4 shrink-0 select-none text-right text-muted-foreground/45">
                    {i + 1}
                  </span>
                  <span
                    className={cn(
                      'min-w-0 flex-1 whitespace-pre-wrap break-all',
                      hot ? 'text-foreground' : 'text-muted-foreground',
                    )}
                  >
                    {p.text}
                    {p.joiner && (
                      <span className="ml-1.5 text-muted-foreground/40">{p.joiner}</span>
                    )}
                  </span>
                  {hot && (
                    <span
                      className="mt-1 size-1.5 shrink-0 rounded-full bg-mira-warn"
                      title="Needs approval"
                    />
                  )}
                </li>
              );
            })}
          </ol>
        ) : (
          <ApprovalDetails
            tool={agent ? agent.tool : call.function.name}
            args={agent ? agent.input : safeJson(call.function.arguments)}
            fallback={prettyArgs}
          />
        )}
      </div>

      {flagged.length > 0 ? (
        <div className="px-1.5 pt-2 text-[11.5px] text-muted-foreground">
          <div className="text-foreground/75">
            {flagged.length === 1
              ? 'This part needs approval:'
              : `These ${flagged.length} parts need approval:`}
          </div>
          <ul className="mt-1 max-h-24 space-y-0.5 overflow-auto font-mono text-[11px]">
            {flagged.map((f, i) => (
              <li key={i} className="flex gap-1.5">
                <span className="mt-[5px] size-1.5 shrink-0 rounded-full bg-mira-warn" />
                <span className="min-w-0 break-all">{f}</span>
              </li>
            ))}
          </ul>
        </div>
      ) : agent?.reason ? (
        <div className="px-1.5 pt-2 text-[11.5px] text-muted-foreground">
          <span className="text-foreground/75">Why it asks:</span> {agent.reason}
        </div>
      ) : null}

      <div className="border-t border-border/30" />

      {/* One shared row of decisions. The composer's inline card and the
          approval modal are the same question asked in two places, and they
          were rendering their own buttons — which is how "Allow" ends up
          meaning slightly different things depending on which appeared. */}
      <div className="px-1.5 py-2">
        <ApprovalChoices
          choices={[
            { id: 'deny', label: 'Deny', title: 'Deny', kbd: denyKey },
            {
              id: 'always',
              label: agent ? 'Always allow via agent' : 'Always allow',
              title: agent
                ? 'Use the agent’s own permission scope'
                : 'Preview the saved permission rule',
            },
            {
              id: 'session',
              label: 'Allow for this chat',
              title: 'Stop asking about this until the chat ends',
            },
            {
              id: 'once',
              label: 'Allow',
              primary: true,
              title: 'Allow this one call',
              kbd: allowKey,
            },
          ]}
          onChoose={(id: string) => {
            if (id === 'always' && !agent) setEditingRule(true);
            else if (id === 'deny') onDecide(false);
            else onDecide(true, id === 'always' ? 'always' : id === 'session' ? 'session' : 'once');
          }}
        />
      </div>
      {editingRule && (
        <div className="space-y-2 rounded-lg border border-border bg-background/60 p-3">
          <p className="text-xs font-medium">Always allow · saved policy rules</p>
          <p className="text-[11px] text-muted-foreground">
            One rule per line. These exact patterns will be saved for future calls.
          </p>
          {approval.rulePreview?.error && (
            <p role="alert" className="text-xs text-amber-600 dark:text-amber-400">
              {approval.rulePreview.error}
            </p>
          )}
          {!approval.rulePreview && (
            <p className="text-xs text-muted-foreground">Loading the exact policy rules…</p>
          )}
          <textarea
            aria-label="Permission rule patterns"
            value={ruleDraft ?? approval.rulePreview?.rules.join('\n') ?? ''}
            onChange={(event) => setRuleDraft(event.target.value)}
            className="w-full rounded-md border border-border bg-background p-2 font-mono text-xs"
            rows={3}
          />
          <div className="flex justify-end gap-2">
            <button
              type="button"
              onClick={() => {
                setEditingRule(false);
                onRuleEditorCancel?.();
              }}
              className="rounded-full px-4 py-2 text-xs font-medium hover:bg-secondary"
            >
              Cancel
            </button>
            <button
              type="button"
              disabled={
                !rules.length || !approval.rulePreview || (!!agent && !!approval.rulePreview.error)
              }
              onClick={() => onDecide(true, 'always', rules)}
              className="rounded-full bg-mira-blue px-4 py-2 text-xs font-medium text-mira-on-accent disabled:opacity-40"
            >
              Save rule & allow
            </button>
          </div>
        </div>
      )}
      <div className="px-1.5 pb-1.5 text-right text-[10.5px] text-muted-foreground/60 touch:hidden">
        <kbd className="rounded bg-secondary/70 px-1 py-0.5 font-mono text-[10px]">y</kbd> allow ·{' '}
        <kbd className="rounded bg-secondary/70 px-1 py-0.5 font-mono text-[10px]">n</kbd> deny
      </div>
    </div>
  );
}

/** An agent's tool name as the Mira tool it corresponds to, for the icon
 *  and verb. Unknown tools keep their own name. */
export function agentToolAsMira(tool: string): string {
  const map: Record<string, string> = {
    Bash: 'bash',
    Read: 'read_file',
    Write: 'write_file',
    Edit: 'edit_file',
    MultiEdit: 'edit_file',
    Glob: 'glob',
    Grep: 'grep',
    WebFetch: 'web_fetch',
    WebSearch: 'web_search',
  };
  return map[tool] ?? tool;
}
