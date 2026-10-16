import { Button } from '@/components/ui/button';
import { SectionInput } from '@/components/ui/input';
import { cn } from '@/lib/utils';
import { Check, ExternalLink, Key, MoreHorizontal, PenLine, Trash2 } from 'lucide-react';
import React, { useState } from 'react';
import type { KeyView, SettingsView } from '../../types';
import { BrandIcon } from '../AgentIcon';
import { Menu, MenuItem } from '../ui/Menu';
import { TRow, TSection } from './SettingsFields';
import { Draft } from './types';
import { useGithubKeyStatus } from '../GithubKeyNotice';
/** Keys we know how to help with: a name, one line on what uses it, and
 *  where to get one. Anything else the server reports lands in "Other". */
const KEY_META: Record<string, { label: string; help: string; url?: string; domain?: string }> = {
  BRAVE_SEARCH_API_KEY: {
    domain: 'brave.com',
    label: 'Brave Search',
    help: 'Powers the web_search tool. Free tier: 2,000 queries a month.',
    url: 'https://brave.com/search/api/',
  },
  TAVILY_API_KEY: {
    domain: 'tavily.com',
    label: 'Tavily',
    help: 'Alternative search backend (not wired up yet).',
    url: 'https://tavily.com/',
  },
  GITHUB_TOKEN: {
    domain: 'github.com',
    label: 'GitHub token',
    help: 'Pull requests, issues and connecting repositories. Needs the repo and workflow scopes.',
    url: 'https://github.com/settings/tokens/new?scopes=repo,workflow&description=Mira',
  },
  SLACK_BOT_TOKEN: {
    domain: 'slack.com',
    label: 'Slack bot token',
    help: 'The xoxb-… token from your Slack app’s OAuth & Permissions page. Used by mira slack.',
    url: 'https://api.slack.com/apps',
  },
  SLACK_APP_TOKEN: {
    domain: 'slack.com',
    label: 'Slack app token',
    help: 'An app-level xapp-… token with connections:write. Used by mira slack.',
    url: 'https://api.slack.com/apps',
  },
};

/** Keys outside the list above, by the service their name starts with. */
const OTHER_DOMAINS: Record<string, string> = { E2B: 'e2b.dev', OPENAI: 'openai.com', ANTHROPIC: 'anthropic.com', EXA: 'exa.ai', FIRECRAWL: 'firecrawl.dev' };

function keyIcon(name: string): string[] {
  const domain = KEY_META[name]?.domain ?? OTHER_DOMAINS[name.split('_')[0]];
  return domain ? [`https://www.google.com/s2/favicons?domain=${domain}&sz=64`] : [];
}

const GROUPS: { title: string; names: string[] }[] = [
  { title: 'Web search', names: ['BRAVE_SEARCH_API_KEY', 'TAVILY_API_KEY'] },
  { title: 'GitHub', names: ['GITHUB_TOKEN'] },
  { title: 'Slack', names: ['SLACK_BOT_TOKEN', 'SLACK_APP_TOKEN'] },
];

type KeyState = 'saved' | 'shell' | 'shell-over-saved' | 'unset' | 'rejected' | 'removing';

const STATE_CHIP: Record<KeyState, { label: string; tone: string }> = {
  saved: { label: 'Saved', tone: 'bg-emerald-500/12 text-emerald-700 dark:text-emerald-300' },
  shell: { label: 'From your shell', tone: 'bg-mira-blue/12 text-mira-blue' },
  'shell-over-saved': { label: 'Shell value in use', tone: 'bg-mira-blue/12 text-mira-blue' },
  unset: { label: 'Not set', tone: 'bg-fg/[0.06] text-muted-foreground' },
  rejected: { label: 'Rejected by GitHub', tone: 'bg-red-500/12 text-red-700 dark:text-red-300' },
  removing: { label: 'Removed on save', tone: 'bg-amber-500/12 text-amber-700 dark:text-amber-300' },
};

export function KeysSection({
  view,
  draft,
  setDraft,
}: {
  view: SettingsView;
  draft: Draft;
  setDraft: (u: (d: Draft) => Draft) => void;
}) {
  const github = useGithubKeyStatus().data;

  function edit(name: string, editing: boolean, value?: string) {
    setDraft((d) => {
      const { [name]: _editing, ...keyEditing } = d.keyEditing;
      const { [name]: _value, ...keyValues } = d.keyValues;
      return {
        ...d,
        keyEditing: editing ? { ...keyEditing, [name]: true } : keyEditing,
        keyValues: value === undefined ? keyValues : { ...keyValues, [name]: value },
      };
    });
  }

  const known = new Set(GROUPS.flatMap((g) => g.names));
  const groups = [
    ...GROUPS.map((g) => ({ ...g, keys: g.names.map((n) => view.keys.find((k) => k.name === n)).filter((k) => !!k) })),
    { title: 'Other keys', names: [], keys: view.keys.filter((k) => !known.has(k.name)) },
  ].filter((g) => g.keys.length > 0);

  return (
    <div className="flex flex-col gap-6">
      <p className="max-w-xl px-1 text-[12.5px] leading-relaxed text-muted-foreground">
        Keys for the services Mira’s tools use, saved in your mira.yaml. If the same variable is set in the shell Mira
        started from, that value wins.
      </p>
      {groups.map((group) => (
        <TSection key={group.title} title={group.title}>
          {group.keys.map((k) => (
            <KeyRow
              key={k.name}
              k={k}
              editing={!!draft.keyEditing[k.name]}
              pending={draft.keyValues[k.name]}
              rejected={k.name === 'GITHUB_TOKEN' && !!github?.rejected}
              ghFallback={!!github?.cli_login}
              edit={(editing, value) => edit(k.name, editing, value)}
            />
          ))}
        </TSection>
      ))}
    </div>
  );
}

function KeyRow({
  k,
  editing,
  pending,
  rejected,
  ghFallback,
  edit,
}: {
  k: KeyView;
  editing: boolean;
  pending: string | undefined;
  rejected: boolean;
  ghFallback: boolean;
  edit: (editing: boolean, value?: string) => void;
}) {
  const [menu, setMenu] = useState(false);
  const meta = KEY_META[k.name];
  const saved = k.masked.length > 0;
  const state: KeyState =
    pending === '' && !editing
      ? 'removing'
      : rejected
        ? 'rejected'
        : saved && k.from_env
          ? 'shell-over-saved'
          : saved
            ? 'saved'
            : k.from_env
              ? 'shell'
              : 'unset';
  const chip = STATE_CHIP[state];
  const label = meta?.label ?? k.name;
  const getKey = meta?.url && (
    <a href={meta.url} target="_blank" rel="noreferrer" className="text-mira-blue hover:underline">
      {state === 'rejected' ? 'Get a new key' : 'Get one'}
    </a>
  );

  let control: React.ReactNode;
  if (editing) {
    control = (
      <>
        <SectionInput
          type="password"
          value={pending ?? ''}
          // Empty while editing means "not yet", not "clear": Remove clears.
          onChange={(e) => edit(true, e.target.value || undefined)}
          placeholder={saved ? `Replaces ${k.masked}` : 'Paste key…'}
          autoComplete="off"
          autoFocus
          spellCheck={false}
          className="h-8 w-52 font-mono text-[12px]"
        />
        <Button variant="ghost" size="sm" className="h-8 shrink-0" onClick={() => edit(false)}>
          Cancel
        </Button>
      </>
    );
  } else if (state === 'removing') {
    control = (
      <Button variant="ghost" size="sm" className="h-8" onClick={() => edit(false)}>
        Undo
      </Button>
    );
  } else if (!saved && !k.from_env) {
    control = (
      <Button variant="outline" size="sm" className="h-8" onClick={() => edit(true)}>
        Add key
      </Button>
    );
  } else {
    control = (
      <>
        <span className="max-w-[10rem] truncate font-mono text-[12px] text-muted-foreground">
          {saved ? k.masked : `$${k.name}`}
        </span>
        <div className="relative">
          <button
            type="button"
            aria-label={`${label} options`}
            aria-haspopup="menu"
            aria-expanded={menu}
            onClick={() => setMenu((v) => !v)}
            className="flex size-8 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-fg/[0.06] hover:text-foreground"
          >
            <MoreHorizontal className="size-4" />
          </button>
          <Menu open={menu} onOpenChange={setMenu} label={`${label} options`} minWidth={190}>
            <MenuItem label={saved ? 'Replace key' : 'Save a key'} icon={<PenLine className="size-3.5" />} onSelect={() => edit(true)} />
            {meta?.url && (
              <MenuItem
                label="Get a new key"
                icon={<ExternalLink className="size-3.5" />}
                onSelect={() => window.open(meta.url, '_blank', 'noreferrer')}
              />
            )}
            {saved && (
              <MenuItem
                label="Remove key"
                icon={<Trash2 className="size-3.5" />}
                className="text-red-600 dark:text-red-400"
                onSelect={() => edit(false, '')}
              />
            )}
          </Menu>
        </div>
      </>
    );
  }

  return (
    <TRow
      title={
        <span className="flex items-center gap-2.5">
          <BrandIcon urls={keyIcon(k.name)} name={label} size="sm" />
          <span>{label}</span>
          <span className={cn('rounded-full px-1.5 py-px text-[10.5px] font-medium', chip.tone)}>{chip.label}</span>
        </span>
      }
      description={
        <span className="block pl-[30px]">
          {meta?.help}
          {meta && state === 'unset' && getKey && <> {getKey}.</>}
          {!meta && <code className="font-mono text-[11px]">{k.name}</code>}
        </span>
      }
      control={control}
    >
      {state === 'rejected' && (
        <div className="ml-[30px] mt-2.5 rounded-lg border border-red-500/25 bg-red-500/[0.06] px-3 py-2 text-[12px] leading-relaxed text-red-800 dark:text-red-200">
          GitHub turned this key down: it expired, was revoked, or lacks access.{' '}
          {ghFallback
            ? 'Mira is using your gh login meanwhile. Replace the key, or remove it to rely on gh.'
            : "Pull requests and issues won't load until you replace it or run gh auth login."}{' '}
          {getKey}
        </div>
      )}
      {state === 'shell-over-saved' && (
        <div className="ml-[30px] mt-2 text-[12px] text-muted-foreground">
          ${k.name} is set in the shell Mira started from, so the saved key isn’t used.
        </div>
      )}
    </TRow>
  );
}

export type KeyStatus =
  { kind: 'none' } | { kind: 'literal'; masked: string } | { kind: 'env'; name: string };

/** Compact API-key control for a row's right column: saved badge, or
 *  an inline paste input. The row itself carries the label + hint. */
export function ApiKeyControl({
  status,
  showInput,
  value,
  onChange,
  onReplace,
  onCancelReplace,
}: {
  status: KeyStatus;
  showInput: boolean;
  value: string;
  onChange: (v: string) => void;
  onReplace: () => void;
  onCancelReplace: () => void;
}) {
  if (status.kind === 'literal' && !showInput) {
    return (
      <SavedBadge
        tone="green"
        icon={<Check className="size-3.5" />}
        label="Key saved"
        masked={status.masked}
        onClick={onReplace}
        action="Replace"
      />
    );
  }
  if (status.kind === 'env' && !showInput) {
    return (
      <SavedBadge
        tone="blue"
        icon={<Key className="size-3.5" />}
        label="From env"
        masked={`$${status.name}`}
        onClick={onReplace}
        action="Override"
      />
    );
  }
  return (
    <div className="flex w-full gap-2 sm:w-64">
      <SectionInput
        type="password"
        value={value}
        onChange={(e) => onChange(e.target.value)}
        placeholder={status.kind === 'literal' ? `replaces ${status.masked}` : 'sk-…'}
        autoComplete="off"
        spellCheck={false}
        autoFocus={showInput}
        className="h-8 text-[13px]"
      />
      {status.kind !== 'none' && (
        <Button
          variant="outline"
          size="sm"
          onClick={onCancelReplace}
          type="button"
          className="h-8 shrink-0"
        >
          Cancel
        </Button>
      )}
    </div>
  );
}

export function SavedBadge({
  tone,
  icon,
  label,
  masked,
  onClick,
  action,
}: {
  tone: 'green' | 'blue';
  icon: React.ReactNode;
  label: string;
  masked: string;
  onClick: () => void;
  action: string;
}) {
  return (
    <div
      className={cn(
        'flex items-center gap-2 rounded-md border px-2.5 py-1.5 text-[12.5px]',
        tone === 'green'
          ? 'border-emerald-500/40 bg-emerald-500/[0.06]'
          : 'border-mira-blue/40 bg-mira-blue/[0.06]',
      )}
    >
      <span className={tone === 'green' ? 'text-emerald-400' : 'text-mira-blue'}>{icon}</span>
      <span className="text-muted-foreground">{label}</span>
      <span className="flex-1 min-w-0 font-mono text-xs overflow-hidden text-ellipsis whitespace-nowrap">
        {masked}
      </span>
      <Button variant="outline" size="sm" onClick={onClick} type="button">
        {action}
      </Button>
    </div>
  );
}
