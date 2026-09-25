/**
 * Settings → Integrations: GitHub (the Runmira-bot app, or a personal
 * token) and Slack.
 */
import React, { useEffect, useMemo, useState } from 'react';
import {
  ArrowClockwise,
  ArrowSquareOut,
  ChatCircleDots,
  CheckCircle,
  CircleNotch,
  GithubLogo,
  GitPullRequest,
  Lock,
  MagnifyingGlass,
  Plus,
  ShieldCheck,
  SlackLogo,
  Warning,
} from '@phosphor-icons/react';
import {
  connectGithub,
  getGithubConnect,
  type GithubConnectReport,
  type GithubConnectView,
} from '../api';
import {
  appStatus,
  createApp,
  githubAppAvailable,
  listInstallations,
  repoToken,
  startInstall,
  type AppInstallations,
  type AppRepo,
  type AppStatus,
} from '../lib/githubApp';
import { SectionInput } from '@/components/ui/input';
import { Button } from '@/components/ui/button';
import { cn } from '@/lib/utils';

/** What people write in a comment to call Mira. */
const TRIGGER = '@runmira-bot';

/** Result of coming back from GitHub (`?github=…` on the app URL). */
export type GithubReturn = { ok: boolean; message: string } | null;

export function IntegrationsSection({
  onOpenKeys, githubReturn,
}: {
  onOpenKeys: () => void;
  githubReturn?: GithubReturn;
}) {
  return (
    <div className="flex flex-col gap-5">
      <div className="px-1">
        <div className="text-[18px] font-semibold tracking-tight text-foreground">Integrations</div>
        <div className="mt-1 text-[12.5px] text-muted-foreground/85">
          Use Mira where your team already works. Nothing to edit by hand.
        </div>
      </div>

      {githubAppAvailable() ? (
        <GithubAppCard githubReturn={githubReturn} onOpenKeys={onOpenKeys} />
      ) : (
        <Card>
          <CardHeader
            icon={<GithubLogo weight="fill" className="size-5" />}
            title="GitHub"
            subtitle={<>Reviews every pull request and works on <Code>{TRIGGER}</Code> requests.</>}
          />
          <div className="border-t border-border/40 px-5 py-4">
            <TokenConnect onOpenKeys={onOpenKeys} />
          </div>
        </Card>
      )}

      <SlackCard onOpenKeys={onOpenKeys} />
    </div>
  );
}

/* ---------- building blocks ---------- */

function Card({ children }: { children: React.ReactNode }) {
  return (
    <div className="overflow-hidden rounded-2xl border border-border/50 bg-mira-elev1/60">
      {children}
    </div>
  );
}

function CardHeader({
  icon, title, subtitle, right,
}: {
  icon: React.ReactNode;
  title: string;
  subtitle: React.ReactNode;
  right?: React.ReactNode;
}) {
  return (
    <div className="flex items-start gap-3.5 px-5 py-4">
      <div className="flex size-10 shrink-0 items-center justify-center rounded-xl border border-border/50 bg-background/60 text-foreground">
        {icon}
      </div>
      <div className="min-w-0 flex-1">
        <div className="flex min-h-8 flex-wrap items-center justify-between gap-x-3 gap-y-2">
          <div className="text-[14px] font-semibold text-foreground">{title}</div>
          {right && <div className="flex items-center gap-2">{right}</div>}
        </div>
        <div className="mt-0.5 text-[12.5px] leading-relaxed text-muted-foreground">{subtitle}</div>
      </div>
    </div>
  );
}

function Code({ children }: { children: React.ReactNode }) {
  return (
    <code className="rounded bg-muted/70 px-1 py-px font-mono text-[11.5px] text-foreground/90">
      {children}
    </code>
  );
}

function StatusPill({ on, label }: { on: boolean; label: string }) {
  return (
    <span
      className={cn(
        'inline-flex items-center gap-1.5 rounded-full border px-2 py-0.5 text-[11px] font-medium',
        on
          ? 'border-emerald-500/30 bg-emerald-500/10 text-emerald-600 dark:text-emerald-400'
          : 'border-border/60 text-muted-foreground',
      )}
    >
      <span className={cn('size-1.5 rounded-full', on ? 'bg-emerald-500' : 'bg-muted-foreground/50')} />
      {label}
    </span>
  );
}

function Avatar({ login, className }: { login: string; className?: string }) {
  const [failed, setFailed] = useState(false);
  if (failed) {
    return (
      <span className={cn('flex items-center justify-center rounded-md bg-muted text-[10px] font-semibold uppercase text-muted-foreground', className)}>
        {login.slice(0, 1)}
      </span>
    );
  }
  return (
    <img
      src={`https://github.com/${encodeURIComponent(login)}.png?size=64`}
      alt=""
      loading="lazy"
      onError={() => setFailed(true)}
      className={cn('rounded-md bg-muted object-cover', className)}
    />
  );
}

function Banner({ tone, children }: { tone: 'ok' | 'warn' | 'error'; children: React.ReactNode }) {
  const Icon = tone === 'ok' ? CheckCircle : Warning;
  return (
    <div
      className={cn(
        'flex items-start gap-2 rounded-lg px-3 py-2 text-[12px]',
        tone === 'ok' && 'bg-emerald-500/10 text-emerald-700 dark:text-emerald-400',
        tone === 'warn' && 'bg-amber-500/10 text-amber-700 dark:text-amber-400',
        tone === 'error' && 'bg-destructive/10 text-destructive',
      )}
    >
      <Icon weight="fill" className="mt-px size-3.5 shrink-0" />
      <div className="min-w-0">{children}</div>
    </div>
  );
}

const message = (e: unknown) => (e instanceof Error ? e.message : String(e));

/* ---------- GitHub: the Runmira-bot app ---------- */

type Note = { ok: boolean; text: string };

/** Install the Runmira-bot GitHub App, then turn Mira on per repository. */
function GithubAppCard({
  githubReturn, onOpenKeys,
}: {
  githubReturn?: GithubReturn;
  onOpenKeys: () => void;
}) {
  const [status, setStatus] = useState<AppStatus | null>(null);
  const [data, setData] = useState<AppInstallations | null>(null);
  const [settings, setSettings] = useState<GithubConnectView | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [notes, setNotes] = useState<Record<string, Note>>({});
  const [query, setQuery] = useState('');
  const [account, setAccount] = useState<string | null>(null);
  // True until the first answer (status + repositories) is in; later
  // reloads keep what's on screen and only show `refreshing`.
  const [loading, setLoading] = useState(true);
  const [refreshing, setRefreshing] = useState(false);
  const lastLoad = React.useRef(0);

  async function load() {
    setError(null);
    setRefreshing(true);
    lastLoad.current = Date.now();
    try {
      const st = await appStatus();
      setStatus(st);
      setSettings(await getGithubConnect().catch(() => null));
      if (st.configured) setData(await listInstallations());
    } catch (e) {
      setError(message(e));
    } finally {
      setLoading(false);
      setRefreshing(false);
    }
  }
  useEffect(() => { void load(); }, []);
  // Back from GitHub in another tab: refresh (not on every quick focus).
  useEffect(() => {
    const onFocus = () => {
      if (Date.now() - lastLoad.current > 5_000) void load();
    };
    window.addEventListener('focus', onFocus);
    return () => window.removeEventListener('focus', onFocus);
  }, []);

  async function install() {
    setBusy('install');
    setError(null);
    try {
      const { url } = await startInstall(window.location.origin + window.location.pathname);
      window.location.href = url;
    } catch (e) {
      setError(message(e));
      setBusy(null);
    }
  }

  async function enable(installation: number, repo: string) {
    setBusy(repo);
    setError(null);
    try {
      const { token } = await repoToken(installation, repo);
      const report = await connectGithub(repo, token);
      setNotes((n) => ({
        ...n,
        [repo]: report.pull_request
          ? { ok: true, text: `The default branch is protected: merge ${report.pull_request} to finish.` }
          : { ok: true, text: `On. New pull requests get reviewed; mention ${TRIGGER} to give it a task.` },
      }));
      await load();
    } catch (e) {
      setNotes((n) => ({ ...n, [repo]: { ok: false, text: message(e) } }));
    } finally {
      setBusy(null);
    }
  }

  const installs = data?.installations ?? [];
  const connected = installs.length > 0;
  const blocked = settings?.settings_problem ?? null;

  const rows = useMemo(() => {
    const q = query.trim().toLowerCase();
    return installs
      .filter((i) => !account || i.account === account)
      .flatMap((i) => i.repos.map((r) => ({ installation: i.id, repo: r })))
      .filter(({ repo }) => !q || repo.full_name.toLowerCase().includes(q))
      // Active first, then the ones you can turn on, then the rest.
      .sort((a, b) => rank(a.repo) - rank(b.repo) || a.repo.full_name.localeCompare(b.repo.full_name));
  }, [installs, query, account]);
  const total = installs.reduce((n, i) => n + i.repos.length, 0);
  const active = installs.reduce((n, i) => n + i.repos.filter((r) => r.enabled).length, 0);

  const header = (
    <CardHeader
      icon={<GithubLogo weight="fill" className="size-5" />}
      title="GitHub"
      subtitle={
        <>Reviews every new pull request and works on <Code>{TRIGGER}</Code> requests in
          issues and pull requests, in each repository’s own GitHub Actions.</>
      }
      right={
        loading ? null : connected ? (
          <>
            <StatusPill on label={active > 0 ? `${active} active` : 'Connected'} />
            <Button variant="outline" className="h-8 gap-1.5 px-2.5 text-[12px]" onClick={() => void install()} disabled={busy !== null}>
              {busy === 'install' ? <CircleNotch className="size-3.5 animate-spin" /> : <Plus className="size-3.5" />}
              Add repositories
            </Button>
          </>
        ) : (
          <StatusPill on={false} label="Not connected" />
        )
      }
    />
  );

  return (
    <Card>
      {header}

      {(githubReturn || blocked || error) && (
        <div className="flex flex-col gap-2 px-5 pb-3">
          {githubReturn && <Banner tone={githubReturn.ok ? 'ok' : 'error'}>{githubReturn.message}</Banner>}
          {blocked && (
            <Banner tone="warn">
              {blocked}. Mira stores your model key in each repository, so it needs one first.
            </Banner>
          )}
          {error && <Banner tone="error">{error}</Banner>}
        </div>
      )}

      <div className="border-t border-border/40">
        {loading ? (
          <RepoSkeleton />
        ) : status && !status.configured ? (
          <div className="px-5 py-4"><CreateGithubApp /></div>
        ) : !connected ? (
          <ConnectEmptyState busy={busy === 'install'} onConnect={() => void install()} />
        ) : (
          <>
            <div className="flex flex-wrap items-center gap-2 px-5 py-3">
              <div className="relative min-w-[180px] flex-1">
                <MagnifyingGlass className="pointer-events-none absolute left-2.5 top-1/2 size-3.5 -translate-y-1/2 text-muted-foreground" />
                <SectionInput
                  value={query}
                  onChange={(e) => setQuery(e.target.value)}
                  placeholder="Search repositories"
                  spellCheck={false}
                  className="h-8 pl-8 text-[12.5px]"
                />
              </div>
              {installs.length > 1 && (
                <div className="order-last flex w-full items-center gap-1 overflow-x-auto rounded-lg border border-border/50 p-0.5 sm:order-none sm:w-auto">
                  <AccountTab selected={account === null} onClick={() => setAccount(null)}>All</AccountTab>
                  {installs.map((i) => (
                    <AccountTab key={i.id} selected={account === i.account} onClick={() => setAccount(i.account)}>
                      <Avatar login={i.account} className="size-4" />
                      {i.account}
                    </AccountTab>
                  ))}
                </div>
              )}
              <button
                type="button"
                title="Refresh"
                onClick={() => void load()}
                disabled={refreshing}
                className="flex size-8 items-center justify-center rounded-md border border-border/50 text-muted-foreground transition-colors hover:bg-muted/50 hover:text-foreground disabled:opacity-60"
              >
                <ArrowClockwise className={cn('size-3.5', refreshing && 'animate-spin')} />
              </button>
            </div>

            <div className="border-t border-border/40">
              {rows.length === 0 ? (
                <div className="px-5 py-8 text-center text-[12.5px] text-muted-foreground">
                  {total === 0
                    ? 'No repositories are shared with Runmira-bot yet. Use Add repositories to pick some.'
                    : 'No repositories match.'}
                </div>
              ) : (
                rows.map(({ installation, repo }) => (
                  <RepoRow
                    key={repo.full_name}
                    repo={repo}
                    note={notes[repo.full_name]}
                    busy={busy === repo.full_name}
                    disabled={busy !== null || !!blocked}
                    onEnable={() => void enable(installation, repo.full_name)}
                  />
                ))
              )}
            </div>

            <div className="flex items-center justify-between gap-3 border-t border-border/40 px-5 py-2.5 text-[11.5px] text-muted-foreground">
              <span>{total} {total === 1 ? 'repository' : 'repositories'} · {active} active</span>
              {data?.manage_url && (
                <a
                  href={data.manage_url}
                  target="_blank"
                  rel="noreferrer"
                  className="inline-flex items-center gap-1 hover:text-foreground"
                >
                  Manage on GitHub <ArrowSquareOut className="size-3" />
                </a>
              )}
            </div>
          </>
        )}
      </div>

      <details className="group border-t border-border/40 px-5 py-3 text-[12px] text-muted-foreground">
        <summary className="cursor-pointer select-none list-none hover:text-foreground">
          <span className="inline-block transition-transform group-open:rotate-90">›</span>{' '}
          How it works, and connecting with a personal access token
        </summary>
        <div className="mt-2 flex flex-col gap-3 pl-3">
          <p className="leading-relaxed">
            Turning a repository on stores your model key as the encrypted Actions secret{' '}
            <Code>MIRA_API_KEY</Code>, your provider and model as Actions variables, and adds{' '}
            <Code>.github/workflows/mira.yml</Code>. Reviews, comments and pull requests come from
            the Runmira-bot app. Only people with write access can start tasks. Changed your model
            or key? Click <span className="text-foreground">Sync</span>.
          </p>
          <TokenConnect onOpenKeys={onOpenKeys} />
        </div>
      </details>
    </Card>
  );
}

const rank = (r: AppRepo) => (r.enabled ? 0 : r.can_manage ? 1 : 2);

function AccountTab({
  selected, onClick, children,
}: {
  selected: boolean;
  onClick: () => void;
  children: React.ReactNode;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      className={cn(
        'flex h-7 shrink-0 items-center gap-1.5 rounded-md px-2 text-[12px] transition-colors',
        selected ? 'bg-muted text-foreground' : 'text-muted-foreground hover:text-foreground',
      )}
    >
      {children}
    </button>
  );
}

function RepoRow({
  repo, note, busy, disabled, onEnable,
}: {
  repo: AppRepo;
  note?: Note;
  busy: boolean;
  disabled: boolean;
  onEnable: () => void;
}) {
  const [owner, name] = repo.full_name.split('/');
  return (
    <div className="flex items-center gap-3 border-b border-border/30 px-5 py-3 transition-colors last:border-b-0 hover:bg-muted/20">
      <Avatar login={owner} className="size-8" />
      <div className="min-w-0 flex-1">
        <div className="flex items-center gap-1.5 text-[13px]">
          <a
            href={`https://github.com/${repo.full_name}`}
            target="_blank"
            rel="noreferrer"
            className="truncate hover:underline"
          >
            <span className="text-muted-foreground">{owner} / </span>
            <span className="font-medium text-foreground">{name}</span>
          </a>
          {repo.private && <Lock className="size-3 shrink-0 text-muted-foreground" aria-label="Private" />}
        </div>
        <div
          className={cn(
            'mt-0.5 flex items-center gap-1.5 truncate text-[11.5px]',
            note && !note.ok ? 'text-destructive' : 'text-muted-foreground',
          )}
        >
          {note ? (
            note.text
          ) : repo.enabled ? (
            <>
              <span className="size-1.5 shrink-0 rounded-full bg-emerald-500" />
              Reviewing pull requests · answers {TRIGGER}
            </>
          ) : repo.can_manage ? (
            'Not set up'
          ) : (
            'Only a repository admin can turn Mira on here'
          )}
        </div>
      </div>
      <div className="shrink-0">
        {!repo.can_manage ? (
          <span
            className="rounded-full border border-border/60 px-2 py-0.5 text-[11px] text-muted-foreground"
            title="Turning Mira on stores a secret in the repository, which needs admin access."
          >
            Needs admin
          </span>
        ) : repo.enabled ? (
          <Button
            variant="ghost"
            className="h-8 gap-1.5 px-2.5 text-[12px] text-muted-foreground hover:text-foreground"
            disabled={disabled}
            onClick={onEnable}
            title="Store your current model, provider and key, and refresh the workflow"
          >
            {busy ? <CircleNotch className="size-3.5 animate-spin" /> : <ArrowClockwise className="size-3.5" />}
            {busy ? 'Syncing…' : 'Sync'}
          </Button>
        ) : (
          <Button className="h-8 px-3 text-[12px]" disabled={disabled} onClick={onEnable}>
            {busy && <CircleNotch className="size-3.5 animate-spin" />}
            {busy ? 'Setting up…' : 'Turn on'}
          </Button>
        )}
      </div>
    </div>
  );
}

function RepoSkeleton() {
  return (
    <div aria-busy="true">
      <div className="flex items-center gap-2 px-5 py-3 text-[12px] text-muted-foreground">
        <CircleNotch className="size-3.5 animate-spin" /> Checking your GitHub connection…
      </div>
      {[0, 1, 2].map((i) => (
        <div key={i} className="flex items-center gap-3 border-t border-border/30 px-5 py-3">
          <div className="size-8 animate-pulse rounded-md bg-muted" />
          <div className="flex-1 space-y-1.5">
            <div className="h-3 w-40 animate-pulse rounded bg-muted" />
            <div className="h-2.5 w-24 animate-pulse rounded bg-muted/70" />
          </div>
          <div className="h-8 w-20 animate-pulse rounded-md bg-muted" />
        </div>
      ))}
    </div>
  );
}

function ConnectEmptyState({ busy, onConnect }: { busy: boolean; onConnect: () => void }) {
  const points = [
    { icon: GitPullRequest, text: 'Reviews every new pull request, with comments on the changed lines' },
    { icon: ChatCircleDots, text: <>Mention <Code>{TRIGGER}</Code> in an issue or PR and it opens a pull request</> },
    { icon: ShieldCheck, text: 'Runs in your own GitHub Actions with your model key' },
  ];
  return (
    <div className="flex flex-col items-center px-6 py-8 text-center">
      <div className="text-[15px] font-semibold text-foreground">Connect your repositories</div>
      <p className="mt-1 max-w-sm text-[12.5px] text-muted-foreground">
        Install the Runmira-bot app on your account or an organization and pick the
        repositories Mira can work in.
      </p>
      <Button className="mt-4 gap-2" onClick={onConnect} disabled={busy}>
        {busy ? <CircleNotch className="size-4 animate-spin" /> : <GithubLogo weight="fill" className="size-4" />}
        {busy ? 'Opening GitHub…' : 'Connect GitHub'}
      </Button>
      <ul className="mt-6 grid w-full max-w-md gap-2 text-left">
        {points.map(({ icon: Icon, text }, i) => (
          <li key={i} className="flex items-start gap-2.5 text-[12px] text-muted-foreground">
            <Icon className="mt-px size-4 shrink-0 text-foreground/70" />
            <span>{text}</span>
          </li>
        ))}
      </ul>
    </div>
  );
}

/** Shown until the app exists: the owner creates it once from here. */
function CreateGithubApp() {
  const [key, setKey] = useState('');
  const [org, setOrg] = useState('runmira');
  const [name, setName] = useState('Runmira-bot');
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  async function create() {
    setBusy(true);
    setError(null);
    try {
      await createApp(key.trim(), org, name.trim() || 'Runmira-bot');
    } catch (e) {
      setError(message(e));
      setBusy(false);
    }
  }
  return (
    <div className="flex flex-col gap-2 text-[12px] text-muted-foreground">
      <div>
        The Mira GitHub App hasn’t been created yet. If you run this Mira service, create it
        once with your setup key.
      </div>
      <div className="grid grid-cols-1 gap-2 sm:grid-cols-3">
        <SectionInput value={key} onChange={(e) => setKey(e.target.value)} placeholder="Setup key" spellCheck={false} />
        <SectionInput value={org} onChange={(e) => setOrg(e.target.value)} placeholder="GitHub organization" spellCheck={false} />
        <SectionInput value={name} onChange={(e) => setName(e.target.value)} placeholder="App name" spellCheck={false} />
      </div>
      <Button className="self-start" onClick={() => void create()} disabled={busy || !key.trim()}>
        {busy ? 'Opening GitHub…' : 'Create the GitHub App'}
      </Button>
      {error && <div className="text-destructive">{error}</div>}
    </div>
  );
}

/* ---------- GitHub: personal token ---------- */

/** Fallback: connect one repository with a personal access token. */
function TokenConnect({ onOpenKeys }: { onOpenKeys: () => void }) {
  const [repo, setRepo] = useState('');
  const [info, setInfo] = useState<GithubConnectView | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [done, setDone] = useState<GithubConnectReport | null>(null);

  async function load(r?: string) {
    setError(null);
    try {
      const v = await getGithubConnect(r);
      setInfo(v);
      if (!r && v.repo) setRepo(v.repo);
    } catch (e) {
      setError(message(e));
    }
  }
  useEffect(() => { void load(); }, []);

  async function connect() {
    setBusy(true);
    setError(null);
    setDone(null);
    try {
      setDone(await connectGithub(repo.trim() || undefined));
      await load(repo.trim() || undefined);
    } catch (e) {
      setError(message(e));
    } finally {
      setBusy(false);
    }
  }

  const connected = !!info?.status?.workflow && !!info?.status?.api_key_secret;
  return (
    <div className="flex flex-col gap-2">
      <div className="flex gap-2">
        <SectionInput
          value={repo}
          onChange={(e) => setRepo(e.target.value)}
          onBlur={() => void load(repo.trim() || undefined)}
          placeholder="owner/repository"
          spellCheck={false}
          className="h-8 text-[12.5px]"
        />
        <Button className="h-8 px-3 text-[12px]" onClick={() => void connect()} disabled={busy || !!info?.problem || !repo.trim()}>
          {busy && <CircleNotch className="size-3.5 animate-spin" />}
          {busy ? 'Connecting…' : connected ? 'Sync' : 'Connect'}
        </Button>
      </div>
      {info?.problem && (
        <div className="text-[12px] text-muted-foreground">
          {info.problem}
          {!info.has_token && (
            <button type="button" className="ml-1 text-foreground underline underline-offset-2" onClick={onOpenKeys}>
              Add a GitHub token
            </button>
          )}
        </div>
      )}
      {error && <div className="text-[12px] text-destructive">{error}</div>}
      {done && (
        <div className="text-[12px] text-muted-foreground">
          {done.pull_request ? (
            <>The default branch is protected, so the setup is in a pull request:{' '}
              <a className="underline" href={done.pull_request} target="_blank" rel="noreferrer">
                merge it
              </a> to finish.</>
          ) : done.workflow_unchanged ? (
            <>Updated the key and settings for {done.repo}.</>
          ) : (
            <>Connected {done.repo}. Open a pull request or mention {TRIGGER} to try it.</>
          )}
        </div>
      )}
    </div>
  );
}

/* ---------- Slack ---------- */

function SlackCard({ onOpenKeys }: { onOpenKeys: () => void }) {
  const steps: React.ReactNode[] = [
    <>Create a Slack app from Mira’s manifest (see <Code>docs/slack.md</Code>) and install it to your workspace.</>,
    <>
      Paste its bot and app tokens under{' '}
      <button type="button" className="text-foreground underline underline-offset-2" onClick={onOpenKeys}>
        Search &amp; keys
      </button>.
    </>,
    <>Run <Code>mira slack</Code> in the project folder, then mention the bot in a channel or send it a direct message.</>,
  ];
  return (
    <Card>
      <CardHeader
        icon={<SlackLogo weight="fill" className="size-5" />}
        title="Slack"
        subtitle="Ask Mira for changes from a channel or a direct message. It replies in the thread."
      />
      <ol className="flex flex-col gap-2.5 border-t border-border/40 px-5 py-4">
        {steps.map((s, i) => (
          <li key={i} className="flex items-start gap-3 text-[12.5px] leading-relaxed text-muted-foreground">
            <span className="flex size-5 shrink-0 items-center justify-center rounded-full border border-border/60 text-[10.5px] font-medium text-foreground/80">
              {i + 1}
            </span>
            <span>{s}</span>
          </li>
        ))}
      </ol>
    </Card>
  );
}
