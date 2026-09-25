import { useState } from 'react';
import { BookOpen, Lock, Terminal, Wrench } from 'lucide-react';
import type { McpServerConfig, McpServerView, WriteScope } from '../../api';
import { Button } from '@/components/ui/button';
import { Dialog, DialogContent } from '@/components/ui/dialog';
import { Input } from '@/components/ui/input';
import { cn } from '@/lib/utils';
import { StatusDot, statusText, writeScope } from './McpTab';
import { ErrorBanner, Pill } from './shared';

/* ---------- detail ---------- */

export function ServerDetailDialog({ s, onClose }: { s: McpServerView; onClose: () => void }) {
  return (
    <Dialog open onOpenChange={(o) => !o && onClose()}>
      <DialogContent className="max-h-[85vh] max-w-2xl gap-0 overflow-hidden p-0">
        <div className="border-b border-border/70 px-6 pb-4 pt-6">
          <div className="flex items-start gap-3">
            <StatusDot status={s.status} />
            <div className="min-w-0 flex-1">
              <div className="flex flex-wrap items-center gap-2">
                <h2 className="text-[17px] font-semibold">{s.name}</h2>
                <Pill className="font-mono uppercase tracking-wider">{s.transport}</Pill>
                <Pill>{s.scope.kind === 'plugin' ? `plugin · ${s.scope.plugin}` : s.scope.kind}</Pill>
              </div>
              <div className="mt-1 break-all font-mono text-[12px] text-muted-foreground">{s.target}</div>
              <div
                className={cn(
                  'mt-2 text-[13px]',
                  s.status.state === 'failed' ? 'text-destructive' : 'text-foreground/85',
                )}
              >
                {statusText(s)}
              </div>
            </div>
          </div>
        </div>
        <div className="min-h-0 flex-1 overflow-y-auto px-6 py-5">
          <div className="flex flex-col gap-5">
            {s.instructions && (
              <section>
                <H icon={<BookOpen />}>Instructions from the server</H>
                <p className="whitespace-pre-wrap text-[12.5px] text-muted-foreground">{s.instructions}</p>
              </section>
            )}
            <section>
              <H icon={<Wrench />}>Tools · {s.tools.length}</H>
              {s.tools.length === 0 ? (
                <p className="text-[12.5px] text-muted-foreground">
                  {s.status.state === 'connected' ? 'This server has no tools.' : 'Known once connected.'}
                </p>
              ) : (
                <ul className="flex flex-col divide-y divide-border/50 rounded-lg border border-border/70">
                  {s.tools.map((t) => (
                    <li key={t.name} className="px-3 py-2">
                      <div className="flex items-center gap-2">
                        <code className="font-mono text-[12.5px] text-foreground">{t.remote_name}</code>
                        {t.read_only && <Pill tone="green">read-only</Pill>}
                      </div>
                      {t.description && (
                        <p className="mt-0.5 line-clamp-3 text-[12px] text-muted-foreground">{t.description}</p>
                      )}
                    </li>
                  ))}
                </ul>
              )}
            </section>
            {s.prompts.length > 0 && (
              <section>
                <H icon={<Terminal />}>Prompts · run as slash commands</H>
                <ul className="flex flex-col gap-1">
                  {s.prompts.map((p) => (
                    <li key={p.name} className="text-[12.5px]">
                      <code className="font-mono text-foreground">
                        /mcp__{s.name.replace(/[^A-Za-z0-9-]+/g, '_')}__{p.name}
                      </code>
                      {p.arguments.length > 0 && (
                        <span className="font-mono text-muted-foreground">
                          {' '}
                          {p.arguments.map((a) => `<${a.name}>`).join(' ')}
                        </span>
                      )}
                      {p.description && <span className="text-muted-foreground"> — {p.description}</span>}
                    </li>
                  ))}
                </ul>
              </section>
            )}
            {s.resources.length > 0 && (
              <section>
                <H icon={<BookOpen />}>Resources · {s.resources.length}</H>
                <ul className="flex flex-col gap-1">
                  {s.resources.slice(0, 50).map((r) => (
                    <li key={r.uri} className="truncate text-[12px]" title={r.uri}>
                      <span className="text-foreground/90">{r.name}</span>{' '}
                      <span className="font-mono text-muted-foreground">{r.uri}</span>
                    </li>
                  ))}
                </ul>
              </section>
            )}
            <section className="grid gap-1 text-[12px] text-muted-foreground">
              {s.server_name && (
                <div>
                  Server: <span className="text-foreground/85">{s.server_name} {s.server_version}</span>
                </div>
              )}
              {s.can_sign_in && (
                <div className="flex items-center gap-1">
                  <Lock className="size-3.5" /> {s.signed_in ? 'Signed in with OAuth' : 'Not signed in'}
                </div>
              )}
              {s.source && (
                <div className="truncate">
                  Defined in <span className="font-mono text-foreground/85">{s.source}</span>
                </div>
              )}
              {s.log_path && (
                <div className="truncate">
                  Server log <span className="font-mono text-foreground/85">{s.log_path}</span>
                </div>
              )}
            </section>
          </div>
        </div>
      </DialogContent>
    </Dialog>
  );
}

function H({ icon, children }: { icon: React.ReactNode; children: React.ReactNode }) {
  return (
    <h3 className="mb-2 flex items-center gap-1.5 text-[12px] font-medium uppercase tracking-wider text-muted-foreground [&_svg]:size-3.5">
      {icon}
      {children}
    </h3>
  );
}

/* ---------- add / edit ---------- */

type Transport = 'stdio' | 'http' | 'sse';

type Draft = {
  name: string;
  scope: WriteScope;
  transport: Transport;
  command: string;
  args: string;
  env: string;
  url: string;
  headers: string;
};

function lines(text: string): string[] {
  return text
    .split('\n')
    .map((l) => l.trim())
    .filter(Boolean);
}

function pairs(text: string, sep: string, what: string): Record<string, string> {
  const out: Record<string, string> = {};
  for (const l of lines(text)) {
    const i = l.indexOf(sep);
    if (i <= 0) throw new Error(`${what} “${l}” should look like KEY${sep}value`);
    out[l.slice(0, i).trim()] = l.slice(i + 1).trim();
  }
  return out;
}

function draftFrom(s: McpServerView | null): Draft {
  const base: Draft = {
    name: '',
    scope: 'local',
    transport: 'stdio',
    command: '',
    args: '',
    env: '',
    url: '',
    headers: '',
  };
  if (!s) return base;
  const c = s.config;
  const d = { ...base, name: s.name, scope: writeScope(s) ?? 'local', transport: s.transport };
  if ('command' in c) {
    return {
      ...d,
      command: c.command,
      args: (c.args ?? []).join('\n'),
      env: Object.entries(c.env ?? {})
        .map(([k, v]) => `${k}=${v}`)
        .join('\n'),
    };
  }
  const headers = { ...(c.headers ?? {}) };
  if (c.auth && !headers.Authorization) headers.Authorization = c.auth;
  return {
    ...d,
    url: c.url,
    headers: Object.entries(headers)
      .map(([k, v]) => `${k}: ${v}`)
      .join('\n'),
  };
}

function configFrom(d: Draft): McpServerConfig {
  if (d.transport === 'stdio') {
    if (!d.command.trim()) throw new Error('Enter the command that starts the server.');
    return { type: 'stdio', command: d.command.trim(), args: lines(d.args), env: pairs(d.env, '=', 'Variable') };
  }
  if (!/^https?:\/\//.test(d.url.trim())) throw new Error('Enter the server’s http(s) URL.');
  return { type: d.transport, url: d.url.trim(), headers: pairs(d.headers, ':', 'Header') };
}

const SCOPES: { key: WriteScope; label: string; hint: string }[] = [
  { key: 'local', label: 'Local', hint: 'This project, only you' },
  { key: 'project', label: 'Project', hint: 'Saved to .mcp.json and shared with the repo' },
  { key: 'user', label: 'User', hint: 'All your projects' },
];

export function ServerEditorDialog({
  editing,
  hasProject,
  onClose,
  onSave,
}: {
  editing: McpServerView | null;
  hasProject: boolean;
  onClose: () => void;
  onSave: (req: {
    name: string;
    scope: WriteScope;
    config: McpServerConfig;
    replaces: { name: string; scope: WriteScope } | null;
  }) => Promise<boolean>;
}) {
  const [draft, setDraft] = useState<Draft>(() => {
    const d = draftFrom(editing);
    if (!editing && !hasProject) d.scope = 'user';
    return d;
  });
  const [mode, setMode] = useState<'form' | 'json'>('form');
  const [json, setJson] = useState('');
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);

  const set = (patch: Partial<Draft>) => setDraft((d) => ({ ...d, ...patch }));

  async function save() {
    setError(null);
    let config: McpServerConfig;
    try {
      if (!/^[A-Za-z0-9._-]{1,64}$/.test(draft.name.trim())) {
        throw new Error('Name: letters, digits, “-”, “_” or “.”.');
      }
      if (mode === 'json') {
        const parsed = JSON.parse(json);
        const entry = parsed.mcpServers ? Object.values(parsed.mcpServers)[0] : parsed;
        if (!entry || typeof entry !== 'object') throw new Error('Paste one server’s JSON.');
        config = entry as McpServerConfig;
      } else {
        config = configFrom(draft);
      }
    } catch (e) {
      setError((e as Error).message);
      return;
    }
    setSaving(true);
    const ok = await onSave({
      name: draft.name.trim(),
      scope: draft.scope,
      config,
      replaces: editing && writeScope(editing) ? { name: editing.name, scope: writeScope(editing)! } : null,
    });
    setSaving(false);
    if (ok) onClose();
  }

  return (
    <Dialog open onOpenChange={(o) => !o && onClose()}>
      <DialogContent className="max-h-[90vh] max-w-xl overflow-y-auto">
        <div>
          <div className="text-[16px] font-semibold">{editing ? `Edit ${editing.name}` : 'Add MCP server'}</div>
          <div className="mt-0.5 text-[12.5px] text-muted-foreground">
            Local command (stdio) or hosted server (HTTP / SSE). Hosted servers that use OAuth
            need no token here: sign in after adding.
          </div>
        </div>

        <Segmented
          value={mode}
          onChange={(v) => setMode(v as 'form' | 'json')}
          options={[
            { value: 'form', label: 'Form' },
            { value: 'json', label: 'Paste JSON' },
          ]}
        />

        <Field label="Name">
          <Input value={draft.name} onChange={(e) => set({ name: e.target.value })} placeholder="github" className="h-9" />
        </Field>

        <Field label="Available in">
          <div className="grid grid-cols-3 gap-1.5">
            {SCOPES.map((s) => (
              <button
                key={s.key}
                type="button"
                disabled={s.key !== 'user' && !hasProject}
                onClick={() => set({ scope: s.key })}
                className={cn(
                  'rounded-lg border px-2.5 py-2 text-left transition-colors disabled:opacity-40',
                  draft.scope === s.key ? 'border-mira-purple/60 bg-mira-purple/10' : 'border-border/70 hover:border-border',
                )}
              >
                <div className="text-[12.5px] font-medium">{s.label}</div>
                <div className="text-[11px] leading-snug text-muted-foreground">{s.hint}</div>
              </button>
            ))}
          </div>
        </Field>

        {mode === 'json' ? (
          <Field label="Server JSON (as in .mcp.json)">
            <textarea
              value={json}
              onChange={(e) => setJson(e.target.value)}
              rows={8}
              spellCheck={false}
              placeholder={'{\n  "type": "http",\n  "url": "https://mcp.example.com/mcp"\n}'}
              className="w-full rounded-md border border-input bg-transparent px-3 py-2 font-mono text-[12px] outline-none focus:ring-2 focus:ring-ring"
            />
          </Field>
        ) : (
          <>
            <Field label="Transport">
              <Segmented
                value={draft.transport}
                onChange={(v) => set({ transport: v as Transport })}
                options={[
                  { value: 'stdio', label: 'Local command' },
                  { value: 'http', label: 'HTTP' },
                  { value: 'sse', label: 'SSE' },
                ]}
              />
            </Field>
            {draft.transport === 'stdio' ? (
              <>
                <Field label="Command">
                  <Input
                    value={draft.command}
                    onChange={(e) => set({ command: e.target.value })}
                    placeholder="npx"
                    className="h-9 font-mono text-[12.5px]"
                  />
                </Field>
                <Field label="Arguments" hint="One per line">
                  <Area value={draft.args} onChange={(v) => set({ args: v })} placeholder={'-y\n@modelcontextprotocol/server-github'} />
                </Field>
                <Field label="Environment" hint="KEY=value per line · ${VAR} reads your environment">
                  <Area value={draft.env} onChange={(v) => set({ env: v })} placeholder="GITHUB_TOKEN=${GITHUB_TOKEN}" />
                </Field>
              </>
            ) : (
              <>
                <Field label="URL">
                  <Input
                    value={draft.url}
                    onChange={(e) => set({ url: e.target.value })}
                    placeholder="https://mcp.example.com/mcp"
                    className="h-9 font-mono text-[12.5px]"
                  />
                </Field>
                <Field label="Headers" hint="Name: value per line · optional">
                  <Area value={draft.headers} onChange={(v) => set({ headers: v })} placeholder="Authorization: Bearer ${API_TOKEN}" />
                </Field>
              </>
            )}
          </>
        )}

        {error && <ErrorBanner message={error} />}

        <div className="flex justify-end gap-2">
          <Button variant="ghost" size="sm" onClick={onClose}>
            Cancel
          </Button>
          <Button size="sm" disabled={saving} onClick={save}>
            {saving ? 'Saving…' : editing ? 'Save' : 'Add server'}
          </Button>
        </div>
      </DialogContent>
    </Dialog>
  );
}

function Field({ label, hint, children }: { label: string; hint?: string; children: React.ReactNode }) {
  return (
    <label className="flex flex-col gap-1.5">
      <span className="text-[12.5px] font-medium">
        {label}
        {hint && <span className="ml-2 font-normal text-muted-foreground">{hint}</span>}
      </span>
      {children}
    </label>
  );
}

function Area({ value, onChange, placeholder }: { value: string; onChange: (v: string) => void; placeholder?: string }) {
  return (
    <textarea
      value={value}
      onChange={(e) => onChange(e.target.value)}
      rows={3}
      spellCheck={false}
      placeholder={placeholder}
      className="w-full rounded-md border border-input bg-transparent px-3 py-2 font-mono text-[12px] outline-none focus:ring-2 focus:ring-ring"
    />
  );
}

function Segmented({
  value,
  onChange,
  options,
}: {
  value: string;
  onChange: (v: string) => void;
  options: { value: string; label: string }[];
}) {
  return (
    <div className="inline-flex w-fit rounded-lg border border-border/70 bg-secondary/40 p-0.5">
      {options.map((o) => (
        <button
          key={o.value}
          type="button"
          onClick={() => onChange(o.value)}
          className={cn(
            'rounded-md px-3 py-1 text-[12.5px] transition-colors',
            value === o.value ? 'bg-background text-foreground shadow-sm' : 'text-muted-foreground hover:text-foreground',
          )}
        >
          {o.label}
        </button>
      ))}
    </div>
  );
}

/* ---------- tokens ---------- */

/** Where to create a token for well-known variables. */
function tokenHelp(name: string): { label: string; url: string } | null {
  const n = name.toUpperCase();
  if (n.includes('GITHUB')) return { label: 'Create a GitHub token', url: 'https://github.com/settings/personal-access-tokens/new' };
  if (n.includes('FIGMA')) return { label: 'Create a Figma token', url: 'https://www.figma.com/settings' };
  if (n.includes('CONTEXT7')) return { label: 'Get a Context7 key', url: 'https://context7.com/dashboard' };
  if (n.includes('LINEAR')) return { label: 'Create a Linear key', url: 'https://linear.app/settings/account/security' };
  return null;
}

/** Paste values for a server's unset `${VAR}`s. Saved privately in
 *  `~/.mira/mcp/variables.json` (readable only by you); servers that use
 *  them reconnect right away. */
export function TokenDialog({
  s,
  onClose,
  onSave,
}: {
  s: McpServerView;
  onClose: () => void;
  onSave: (values: Record<string, string>) => Promise<boolean>;
}) {
  const [values, setValues] = useState<Record<string, string>>({});
  const [saving, setSaving] = useState(false);
  const filled = Object.values(values).some((v) => v.trim());
  return (
    <Dialog open onOpenChange={(o) => !o && onClose()}>
      <DialogContent className="max-w-md">
        <div>
          <div className="text-[16px] font-semibold">Add token for {s.name.split(':').pop()}</div>
          <div className="mt-0.5 text-[12.5px] text-muted-foreground">
            This server’s definition uses {s.missing_vars.length === 1 ? 'a value' : 'values'} you
            haven’t set. It’s saved privately on this computer (~/.mira/mcp/variables.json) and the
            server reconnects.
          </div>
        </div>
        {s.missing_vars.map((name) => {
          const help = tokenHelp(name);
          return (
            <label key={name} className="flex flex-col gap-1.5">
              <span className="flex items-center justify-between gap-2 text-[12.5px] font-medium">
                <code className="font-mono">{name}</code>
                {help && (
                  <a href={help.url} target="_blank" rel="noreferrer" className="text-[11.5px] font-normal text-mira-blue hover:underline">
                    {help.label}
                  </a>
                )}
              </span>
              <Input
                type="password"
                autoComplete="off"
                spellCheck={false}
                value={values[name] ?? ''}
                onChange={(e) => setValues((v) => ({ ...v, [name]: e.target.value }))}
                className="h-9 font-mono text-[12.5px]"
              />
            </label>
          );
        })}
        <div className="flex justify-end gap-2">
          <Button variant="ghost" size="sm" onClick={onClose}>
            Cancel
          </Button>
          <Button
            size="sm"
            disabled={!filled || saving}
            onClick={async () => {
              setSaving(true);
              const ok = await onSave(values);
              setSaving(false);
              if (ok) onClose();
            }}
          >
            {saving ? 'Saving…' : 'Save'}
          </Button>
        </div>
      </DialogContent>
    </Dialog>
  );
}
