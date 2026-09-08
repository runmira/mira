import { useEffect, useState } from 'react';
import {
  ArrowClockwise,
  ArrowSquareOut,
  CheckCircle,
  DotsThree,
  Info,
  PencilSimple,
  Plus,
  PuzzlePiece,
  Trash,
  Warning,
  WarningCircle,
  X,
} from '@phosphor-icons/react';
import type {
  McpHttpConfig,
  McpListView,
  McpServerConfig,
  McpServerView,
  McpStdioConfig,
  McpToolInfo,
} from '../api';
import { listMcp, putMcp } from '../api';
import { Button } from '@/components/ui/button';
import { Dialog, DialogContent } from '@/components/ui/dialog';
import { Input } from '@/components/ui/input';
import { Popover, PopoverContent, PopoverTrigger } from '@/components/ui/popover';
import { cn } from '@/lib/utils';

/** Full-pane Plugins view. Replaces the chat transcript+composer while
 *  the user is managing MCP servers. Mirrors the sidebar's mainView state
 *  in App.tsx; there's no local routing here. */
export function PluginsPanel() {
  const [view, setView] = useState<McpListView | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [saveError, setSaveError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const [editorOpen, setEditorOpen] = useState(false);
  const [editorSeed, setEditorSeed] = useState<{ originalName: string | null; draft: DraftServer } | null>(null);

  const [viewingTools, setViewingTools] = useState<McpServerView | null>(null);
  const [confirming, setConfirming] = useState<McpServerView | null>(null);

  async function refresh() {
    try {
      const v = await listMcp();
      setView(v);
      setLoadError(null);
    } catch (e) {
      setLoadError((e as Error).message);
    }
  }

  useEffect(() => {
    refresh();
  }, []);

  function openAdd() {
    setEditorSeed({ originalName: null, draft: emptyStdioDraft() });
    setSaveError(null);
    setEditorOpen(true);
  }

  function openEdit(s: McpServerView) {
    setEditorSeed({ originalName: s.name, draft: draftFromView(s) });
    setSaveError(null);
    setEditorOpen(true);
  }

  async function saveDraft(originalName: string | null, draft: DraftServer) {
    if (!view) return;
    const name = draft.name.trim();
    if (!name) {
      setSaveError('Name is required.');
      return;
    }
    // Build the wire config from the draft.
    let cfg: McpServerConfig;
    if (draft.kind === 'stdio') {
      if (!draft.command.trim()) {
        setSaveError('Command is required for stdio servers.');
        return;
      }
      const env: Record<string, string> = {};
      for (const { key, value } of draft.env) {
        const k = key.trim();
        if (!k) continue;
        env[k] = value;
      }
      const stdio: McpStdioConfig = {
        command: draft.command.trim(),
        args: draft.args.map((a) => a.trim()).filter((a) => a.length > 0),
        env,
        cwd: draft.cwd.trim() ? draft.cwd.trim() : null,
      };
      cfg = stdio;
    } else {
      if (!draft.url.trim()) {
        setSaveError('URL is required for http servers.');
        return;
      }
      const http: McpHttpConfig = {
        url: draft.url.trim(),
        auth: draft.auth.trim() ? draft.auth.trim() : null,
      };
      cfg = http;
    }

    const next: Record<string, McpServerConfig> = {};
    for (const s of view.servers) {
      if (s.name === originalName) continue; // will be re-added under new name
      if (s.name === name && originalName !== name) {
        setSaveError(`A server named "${name}" already exists.`);
        return;
      }
      next[s.name] = s.config;
    }
    next[name] = cfg;

    setBusy(true);
    setSaveError(null);
    try {
      const v = await putMcp(next);
      setView(v);
      setEditorOpen(false);
    } catch (e) {
      setSaveError((e as Error).message);
    } finally {
      setBusy(false);
    }
  }

  async function removeServer(s: McpServerView) {
    if (!view) return;
    const next: Record<string, McpServerConfig> = {};
    for (const other of view.servers) {
      if (other.name === s.name) continue;
      next[other.name] = other.config;
    }
    setBusy(true);
    try {
      const v = await putMcp(next);
      setView(v);
      setConfirming(null);
    } catch (e) {
      setSaveError((e as Error).message);
    } finally {
      setBusy(false);
    }
  }

  const servers = view?.servers ?? [];

  return (
    <div className="mx-auto flex w-full max-w-3xl flex-col gap-5 px-6 pb-10 pt-6">
      <header className="flex items-start justify-between gap-4">
        <div className="min-w-0">
          <div className="flex items-center gap-2">
            <PuzzlePiece className="size-5 text-mira-purple" weight="fill" />
            <h1 className="text-[19px] font-semibold tracking-tight">Plugins</h1>
          </div>
          <p className="mt-1 text-[13px] text-muted-foreground">
            Model Context Protocol servers. Each server exposes remote tools
            to the agent under the <code className="font-mono">mcp__&lt;server&gt;__*</code> namespace.
          </p>
        </div>
        <div className="flex shrink-0 items-center gap-2">
          <Button
            type="button"
            variant="outline"
            size="sm"
            onClick={refresh}
            title="Reload status"
            className="gap-1.5"
          >
            <ArrowClockwise className="size-3.5" />
            Refresh
          </Button>
          <Button type="button" size="sm" onClick={openAdd} className="gap-1.5">
            <Plus className="size-3.5" weight="bold" />
            Add server
          </Button>
        </div>
      </header>

      {view?.config_path && (
        <div className="flex items-center gap-2 rounded-md border border-border/70 bg-secondary/30 px-3 py-2 text-[12px] text-muted-foreground">
          <Info className="size-3.5 shrink-0" />
          <span className="min-w-0 truncate">
            Saved to <code className="font-mono text-foreground/80">{view.config_path}</code>
          </span>
        </div>
      )}

      {loadError && (
        <div className="rounded-md border border-destructive/40 bg-destructive/10 px-3 py-2 text-[13px] text-destructive">
          {loadError}
        </div>
      )}

      {view && servers.length === 0 && (
        <EmptyState onAdd={openAdd} />
      )}

      <ul className="flex flex-col gap-2.5">
        {servers.map((s) => (
          <ServerCard
            key={s.name}
            server={s}
            onEdit={() => openEdit(s)}
            onRemove={() => setConfirming(s)}
            onViewTools={() => setViewingTools(s)}
          />
        ))}
      </ul>

      {editorOpen && editorSeed && (
        <EditorDialog
          seed={editorSeed}
          busy={busy}
          error={saveError}
          onCancel={() => setEditorOpen(false)}
          onSave={(originalName, draft) => saveDraft(originalName, draft)}
        />
      )}

      {viewingTools && (
        <ToolsDialog
          server={viewingTools}
          onClose={() => setViewingTools(null)}
        />
      )}

      {confirming && (
        <ConfirmRemoveDialog
          server={confirming}
          busy={busy}
          onCancel={() => setConfirming(null)}
          onConfirm={() => removeServer(confirming)}
        />
      )}
    </div>
  );
}

/* ---------- server card ---------- */

function ServerCard({
  server,
  onEdit,
  onRemove,
  onViewTools,
}: {
  server: McpServerView;
  onEdit: () => void;
  onRemove: () => void;
  onViewTools: () => void;
}) {
  const summary = configSummary(server);
  return (
    <li className="group rounded-lg border border-border/70 bg-card/60 transition-colors hover:border-border">
      <div className="flex items-start gap-3 px-4 py-3">
        <StatusDot server={server} />

        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-2">
            <span className="truncate text-[14.5px] font-medium text-foreground">
              {server.name}
            </span>
            <KindBadge kind={server.kind} />
            <StatusPill server={server} />
          </div>
          <div className="mt-1 truncate font-mono text-[12px] text-muted-foreground/90" title={summary}>
            {summary}
          </div>
        </div>

        <RowMenu
          items={[
            {
              label: server.tools.length === 0 ? 'No tools loaded' : `View ${server.tools.length} tool${server.tools.length === 1 ? '' : 's'}`,
              icon: <PuzzlePiece className="size-3.5" />,
              onSelect: onViewTools,
              disabled: server.tools.length === 0,
            },
            {
              label: 'Edit',
              icon: <PencilSimple className="size-3.5" />,
              onSelect: onEdit,
            },
            {
              label: 'Remove',
              icon: <Trash className="size-3.5" />,
              danger: true,
              onSelect: onRemove,
            },
          ]}
        />
      </div>
    </li>
  );
}

function StatusDot({ server }: { server: McpServerView }) {
  const cls = statusColor(server);
  return (
    <span
      className={cn('mt-1.5 size-2 shrink-0 rounded-full', cls)}
      title={statusLabel(server)}
      aria-label={statusLabel(server)}
    />
  );
}

function KindBadge({ kind }: { kind: 'stdio' | 'http' }) {
  return (
    <span className="rounded-full border border-border bg-secondary/60 px-2 py-0.5 font-mono text-[10.5px] uppercase tracking-wider text-muted-foreground">
      {kind}
    </span>
  );
}

function StatusPill({ server }: { server: McpServerView }) {
  if (server.restart_required) {
    return (
      <Pill className="border-amber-500/30 bg-amber-500/[0.08] text-amber-300">
        <Warning className="size-3" weight="fill" />
        restart to activate
      </Pill>
    );
  }
  if (server.status.kind === 'connected') {
    return (
      <Pill className="border-emerald-500/30 bg-emerald-500/[0.08] text-emerald-300">
        <CheckCircle className="size-3" weight="fill" />
        connected · {server.status.tool_count} tool{server.status.tool_count === 1 ? '' : 's'}
      </Pill>
    );
  }
  if (server.status.kind === 'error') {
    return (
      <Pill
        className="border-destructive/40 bg-destructive/10 text-destructive"
        title={server.status.message}
      >
        <WarningCircle className="size-3" weight="fill" />
        error
      </Pill>
    );
  }
  return null;
}

function Pill({ children, className, title }: { children: React.ReactNode; className?: string; title?: string }) {
  return (
    <span
      className={cn(
        'inline-flex items-center gap-1 rounded-full border px-2 py-0.5 text-[11px]',
        className,
      )}
      title={title}
    >
      {children}
    </span>
  );
}

/* ---------- empty state ---------- */

function EmptyState({ onAdd }: { onAdd: () => void }) {
  return (
    <div className="flex flex-col items-center justify-center gap-3 rounded-lg border border-dashed border-border/70 bg-card/40 px-6 py-14 text-center">
      <div className="rounded-full border border-border bg-secondary p-3 text-muted-foreground">
        <PuzzlePiece className="size-5" weight="fill" />
      </div>
      <div className="text-[14.5px] font-medium">No plugins yet</div>
      <p className="max-w-sm text-[13px] text-muted-foreground">
        Add a Model Context Protocol server to expose its tools to the agent. Try
        the official servers from{' '}
        <a
          href="https://github.com/modelcontextprotocol/servers"
          target="_blank"
          rel="noreferrer"
          className="inline-flex items-center gap-0.5 underline underline-offset-2 hover:text-foreground"
        >
          the MCP registry
          <ArrowSquareOut className="size-3" />
        </a>
        .
      </p>
      <Button size="sm" onClick={onAdd} className="mt-1 gap-1.5">
        <Plus className="size-3.5" weight="bold" />
        Add server
      </Button>
    </div>
  );
}

/* ---------- editor dialog ---------- */

type DraftServer =
  | {
      kind: 'stdio';
      name: string;
      command: string;
      args: string[];
      env: { key: string; value: string }[];
      cwd: string;
    }
  | {
      kind: 'http';
      name: string;
      url: string;
      auth: string;
    };

function emptyStdioDraft(): DraftServer {
  return { kind: 'stdio', name: '', command: '', args: [], env: [], cwd: '' };
}

function draftFromView(s: McpServerView): DraftServer {
  if (s.kind === 'stdio') {
    const stdio = s.config as McpStdioConfig;
    return {
      kind: 'stdio',
      name: s.name,
      command: stdio.command,
      args: [...stdio.args],
      env: Object.entries(stdio.env ?? {}).map(([key, value]) => ({ key, value })),
      cwd: stdio.cwd ?? '',
    };
  }
  const http = s.config as McpHttpConfig;
  return { kind: 'http', name: s.name, url: http.url, auth: http.auth ?? '' };
}

function EditorDialog({
  seed,
  busy,
  error,
  onCancel,
  onSave,
}: {
  seed: { originalName: string | null; draft: DraftServer };
  busy: boolean;
  error: string | null;
  onCancel: () => void;
  onSave: (originalName: string | null, draft: DraftServer) => void;
}) {
  const [draft, setDraft] = useState<DraftServer>(seed.draft);
  useEffect(() => {
    setDraft(seed.draft);
  }, [seed]);

  function setKind(kind: 'stdio' | 'http') {
    if (kind === draft.kind) return;
    if (kind === 'stdio') {
      setDraft({ kind: 'stdio', name: draft.name, command: '', args: [], env: [], cwd: '' });
    } else {
      setDraft({ kind: 'http', name: draft.name, url: '', auth: '' });
    }
  }

  return (
    <Dialog open onOpenChange={(o) => !o && onCancel()}>
      <DialogContent className="max-w-lg p-0 gap-0 overflow-hidden">
        <form
          onSubmit={(e) => {
            e.preventDefault();
            onSave(seed.originalName, draft);
          }}
          className="flex flex-col gap-4 p-5"
        >
          <div>
            <div className="text-[15px] font-semibold text-foreground">
              {seed.originalName ? 'Edit MCP server' : 'Add MCP server'}
            </div>
            <p className="mt-1 text-[12px] text-muted-foreground">
              Changes save immediately, but take effect on the next{' '}
              <code className="font-mono">mira serve</code> restart.
            </p>
          </div>

          <SegmentedKind value={draft.kind} onChange={setKind} />

          <Field label="Name" hint="Used as the tool namespace: mcp__<name>__*">
            <Input
              value={draft.name}
              onChange={(e) => setDraft({ ...draft, name: e.target.value })}
              placeholder="github"
              spellCheck={false}
              autoFocus
              disabled={busy}
            />
          </Field>

          {draft.kind === 'stdio' ? (
            <StdioFields draft={draft} setDraft={setDraft} busy={busy} />
          ) : (
            <HttpFields draft={draft} setDraft={setDraft} busy={busy} />
          )}

          {error && (
            <div className="rounded-md border border-destructive/40 bg-destructive/10 px-3 py-2 text-[12.5px] text-destructive">
              {error}
            </div>
          )}

          <div className="flex items-center justify-end gap-2">
            <Button type="button" variant="outline" onClick={onCancel} disabled={busy}>
              Cancel
            </Button>
            <Button type="submit" disabled={busy}>
              {busy ? 'Saving…' : seed.originalName ? 'Save' : 'Add server'}
            </Button>
          </div>
        </form>
      </DialogContent>
    </Dialog>
  );
}

function SegmentedKind({ value, onChange }: { value: 'stdio' | 'http'; onChange: (k: 'stdio' | 'http') => void }) {
  return (
    <div className="inline-flex w-fit rounded-full border border-border bg-secondary/60 p-0.5">
      {(['stdio', 'http'] as const).map((k) => (
        <button
          key={k}
          type="button"
          onClick={() => onChange(k)}
          className={cn(
            'rounded-full px-3.5 py-1 text-[12.5px] font-medium transition-colors',
            value === k
              ? 'bg-secondary text-foreground'
              : 'text-muted-foreground hover:text-foreground',
          )}
        >
          {k}
        </button>
      ))}
    </div>
  );
}

function StdioFields({
  draft,
  setDraft,
  busy,
}: {
  draft: Extract<DraftServer, { kind: 'stdio' }>;
  setDraft: (d: DraftServer) => void;
  busy: boolean;
}) {
  return (
    <>
      <Field label="Command">
        <Input
          value={draft.command}
          onChange={(e) => setDraft({ ...draft, command: e.target.value })}
          placeholder="npx"
          spellCheck={false}
          disabled={busy}
        />
      </Field>

      <Field label="Arguments" hint="One per row.">
        <ChipList
          values={draft.args}
          onChange={(next) => setDraft({ ...draft, args: next })}
          placeholder="-y @modelcontextprotocol/server-github"
          disabled={busy}
        />
      </Field>

      <Field label="Environment" hint="Use ${VAR} to pull from the parent env — e.g. ${GITHUB_TOKEN}.">
        <KeyValueList
          values={draft.env}
          onChange={(next) => setDraft({ ...draft, env: next })}
          disabled={busy}
        />
      </Field>

      <Field label="Working directory" hint="Optional.">
        <Input
          value={draft.cwd}
          onChange={(e) => setDraft({ ...draft, cwd: e.target.value })}
          placeholder="/path/to/dir"
          spellCheck={false}
          disabled={busy}
        />
      </Field>
    </>
  );
}

function HttpFields({
  draft,
  setDraft,
  busy,
}: {
  draft: Extract<DraftServer, { kind: 'http' }>;
  setDraft: (d: DraftServer) => void;
  busy: boolean;
}) {
  return (
    <>
      <Field label="URL">
        <Input
          value={draft.url}
          onChange={(e) => setDraft({ ...draft, url: e.target.value })}
          placeholder="https://mcp.example.com"
          spellCheck={false}
          disabled={busy}
        />
      </Field>
      <Field label="Authorization header" hint="Optional. Use ${VAR} — e.g. Bearer ${MCP_TOKEN}.">
        <Input
          value={draft.auth}
          onChange={(e) => setDraft({ ...draft, auth: e.target.value })}
          placeholder="Bearer ${MCP_TOKEN}"
          spellCheck={false}
          disabled={busy}
        />
      </Field>
    </>
  );
}

function Field({
  label,
  hint,
  children,
}: {
  label: string;
  hint?: string;
  children: React.ReactNode;
}) {
  return (
    <div className="flex flex-col gap-1.5">
      <label className="text-[12.5px] font-medium text-foreground/85">{label}</label>
      {children}
      {hint && <div className="text-[11.5px] text-muted-foreground/80">{hint}</div>}
    </div>
  );
}

function ChipList({
  values,
  onChange,
  placeholder,
  disabled,
}: {
  values: string[];
  onChange: (next: string[]) => void;
  placeholder?: string;
  disabled?: boolean;
}) {
  return (
    <div className="flex flex-col gap-1.5">
      {values.map((v, i) => (
        <div key={i} className="flex items-center gap-1.5">
          <Input
            value={v}
            onChange={(e) => {
              const next = values.slice();
              next[i] = e.target.value;
              onChange(next);
            }}
            placeholder={placeholder}
            spellCheck={false}
            disabled={disabled}
          />
          <IconButton
            title="Remove"
            onClick={() => onChange(values.filter((_, j) => j !== i))}
            disabled={disabled}
          >
            <X className="size-3.5" />
          </IconButton>
        </div>
      ))}
      <button
        type="button"
        onClick={() => onChange([...values, ''])}
        disabled={disabled}
        className="w-fit rounded-md border border-dashed border-border/70 px-2 py-1 text-[12px] text-muted-foreground transition-colors hover:border-border hover:text-foreground disabled:opacity-50"
      >
        + Add argument
      </button>
    </div>
  );
}

function KeyValueList({
  values,
  onChange,
  disabled,
}: {
  values: { key: string; value: string }[];
  onChange: (next: { key: string; value: string }[]) => void;
  disabled?: boolean;
}) {
  return (
    <div className="flex flex-col gap-1.5">
      {values.map((kv, i) => (
        <div key={i} className="grid grid-cols-[minmax(0,1fr)_minmax(0,1.4fr)_auto] items-center gap-1.5">
          <Input
            value={kv.key}
            onChange={(e) => {
              const next = values.slice();
              next[i] = { ...next[i], key: e.target.value };
              onChange(next);
            }}
            placeholder="GITHUB_TOKEN"
            spellCheck={false}
            disabled={disabled}
          />
          <Input
            value={kv.value}
            onChange={(e) => {
              const next = values.slice();
              next[i] = { ...next[i], value: e.target.value };
              onChange(next);
            }}
            placeholder="${GITHUB_TOKEN}"
            spellCheck={false}
            disabled={disabled}
          />
          <IconButton
            title="Remove"
            onClick={() => onChange(values.filter((_, j) => j !== i))}
            disabled={disabled}
          >
            <X className="size-3.5" />
          </IconButton>
        </div>
      ))}
      <button
        type="button"
        onClick={() => onChange([...values, { key: '', value: '' }])}
        disabled={disabled}
        className="w-fit rounded-md border border-dashed border-border/70 px-2 py-1 text-[12px] text-muted-foreground transition-colors hover:border-border hover:text-foreground disabled:opacity-50"
      >
        + Add variable
      </button>
    </div>
  );
}

function IconButton({
  onClick,
  title,
  disabled,
  children,
}: {
  onClick: () => void;
  title: string;
  disabled?: boolean;
  children: React.ReactNode;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      title={title}
      disabled={disabled}
      className="rounded-md p-1.5 text-muted-foreground transition-colors hover:bg-accent hover:text-foreground disabled:opacity-40"
    >
      {children}
    </button>
  );
}

/* ---------- tools dialog ---------- */

function ToolsDialog({ server, onClose }: { server: McpServerView; onClose: () => void }) {
  return (
    <Dialog open onOpenChange={(o) => !o && onClose()}>
      <DialogContent className="max-w-xl p-0 gap-0 overflow-hidden">
        <div className="flex flex-col gap-4 p-5">
          <div>
            <div className="text-[15px] font-semibold text-foreground">
              {server.name}
            </div>
            <p className="mt-1 text-[12px] text-muted-foreground">
              {server.tools.length} tool{server.tools.length === 1 ? '' : 's'} exposed to the agent.
            </p>
          </div>
          <div className="max-h-[420px] overflow-y-auto rounded-md border border-border/70">
            <ul className="divide-y divide-border/70">
              {server.tools.map((t) => (
                <ToolRow key={t.name} tool={t} />
              ))}
            </ul>
          </div>
          <div className="flex justify-end">
            <Button type="button" variant="outline" onClick={onClose}>
              Close
            </Button>
          </div>
        </div>
      </DialogContent>
    </Dialog>
  );
}

function ToolRow({ tool }: { tool: McpToolInfo }) {
  const desc = tool.description.trim();
  return (
    <li className="flex flex-col gap-1 px-3 py-2.5">
      <span className="font-mono text-[12.5px] text-foreground">{tool.name}</span>
      {desc && (
        <span className="text-[12px] leading-snug text-muted-foreground">
          {desc.length > 400 ? desc.slice(0, 400) + '…' : desc}
        </span>
      )}
    </li>
  );
}

/* ---------- confirm remove dialog ---------- */

function ConfirmRemoveDialog({
  server,
  busy,
  onCancel,
  onConfirm,
}: {
  server: McpServerView;
  busy: boolean;
  onCancel: () => void;
  onConfirm: () => void;
}) {
  return (
    <Dialog open onOpenChange={(o) => !o && onCancel()}>
      <DialogContent className="max-w-sm p-0 gap-0 overflow-hidden">
        <div className="flex flex-col gap-4 p-5">
          <div>
            <div className="text-[15px] font-semibold text-foreground">Remove server?</div>
            <p className="mt-1 text-[12.5px] text-muted-foreground">
              <span className="font-medium text-foreground">{server.name}</span> will be
              removed from your config. Its tools stay loaded until the next{' '}
              <code className="font-mono">mira serve</code> restart.
            </p>
          </div>
          <div className="flex justify-end gap-2">
            <Button type="button" variant="outline" onClick={onCancel} disabled={busy}>
              Cancel
            </Button>
            <Button
              type="button"
              onClick={onConfirm}
              disabled={busy}
              className="bg-destructive text-destructive-foreground hover:opacity-90"
            >
              {busy ? 'Removing…' : 'Remove'}
            </Button>
          </div>
        </div>
      </DialogContent>
    </Dialog>
  );
}

/* ---------- row menu (copy of Sidebar's, kept local for now) ---------- */

type RowMenuItem = {
  label: string;
  icon?: React.ReactNode;
  danger?: boolean;
  disabled?: boolean;
  onSelect: () => void;
};

function RowMenu({ items }: { items: RowMenuItem[] }) {
  const [open, setOpen] = useState(false);
  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>
        <button
          type="button"
          aria-label="Row menu"
          className={cn(
            'shrink-0 rounded-md p-1 text-muted-foreground/70 transition-colors hover:bg-accent hover:text-foreground',
            open && 'bg-accent text-foreground',
          )}
        >
          <DotsThree className="size-4" />
        </button>
      </PopoverTrigger>
      <PopoverContent className="w-56 p-1" align="end">
        <div className="flex flex-col">
          {items.map((it, i) => (
            <button
              key={i}
              type="button"
              disabled={it.disabled}
              onClick={() => {
                if (it.disabled) return;
                setOpen(false);
                it.onSelect();
              }}
              className={cn(
                'flex items-center gap-2 rounded-md px-2 py-1.5 text-left text-[13px] transition-colors',
                it.disabled
                  ? 'text-muted-foreground/40 cursor-not-allowed'
                  : it.danger
                    ? 'text-destructive hover:bg-destructive/10'
                    : 'text-foreground hover:bg-accent/60',
              )}
            >
              {it.icon && <span className="shrink-0 text-muted-foreground">{it.icon}</span>}
              <span>{it.label}</span>
            </button>
          ))}
        </div>
      </PopoverContent>
    </Popover>
  );
}

/* ---------- helpers ---------- */

function configSummary(s: McpServerView): string {
  if (s.kind === 'stdio') {
    const stdio = s.config as McpStdioConfig;
    return [stdio.command, ...stdio.args].filter(Boolean).join(' ');
  }
  const http = s.config as McpHttpConfig;
  return http.url;
}

function statusColor(s: McpServerView): string {
  if (s.restart_required) return 'bg-amber-500';
  if (s.status.kind === 'connected') return 'bg-emerald-500';
  if (s.status.kind === 'error') return 'bg-destructive';
  return 'bg-muted-foreground/50';
}

function statusLabel(s: McpServerView): string {
  if (s.restart_required) return 'Restart required to activate';
  if (s.status.kind === 'connected') return `${s.status.tool_count} tools registered`;
  if (s.status.kind === 'error') return s.status.message;
  return 'Not loaded';
}
