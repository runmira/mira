import { Button } from '@/components/ui/button';
import { SectionInput } from '@/components/ui/input';
import { cn } from '@/lib/utils';
import { Check, Key, Search } from 'lucide-react';
import React from 'react';
import type { SettingsView } from '../../types';
import { TRow, TSection } from './SettingsFields';
import { Draft } from './types';
/** Well-known third-party keys we know how to help configure. Extend as
 *  we add tools; anything not here still appears if the backend surfaces
 *  it — this list just adds friendly labels + signup links. */
export const KEY_META: Record<string, { label: string; help: string; url?: string }> = {
  BRAVE_SEARCH_API_KEY: {
    label: 'Brave Search',
    help: 'Powers the `web_search` tool. 2000 queries/month free tier.',
    url: 'https://api.search.brave.com/',
  },
  TAVILY_API_KEY: {
    label: 'Tavily',
    help: 'Alternative search backend (not yet wired). Free tier.',
    url: 'https://tavily.com/',
  },
  GITHUB_TOKEN: {
    label: 'GitHub',
    help: 'Powers the pull-request panel and connecting repositories (Integrations). The link opens a new-token page with the `repo` and `workflow` scopes pre-selected.',
    url: 'https://github.com/settings/tokens/new?scopes=repo,workflow&description=Mira',
  },
  SLACK_BOT_TOKEN: {
    label: 'Slack bot token',
    help: 'The xoxb-… token from your Slack app’s OAuth & Permissions page. Used by `mira slack`.',
    url: 'https://api.slack.com/apps',
  },
  SLACK_APP_TOKEN: {
    label: 'Slack app token',
    help: 'An app-level xapp-… token with the connections:write scope (Basic Information → App-Level Tokens). Used by `mira slack`.',
    url: 'https://api.slack.com/apps',
  },
};

/* ---------- section: search & keys ---------- */

export function KeysSection({
  view,
  draft,
  setDraft,
}: {
  view: SettingsView;
  draft: Draft;
  setDraft: (u: (d: Draft) => Draft) => void;
}) {
  function beginEdit(name: string) {
    setDraft((d) => ({
      ...d,
      keyEditing: { ...d.keyEditing, [name]: true },
      keyValues: { ...d.keyValues, [name]: '' },
    }));
  }
  function cancelEdit(name: string) {
    setDraft((d) => {
      const { [name]: _ignored1, ...restEditing } = d.keyEditing;
      const { [name]: _ignored2, ...restValues } = d.keyValues;
      return { ...d, keyEditing: restEditing, keyValues: restValues };
    });
  }
  function updateValue(name: string, value: string) {
    setDraft((d) => ({ ...d, keyValues: { ...d.keyValues, [name]: value } }));
  }

  return (
    <TSection
      icon={<Search className="size-3.5" />}
      title="Search & keys"
      description="Third-party API keys tools consume. Stored in your mira.yaml and exported to the process env at startup so tools pick them up transparently."
    >
      {view.keys.map((k) => {
        const meta = KEY_META[k.name];
        const editing = !!draft.keyEditing[k.name];
        const pending = draft.keyValues[k.name];
        const hasStored = k.masked.length > 0;
        return (
          <TRow
            key={k.name}
            title={
              <span className="flex flex-wrap items-baseline gap-x-2">
                <span>{meta?.label ?? k.name}</span>
                <code className="font-mono text-[10.5px] font-normal text-muted-foreground/70">
                  {k.name}
                </code>
              </span>
            }
            description={meta?.help}
            control={
              <span className="flex items-center gap-2">
                {meta?.url && (
                  <a
                    href={meta.url}
                    target="_blank"
                    rel="noreferrer"
                    className="shrink-0 text-[11.5px] text-mira-blue hover:underline"
                  >
                    Get key →
                  </a>
                )}
                {!editing && hasStored && (
                  <SavedBadge
                    tone={k.from_env ? 'blue' : 'green'}
                    icon={
                      k.from_env ? <Key className="size-3.5" /> : <Check className="size-3.5" />
                    }
                    label={k.from_env ? 'From env (yaml overrides)' : 'Saved'}
                    masked={k.masked}
                    onClick={() => beginEdit(k.name)}
                    action="Replace"
                  />
                )}
                {!editing && !hasStored && k.from_env && (
                  <SavedBadge
                    tone="blue"
                    icon={<Key className="size-3.5" />}
                    label="Using env value"
                    masked={`$${k.name}`}
                    onClick={() => beginEdit(k.name)}
                    action="Override"
                  />
                )}
                {!editing && !hasStored && !k.from_env && (
                  <Button
                    variant="outline"
                    size="sm"
                    className="h-8"
                    onClick={() => beginEdit(k.name)}
                  >
                    Add key
                  </Button>
                )}
                {editing && (
                  <>
                    <SectionInput
                      type="password"
                      value={pending ?? ''}
                      onChange={(e) => updateValue(k.name, e.target.value)}
                      placeholder="Paste key…"
                      autoComplete="off"
                      autoFocus
                      spellCheck={false}
                      className="h-8 w-44 font-mono text-[12px]"
                    />
                    <Button
                      variant="outline"
                      size="sm"
                      className="h-8 shrink-0"
                      onClick={() => cancelEdit(k.name)}
                    >
                      Cancel
                    </Button>
                  </>
                )}
              </span>
            }
          />
        );
      })}
    </TSection>
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
