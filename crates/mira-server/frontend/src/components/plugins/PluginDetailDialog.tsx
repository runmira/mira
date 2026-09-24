import { useEffect, useState } from 'react';
import {
  BookOpen,
  Bot,
  CheckCircle2,
  Cpu,
  ExternalLink,
  Plug,
  Sparkles,
  Terminal,
  Zap,
} from 'lucide-react';
import { getPluginDetail, type PluginDetail } from '../../api';
import { Button } from '@/components/ui/button';
import { Dialog, DialogContent } from '@/components/ui/dialog';
import { Markdown } from '../Markdown';
import { Avatar, ErrorBanner, Pill, Switch } from './shared';

export function PluginDetailDialog({
  id,
  version,
  busy,
  onClose,
  onInstall,
  onUninstall,
  onToggle,
}: {
  id: string;
  version: number;
  busy: string | null;
  onClose: () => void;
  onInstall: (id: string) => void;
  onUninstall: (id: string) => void;
  onToggle: (id: string, enabled: boolean) => void;
}) {
  const [detail, setDetail] = useState<PluginDetail | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    getPluginDetail(id)
      .then((d) => {
        setDetail(d);
        setError(null);
      })
      .catch((e) => setError((e as Error).message));
  }, [id, version]);

  const title = detail?.display_name || detail?.name || id.split('@')[0];
  const c = detail?.components;
  const installing = busy === `install:${id}`;

  return (
    <Dialog open onOpenChange={(o) => !o && onClose()}>
      <DialogContent className="max-h-[85vh] max-w-2xl gap-0 overflow-hidden p-0">
        <div className="flex items-start gap-4 border-b border-border/60 px-6 pb-5 pt-6">
          <Avatar name={id} size="lg" src={detail?.icon_url} />
          <div className="min-w-0 flex-1">
            <div className="flex flex-wrap items-center gap-2">
              <h2 className="text-[18px] font-semibold tracking-tight">{title}</h2>
              {detail?.installed && (
                <Pill tone={detail.enabled ? 'green' : 'neutral'}>
                  {detail.enabled ? (
                    <>
                      <CheckCircle2 className="size-3" /> Installed
                    </>
                  ) : (
                    'Disabled'
                  )}
                </Pill>
              )}
            </div>
            <div className="mt-1 text-[12.5px] text-muted-foreground/70">
              {[detail?.author, detail?.marketplace, detail?.installed_version ?? detail?.version]
                .filter(Boolean)
                .join(' · ')}
            </div>
            {detail?.description && (
              <p className="mt-2 text-[13.5px] leading-relaxed text-foreground/85">{detail.description}</p>
            )}
          </div>
        </div>

        <div className="min-h-0 flex-1 overflow-y-auto px-6 py-5">
          {error && <ErrorBanner message={error} />}
          {!detail && !error && (
            <div className="py-6 text-center text-[13px] text-muted-foreground">Loading…</div>
          )}
          {detail && (
            <div className="flex flex-col gap-6">
              <section>
                <h3 className="mb-3 text-[11.5px] font-semibold uppercase tracking-wider text-muted-foreground/60">
                  Includes
                </h3>
                {c ? (
                  <div className="grid gap-2 sm:grid-cols-2">
                    <Includes icon={<Terminal />}  label="Commands"    items={detail.command_names.map((n) => `/${n}`)} />
                    <Includes icon={<Bot />}        label="Agents"      items={detail.agent_names} />
                    <Includes icon={<Sparkles />}   label="Skills"      items={c.skills} />
                    <Includes icon={<Plug />}        label="MCP servers" items={c.mcp_server_names} />
                    <Includes icon={<Zap />}         label="Hooks"       items={c.hooks} note="not run by Mira yet" />
                    <Includes icon={<Cpu />}         label="LSP servers" items={c.lsp_servers} note="not run by Mira yet" />
                    {detail.command_names.length +
                      detail.agent_names.length +
                      c.skills.length +
                      c.mcp_server_names.length +
                      c.hooks.length +
                      c.lsp_servers.length ===
                      0 && (
                      <div className="text-[13px] text-muted-foreground">Nothing Mira can use.</div>
                    )}
                  </div>
                ) : (
                  <div className="text-[13px] text-muted-foreground">
                    Shown after install: this plugin is fetched from{' '}
                    <code className="font-mono text-[12px]">{detail.source}</code>.
                  </div>
                )}
                {c && c.problems.length > 0 && (
                  <ul className="mt-2 list-disc pl-5 text-[12.5px] text-amber-300">
                    {c.problems.map((p) => (
                      <li key={p}>{p}</li>
                    ))}
                  </ul>
                )}
              </section>

              {detail.readme && (
                <section>
                  <h3 className="mb-3 flex items-center gap-1.5 text-[11.5px] font-semibold uppercase tracking-wider text-muted-foreground/60">
                    <BookOpen className="size-3.5" /> Readme
                  </h3>
                  <div className="rounded-xl border border-border/60 bg-white/[0.02] px-4 py-3 text-[13.5px]">
                    <Markdown text={detail.readme} />
                  </div>
                </section>
              )}

              <section className="grid gap-1.5 text-[12.5px] text-muted-foreground">
                <Row label="Source" value={detail.source} mono />
                {detail.license && <Row label="License" value={detail.license} />}
                {detail.path && <Row label="Installed at" value={detail.path} mono />}
                {detail.homepage && (
                  <div className="flex gap-2">
                    <span className="w-24 shrink-0">Homepage</span>
                    <a
                      href={detail.homepage}
                      target="_blank"
                      rel="noreferrer"
                      className="inline-flex min-w-0 items-center gap-1 truncate text-foreground/85 underline-offset-2 hover:underline"
                    >
                      {detail.homepage}
                      <ExternalLink className="size-3 shrink-0" />
                    </a>
                  </div>
                )}
              </section>
            </div>
          )}
        </div>

        <div className="flex items-center justify-between gap-3 border-t border-border/60 px-6 py-4">
          <div className="text-[12px] text-muted-foreground">
            {detail?.installed
              ? 'Commands, skills and MCP servers apply right away; agents in new sessions.'
              : 'Plugins can run commands on your machine. Install ones you trust.'}
          </div>
          <div className="flex shrink-0 items-center gap-2">
            {detail?.installed ? (
              <>
                <Switch
                  checked={detail.enabled}
                  label={detail.enabled ? 'Disable plugin' : 'Enable plugin'}
                  disabled={!!busy}
                  onChange={(v) => onToggle(detail.id, v)}
                />
                <Button variant="outline" size="sm" disabled={!!busy} onClick={() => onUninstall(detail.id)}>
                  Uninstall
                </Button>
              </>
            ) : (
              <Button
                size="sm"
                disabled={!detail || !detail.installable || !!busy}
                onClick={() => detail && onInstall(detail.id)}
                title={detail && !detail.installable ? "Mira can't install this source type yet" : undefined}
              >
                {installing ? 'Installing…' : 'Install'}
              </Button>
            )}
          </div>
        </div>
      </DialogContent>
    </Dialog>
  );
}

function Includes({
  icon,
  label,
  items,
  note,
}: {
  icon: React.ReactNode;
  label: string;
  items: string[];
  note?: string;
}) {
  if (items.length === 0) return null;
  return (
    <div className="rounded-xl border border-border/50 bg-white/[0.02] px-3 py-2.5">
      <div className="flex items-center gap-1.5 text-[12.5px] font-medium [&_svg]:size-3.5 [&_svg]:text-muted-foreground/70">
        {icon}
        {label}
        <span className="text-muted-foreground/60">· {items.length}</span>
        {note && <span className="ml-auto text-[11px] font-normal text-muted-foreground/50">{note}</span>}
      </div>
      <div className="mt-2 flex flex-wrap gap-1">
        {items.slice(0, 12).map((i) => (
          <code
            key={i}
            className="rounded-md bg-white/5 px-1.5 py-0.5 font-mono text-[11.5px] text-foreground/80"
          >
            {i}
          </code>
        ))}
        {items.length > 12 && (
          <span className="text-[11.5px] text-muted-foreground/60">+{items.length - 12} more</span>
        )}
      </div>
    </div>
  );
}

function Row({ label, value, mono }: { label: string; value: string; mono?: boolean }) {
  return (
    <div className="flex gap-2">
      <span className="w-24 shrink-0">{label}</span>
      <span
        className={mono ? 'min-w-0 truncate font-mono text-[12px] text-foreground/80' : 'text-foreground/80'}
        title={value}
      >
        {value}
      </span>
    </div>
  );
}
