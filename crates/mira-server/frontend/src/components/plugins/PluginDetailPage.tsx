import { useEffect, useState } from 'react';
import {
  ArrowLeft,
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
import { Markdown } from '../Markdown';
import { Avatar, ErrorBanner, Pill, Switch } from './shared';
import { faviconSrc } from './DiscoverTab';

export function PluginDetailPage({
  id,
  version,
  busy,
  onBack,
  onInstall,
  onUninstall,
  onToggle,
  actionError,
}: {
  id: string;
  version: number;
  busy: string | null;
  onBack: () => void;
  onInstall: (id: string) => void;
  onUninstall: (id: string) => void;
  onToggle: (id: string, enabled: boolean) => void;
  actionError?: string | null;
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

  const title = ((detail?.display_name || detail?.name) || id.split('@')[0]).replace(/^\w/, (c) => c.toUpperCase());
  const c = detail?.components;
  const installing = busy === `install:${id}`;

  return (
    <div className="flex flex-col gap-6">
      {/* Breadcrumb */}
      <button
        type="button"
        onClick={onBack}
        className="flex items-center gap-1.5 self-start text-[12.5px] text-muted-foreground transition-colors hover:text-foreground"
      >
        <ArrowLeft className="size-3.5" />
        <span>Plugins</span>
        <span className="mx-0.5 text-muted-foreground/40">›</span>
        <span className="text-foreground/80">{title}</span>
      </button>

      {(error || actionError) && <ErrorBanner message={(error || actionError)!} />}

      {!detail && !error && (
        <div className="py-10 text-center text-[13px] text-muted-foreground">Loading…</div>
      )}

      {detail && (
        <>
          {/* Hero card */}
          <div className="flex gap-5 rounded-2xl border border-border/60 bg-white/[0.035] p-6">
            <Avatar name={id} size="lg" src={detail.icon_url ?? faviconSrc(detail.homepage)} />
            <div className="min-w-0 flex-1">
              <div className="flex flex-wrap items-start justify-between gap-3">
                <div className="min-w-0">
                  <h1 className="text-[22px] font-bold leading-tight tracking-tight">{title}</h1>
                  <div className="mt-0.5 text-[12.5px] text-muted-foreground/70">
                    {[detail.author, detail.marketplace, detail.installed_version ?? detail.version]
                      .filter(Boolean)
                      .join(' · ')}
                  </div>
                </div>
                <div className="flex shrink-0 items-center gap-2">
                  {detail.installed && (
                    <Switch
                      checked={detail.enabled}
                      label={detail.enabled ? 'Disable plugin' : 'Enable plugin'}
                      disabled={!!busy}
                      onChange={(v) => onToggle(detail.id, v)}
                    />
                  )}
                  {detail.installed ? (
                    <>
                      <Pill tone={detail.enabled ? 'green' : 'neutral'}>
                        {detail.enabled ? (
                          <><CheckCircle2 className="size-3" /> Installed</>
                        ) : (
                          'Disabled'
                        )}
                      </Pill>
                      <Button variant="outline" size="sm" disabled={!!busy} onClick={() => onUninstall(detail.id)}>
                        Uninstall
                      </Button>
                    </>
                  ) : (
                    <Button
                      size="sm"
                      disabled={!detail.installable || !!busy}
                      onClick={() => onInstall(detail.id)}
                      title={!detail.installable ? "Mira can't install this source type yet" : undefined}
                    >
                      {installing ? 'Installing…' : '+ Install'}
                    </Button>
                  )}
                </div>
              </div>
              {detail.description && (
                <p className="mt-3 text-[13.5px] leading-relaxed text-foreground/85">{detail.description}</p>
              )}
              {detail.category && (
                <span className="mt-3 inline-block rounded-full border border-border/40 px-2.5 py-0.5 text-[11px] capitalize text-muted-foreground/60">
                  {detail.category}
                </span>
              )}
            </div>
          </div>

          {/* What's included */}
          <section>
            <h2 className="mb-3 text-[11.5px] font-semibold uppercase tracking-wider text-muted-foreground/60">
              What's Included
            </h2>
            {c ? (
              <div className="flex flex-col gap-2">
                <IncludeRow icon={<Terminal />} label="Commands" items={detail.command_names.map((n) => `/${n}`)} />
                <IncludeRow icon={<Bot />} label="Agents" items={detail.agent_names} />
                <IncludeRow icon={<Sparkles />} label="Skills" items={c.skills} />
                <IncludeRow icon={<Plug />} label="MCP Servers" items={c.mcp_server_names} />
                <IncludeRow icon={<Zap />} label="Hooks" items={c.hooks} note="not run by Mira yet" />
                <IncludeRow icon={<Cpu />} label="LSP Servers" items={c.lsp_servers} note="not run by Mira yet" />
                {detail.command_names.length +
                  detail.agent_names.length +
                  c.skills.length +
                  c.mcp_server_names.length +
                  c.hooks.length +
                  c.lsp_servers.length === 0 && (
                  <div className="rounded-xl border border-border/50 bg-white/[0.035] px-4 py-3 text-[13px] text-muted-foreground">
                    Nothing Mira can use yet.
                  </div>
                )}
              </div>
            ) : (
              <div className="rounded-xl border border-border/50 bg-white/[0.035] px-4 py-3 text-[13px] text-muted-foreground">
                Shown after install: fetched from{' '}
                <code className="font-mono text-[12px]">{detail.source}</code>.
              </div>
            )}
            {c && c.problems.length > 0 && (
              <ul className="mt-2 list-disc pl-5 text-[12.5px] text-amber-300">
                {c.problems.map((p) => <li key={p}>{p}</li>)}
              </ul>
            )}
          </section>

          {/* Readme */}
          {detail.readme && (
            <section>
              <h2 className="mb-3 flex items-center gap-1.5 text-[11.5px] font-semibold uppercase tracking-wider text-muted-foreground/60">
                <BookOpen className="size-3.5" /> Readme
              </h2>
              <div className="rounded-xl border border-border/60 bg-white/[0.035] px-5 py-4 text-[13.5px]">
                <Markdown text={detail.readme} />
              </div>
            </section>
          )}

          {/* Information table */}
          <section>
            <h2 className="mb-3 text-[11.5px] font-semibold uppercase tracking-wider text-muted-foreground/60">
              Information
            </h2>
            <div className="overflow-hidden rounded-xl border border-border/60 bg-white/[0.035]">
              <InfoRow label="Source" value={detail.source} mono />
              {detail.license && <InfoRow label="License" value={detail.license} />}
              {detail.marketplace && <InfoRow label="Marketplace" value={detail.marketplace} />}
              {detail.path && <InfoRow label="Installed at" value={detail.path} mono />}
              {detail.homepage && (
                <div className="flex items-center gap-4 border-t border-border/40 px-4 py-3 text-[12.5px] first:border-t-0">
                  <span className="w-28 shrink-0 text-muted-foreground">Homepage</span>
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
            </div>
          </section>

          <p className="pb-4 text-[12px] text-muted-foreground">
            {detail.installed
              ? 'Commands, skills and MCP servers apply right away; agents in new sessions.'
              : 'Plugins can run commands on your machine. Install ones you trust.'}
          </p>
        </>
      )}
    </div>
  );
}

function IncludeRow({
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
    <div className="flex items-start gap-4 rounded-xl border border-border/50 bg-white/[0.035] px-4 py-3">
      <div className="mt-0.5 shrink-0 text-muted-foreground/70 [&_svg]:size-4">{icon}</div>
      <div className="min-w-0 flex-1">
        <div className="flex items-center gap-2 text-[13px] font-medium">
          {label}
          <span className="text-[12px] text-muted-foreground/60">· {items.length}</span>
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
    </div>
  );
}

function InfoRow({ label, value, mono }: { label: string; value: string; mono?: boolean }) {
  return (
    <div className="flex items-center gap-4 border-t border-border/40 px-4 py-3 text-[12.5px] first:border-t-0">
      <span className="w-28 shrink-0 text-muted-foreground">{label}</span>
      <span
        className={mono ? 'min-w-0 truncate font-mono text-[12px] text-foreground/80' : 'text-foreground/80'}
        title={value}
      >
        {value}
      </span>
    </div>
  );
}
