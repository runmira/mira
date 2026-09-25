import { useCallback, useEffect, useState } from 'react';
import { AlertCircle, Puzzle, Search } from 'lucide-react';
import {
  addMarketplace,
  deleteMcpServer,
  getPlugins,
  installPlugin,
  listMcp,
  reconnectMcp,
  removeMarketplace,
  saveMcpServer,
  setMcpApproval,
  setMcpEnabled,
  setPluginEnabled,
  signInMcp,
  setMcpToolEnabled,
  setMcpToolLoading,
  setMcpVariable,
  signOutMcp,
  uninstallPlugin,
  updateMarketplace,
  type InstalledPluginView,
  type MarketplaceView,
  type McpListView,
  type McpServerView,
  type ToolLoading,
  type PluginsOverview,
} from '../api';
import { cn } from '@/lib/utils';
import { isDesktop, openExternal } from '../lib/desktop';
import { DiscoverTab } from './plugins/DiscoverTab';
import { InstalledTab } from './plugins/InstalledTab';
import { MarketplacesTab } from './plugins/MarketplacesTab';
import { McpTab, writeScope } from './plugins/McpTab';
import { ServerEditorDialog, TokenDialog } from './plugins/McpDialogs';
import { McpDetailPage } from './plugins/McpDetailPage';
import { PluginDetailPage } from './plugins/PluginDetailPage';
import { ConfirmDialog, ErrorBanner, PostInstallSignInDialog, useActions } from './plugins/shared';

type Tab = 'discover' | 'installed' | 'marketplaces' | 'mcp' | 'errors';

type Confirm =
  | { kind: 'uninstall'; plugin: InstalledPluginView }
  | { kind: 'market'; market: MarketplaceView }
  | { kind: 'server'; server: McpServerView };

/** Full-pane Plugins view: plugins from marketplaces (Claude Code's
 *  format) and MCP servers. `version` is bumped by App on every
 *  `extensions_changed` push so status stays live. */
export function PluginsPanel({ version = 0 }: { version?: number }) {
  const [tab, setTab] = useState<Tab>('discover');
  const [query, setQuery] = useState('');
  const [plugins, setPlugins] = useState<PluginsOverview | null>(null);
  const [mcp, setMcp] = useState<McpListView | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [detailId, setDetailId] = useState<string | null>(null);
  const [serverOpen, setServerOpen] = useState<string | null>(null);
  const [editor, setEditor] = useState<{ editing: McpServerView | null } | null>(null);
  const [confirm, setConfirm] = useState<Confirm | null>(null);
  const [localVersion, setLocalVersion] = useState(0);
  // The sign-in in progress, with its link in case the tab didn't open.
  const [signIn, setSignIn] = useState<{ name: string; url: string } | null>(null);
  // Post-install prompt: servers from the just-installed plugin that need OAuth.
  const [postInstallSignIn, setPostInstallSignIn] = useState<{
    pluginName: string;
    servers: { name: string; displayName: string }[];
  } | null>(null);
  // Plugin we're watching for MCP servers to reach needs_auth after install.
  const [watchPlugin, setWatchPlugin] = useState<{ name: string; displayName: string } | null>(null);
  // The server whose missing token(s) are being entered.
  const [tokenFor, setTokenFor] = useState<string | null>(null);
  const { busy, error, setError, run } = useActions();

  const refresh = useCallback(async () => {
    try {
      const [p, m] = await Promise.all([getPlugins(), listMcp()]);
      setPlugins(p);
      setMcp(m);
      setLoadError(null);
    } catch (e) {
      setLoadError((e as Error).message);
    }
  }, []);

  useEffect(() => {
    refresh();
  }, [refresh, version]);

  // First visit with nothing installed: land on Discover; with
  // something installed but no marketplace: Installed.
  useEffect(() => {
    if (!plugins) return;
    setTab((t) => (t === 'discover' && plugins.marketplaces.length === 0 && plugins.installed.length > 0 ? 'installed' : t));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [plugins === null]);

  // While a server is connecting, poll gently in case a push is missed.
  useEffect(() => {
    if (!mcp?.servers.some((s) => s.status.state === 'connecting')) return;
    const t = setTimeout(refresh, 1500);
    return () => clearTimeout(t);
  }, [mcp, refresh]);

  // When MCP state updates, check if the watched plugin's servers reached needs_auth.
  useEffect(() => {
    if (!watchPlugin || !mcp) return;
    const authServers = mcp.servers.filter(
      (s) => s.scope.kind === 'plugin' && s.scope.plugin === watchPlugin.name && s.status.state === 'needs_auth',
    );
    if (authServers.length === 0) return;
    const toDisplayName = (s: McpServerView) => s.name.split(':').slice(2).join(':') || s.name;
    setPostInstallSignIn({
      pluginName: watchPlugin.displayName,
      servers: authServers.map((s) => ({ name: s.name, displayName: toDisplayName(s) })),
    });
    setWatchPlugin(null);
  }, [mcp, watchPlugin]);

  const afterPlugins = (p: PluginsOverview | undefined) => {
    if (p) {
      setPlugins(p);
      setLocalVersion((v) => v + 1);
      listMcp().then(setMcp).catch(() => {});
    }
  };
  const afterMcp = (m: McpListView | undefined) => {
    if (m) setMcp(m);
  };

  const pluginActions = {
    install: (id: string) =>
      run(`install:${id}`, () => installPlugin(id)).then((p) => {
        afterPlugins(p);
        if (!p) return;
        const installed = p.installed.find((pl) => pl.id === id);
        if (!installed || installed.mcp_servers.length === 0) return;
        // Watch for this plugin's MCP servers to reach needs_auth.
        // The servers start in `connecting` and the existing polling effect
        // will refresh mcp state; the watchPlugin effect fires the dialog.
        setWatchPlugin({ name: installed.name, displayName: installed.display_name || installed.name });
      }),
    toggle: (id: string, enabled: boolean) => run(`toggle:${id}`, () => setPluginEnabled(id, enabled)).then(afterPlugins),
    uninstall: (id: string) =>
      run(`uninstall:${id}`, () => uninstallPlugin(id)).then((p) => {
        afterPlugins(p);
        setConfirm(null);
      }),
  };

  async function addMarket(source: string): Promise<boolean> {
    const p = await run(`market-add:${source}`, () => addMarketplace(source));
    afterPlugins(p);
    if (p) setTab('discover');
    return !!p;
  }

  const mcpActions = {
    onAdd: () => setEditor({ editing: null }),
    onEdit: (s: McpServerView) => setEditor({ editing: s }),
    onRemove: (s: McpServerView) => setConfirm({ kind: 'server', server: s }),
    onOpen: (s: McpServerView) => setServerOpen(s.name),
    onReconnect: (name: string) => run(`mcp:${name}`, () => reconnectMcp(name)).then(afterMcp),
    onToggle: (name: string, enabled: boolean) => run(`mcp:${name}`, () => setMcpEnabled(name, enabled)).then(afterMcp),
    onApprove: (name: string, approve: boolean) => run(`mcp:${name}`, () => setMcpApproval(name, approve)).then(afterMcp),
    onSignIn: (name: string) => {
      // Open the tab now, while this is still the user's click: a tab
      // opened after the request returns gets silently pop-up blocked.
      const tab = isDesktop() ? null : window.open('about:blank', '_blank');
      return run(`mcp:${name}`, async () => {
        try {
          const { url } = await signInMcp(name);
          if (isDesktop()) {
            await openExternal(url);
          } else if (tab && !tab.closed) {
            tab.opener = null;
            tab.location.href = url;
          }
          setSignIn({ name, url });
        } catch (e) {
          tab?.close();
          throw e;
        }
      });
    },
    onSignOut: (name: string) => run(`mcp:${name}`, () => signOutMcp(name)).then(afterMcp),
    onAddToken: (s: McpServerView) => setTokenFor(s.name),
    onToolEnabled: (tool: string, enabled: boolean) =>
      run(`tool:${tool}`, () => setMcpToolEnabled(tool, enabled)).then(afterMcp),
    onToolLoading: (mode: ToolLoading) => run('tool-loading', () => setMcpToolLoading(mode)).then(afterMcp),
  };

  const failedServers = mcp?.servers.filter((s) => s.status.state === 'failed') ?? [];
  const attention = mcp?.servers.filter((s) => ['needs_auth', 'needs_approval'].includes(s.status.state)).length ?? 0;
  const pluginProblems = plugins?.installed.filter((p) => p.problems.length > 0) ?? [];
  const marketErrors = plugins?.marketplaces.filter((m) => m.error) ?? [];
  const configProblems = mcp?.problems.filter((p) => !p.message.startsWith('overridden')) ?? [];
  const errorCount = failedServers.length + pluginProblems.length + marketErrors.length + configProblems.length;

  const tabs: { key: Tab; label: string; count?: number; dot?: 'amber' | 'red' }[] = [
    { key: 'discover', label: 'Discover' },
    { key: 'installed', label: 'Installed', count: plugins?.installed.length },
    { key: 'marketplaces', label: 'Marketplaces', count: plugins?.marketplaces.length },
    {
      key: 'mcp',
      label: 'MCP servers',
      count: mcp?.servers.length,
      dot: failedServers.length ? 'red' : attention ? 'amber' : undefined,
    },
    ...(errorCount > 0 ? [{ key: 'errors' as Tab, label: 'Errors', count: errorCount, dot: 'red' as const }] : []),
  ];

  const openServer = mcp?.servers.find((s) => s.name === serverOpen) ?? null;
  const tokenServer = mcp?.servers.find((s) => s.name === tokenFor) ?? null;

  return (
    <div className="min-h-full">
    <div className="mx-auto flex w-full max-w-4xl flex-col gap-6 px-6 pb-12 pt-6">

    {detailId ? (
      <>
        <PluginDetailPage
          id={detailId}
          version={version + localVersion}
          busy={busy}
          onBack={() => setDetailId(null)}
          onInstall={pluginActions.install}
          onToggle={pluginActions.toggle}
          onUninstall={(id) => {
            const p = plugins?.installed.find((x) => x.id === id);
            if (p) setConfirm({ kind: 'uninstall', plugin: p });
          }}
          actionError={error}
        />
        {confirm?.kind === 'uninstall' && (
          <ConfirmDialog
            title={`Uninstall ${confirm.plugin.display_name || confirm.plugin.name}?`}
            body="Its commands, agents, skills and MCP servers are removed. You can install it again from its marketplace."
            confirmLabel="Uninstall"
            busy={!!busy}
            onCancel={() => setConfirm(null)}
            onConfirm={() => {
              setDetailId(null);
              pluginActions.uninstall(confirm.plugin.id);
            }}
          />
        )}
      </>
    ) : serverOpen && openServer ? (
      <>
        <McpDetailPage
          server={openServer}
          busy={busy}
          onBack={() => setServerOpen(null)}
          actions={{
            ...mcpActions,
            onOpen: (s) => setServerOpen(s.name),
            onRemove: (s) => { setConfirm({ kind: 'server', server: s }); },
          }}
        />
        {confirm?.kind === 'server' && (
          <ConfirmDialog
            title={`Remove ${confirm.server.name}?`}
            body={
              confirm.server.scope.kind === 'project'
                ? "It's removed from the project's .mcp.json, for everyone using the repo."
                : 'Its tools stop being available right away.'
            }
            confirmLabel="Remove"
            busy={!!busy}
            onCancel={() => setConfirm(null)}
            onConfirm={() => {
              const scope = writeScope(confirm.server);
              if (!scope) return;
              run(`mcp:${confirm.server.name}`, () => deleteMcpServer(confirm.server.name, scope)).then((m) => {
                afterMcp(m);
                setConfirm(null);
                setServerOpen(null);
              });
            }}
          />
        )}
      </>
    ) : (
      <>
      <header className="flex flex-wrap items-center justify-between gap-4">
        <div className="flex items-center gap-3">
          <div className="flex size-9 items-center justify-center rounded-xl bg-mira-purple/15">
            <Puzzle className="size-4.5 text-mira-purple" />
          </div>
          <div>
            <h1 className="text-[18px] font-semibold tracking-tight">Plugins</h1>
            <p className="text-[12px] text-muted-foreground">
              Commands, agents, skills and MCP servers.
            </p>
          </div>
        </div>
        <label className="relative w-full sm:w-60">
          <Search className="pointer-events-none absolute left-2.5 top-1/2 size-3.5 -translate-y-1/2 text-muted-foreground/60" />
          <input
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder={tab === 'mcp' ? 'Search servers…' : 'Search plugins…'}
            className="h-8 w-full rounded-full border border-border/60 bg-white/[0.03] pl-8 pr-3 text-[12.5px] outline-none placeholder:text-muted-foreground/50 focus:border-border focus:bg-white/[0.05]"
          />
        </label>
      </header>

      <nav className="flex gap-1 overflow-x-auto border-b border-border/25 pb-3">
        {tabs.map((t) => (
          <button
            key={t.key}
            type="button"
            onClick={() => setTab(t.key)}
            className={cn(
              'flex shrink-0 items-center gap-1.5 rounded-full px-4 py-1.5 text-[12.5px] font-medium transition-all',
              tab === t.key
                ? 'bg-white text-black shadow-sm'
                : 'text-muted-foreground hover:bg-white/8 hover:text-foreground',
            )}
          >
            {t.label}
            {t.count !== undefined && t.count > 0 && (
              <span className={cn(
                'rounded-full px-1.5 text-[11px]',
                tab === t.key ? 'bg-black/10 text-black/60' : 'bg-white/8 text-muted-foreground',
              )}>
                {t.count}
              </span>
            )}
            {t.dot && (
              <span className={cn('size-1.5 rounded-full', t.dot === 'red' ? 'bg-destructive' : 'bg-amber-400')} />
            )}
          </button>
        ))}
      </nav>

      {loadError && <ErrorBanner message={loadError} />}
      {error && <ErrorBanner message={error} onDismiss={() => setError(null)} />}
      {signIn && mcp?.servers.find((s) => s.name === signIn.name)?.status.state !== 'connected' && (
        <div className="flex items-center gap-3 rounded-lg border border-mira-blue/30 bg-mira-blue/[0.06] px-3 py-2 text-[13px]">
          <span className="min-w-0 flex-1">
            Finish signing in to <b>{signIn.name}</b> in your browser. This page updates when you’re done.
          </span>
          <a
            href={signIn.url}
            target="_blank"
            rel="noreferrer"
            className="shrink-0 text-mira-blue underline-offset-2 hover:underline"
            onClick={(e) => {
              if (isDesktop()) {
                e.preventDefault();
                openExternal(signIn.url);
              }
            }}
          >
            Open sign-in page
          </a>
          <button type="button" onClick={() => setSignIn(null)} className="shrink-0 text-muted-foreground hover:text-foreground">
            Dismiss
          </button>
        </div>
      )}

      {!plugins || !mcp ? (
        !loadError && <div className="py-10 text-center text-[13px] text-muted-foreground">Loading…</div>
      ) : (
        <>
          {tab === 'discover' && (
            <DiscoverTab
              data={plugins}
              query={query}
              busy={busy}
              onOpen={setDetailId}
              onInstall={pluginActions.install}
              onAddSuggested={addMarket}
            />
          )}
          {tab === 'installed' && (
            <InstalledTab
              data={plugins}
              query={query}
              busy={busy}
              onOpen={setDetailId}
              onToggle={pluginActions.toggle}
              onUpdate={pluginActions.install}
              onUninstall={(p) => setConfirm({ kind: 'uninstall', plugin: p })}
              onBrowse={() => setTab('discover')}
            />
          )}
          {tab === 'marketplaces' && (
            <MarketplacesTab
              data={plugins}
              busy={busy}
              onAdd={addMarket}
              onUpdate={(name) => run(`market-update:${name}`, () => updateMarketplace(name)).then(afterPlugins)}
              onRemove={(m) => setConfirm({ kind: 'market', market: m })}
            />
          )}
          {tab === 'mcp' && <McpTab data={mcp} query={query} busy={busy} actions={mcpActions} />}
          {tab === 'errors' && (
            <div className="flex flex-col gap-2">
              {failedServers.map((s) => (
                <ErrorRow key={s.name} title={`MCP server ${s.name}`} message={s.status.state === 'failed' ? s.status.message : ''} onClick={() => setServerOpen(s.name)} />
              ))}
              {configProblems.map((p, i) => (
                <ErrorRow key={`c${i}`} title={p.server ? `${p.server} (${p.source})` : p.source} message={p.message} />
              ))}
              {pluginProblems.flatMap((p) =>
                p.problems.map((m) => <ErrorRow key={p.id + m} title={p.id} message={m} onClick={() => setDetailId(p.id)} />),
              )}
              {marketErrors.map((m) => (
                <ErrorRow key={m.name} title={`Marketplace ${m.name}`} message={m.error ?? ''} />
              ))}
            </div>
          )}
        </>
      )}

      {postInstallSignIn && (
        <PostInstallSignInDialog
          pluginName={postInstallSignIn.pluginName}
          servers={postInstallSignIn.servers}
          busy={!!busy}
          onSignIn={(name) => {
            setPostInstallSignIn(null);
            setTab('mcp');
            mcpActions.onSignIn(name);
          }}
          onSkip={() => setPostInstallSignIn(null)}
        />
      )}

      {tokenServer && (
        <TokenDialog
          s={tokenServer}
          onClose={() => setTokenFor(null)}
          onSave={async (values) => {
            const m = await run(`mcp:${tokenServer.name}`, async () => {
              let last: McpListView | undefined;
              for (const [name, value] of Object.entries(values)) {
                if (value.trim()) last = await setMcpVariable(name, value.trim());
              }
              return last;
            });
            afterMcp(m);
            return !!m;
          }}
        />
      )}

      {editor && (
        <ServerEditorDialog
          editing={editor.editing}
          hasProject={!!mcp?.project}
          onClose={() => setEditor(null)}
          onSave={async (req) => {
            const m = await run('mcp-save', () => saveMcpServer(req));
            afterMcp(m);
            return !!m;
          }}
        />
      )}

      {confirm?.kind === 'uninstall' && (
        <ConfirmDialog
          title={`Uninstall ${confirm.plugin.display_name || confirm.plugin.name}?`}
          body="Its commands, agents, skills and MCP servers are removed. You can install it again from its marketplace."
          confirmLabel="Uninstall"
          busy={!!busy}
          onCancel={() => setConfirm(null)}
          onConfirm={() => {
            setDetailId(null);
            pluginActions.uninstall(confirm.plugin.id);
          }}
        />
      )}
      {confirm?.kind === 'market' && (
        <ConfirmDialog
          title={`Remove ${confirm.market.name}?`}
          body="Plugins installed from this marketplace are uninstalled too."
          confirmLabel="Remove"
          busy={!!busy}
          onCancel={() => setConfirm(null)}
          onConfirm={() =>
            run(`market-remove:${confirm.market.name}`, () => removeMarketplace(confirm.market.name)).then((p) => {
              afterPlugins(p);
              setConfirm(null);
            })
          }
        />
      )}
    </> /* end normal panel */
    )}
    </div>
    </div>
  );
}

function ErrorRow({ title, message, onClick }: { title: string; message: string; onClick?: () => void }) {
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={!onClick}
      className="flex items-start gap-3 rounded-xl border border-destructive/25 bg-destructive/5 px-4 py-3 text-left enabled:hover:bg-destructive/10"
    >
      <AlertCircle className="mt-0.5 size-4 shrink-0 text-destructive" />
      <div className="min-w-0">
        <div className="text-[13px] font-medium">{title}</div>
        <div className="break-words text-[12.5px] text-destructive/80">{message}</div>
      </div>
    </button>
  );
}
