import { ProviderSection } from '../ProviderSection';
import { useSettingsDraft } from '../SettingsContext';

export function Component() {
  const { view, draft, setDraft, refetch, onSectionChange } = useSettingsDraft();
  return (
    (view && (
      <>
        <div className="mb-4 rounded-lg border border-border/70 bg-muted/30 px-3 py-2.5 text-[11.5px] leading-relaxed text-muted-foreground">
          These credentials are for models <span className="text-foreground/85">Mira</span> calls
          directly. External coding agents — Claude Code, Codex, Cursor and the rest — bring their
          own login and are configured under{' '}
          <button
            type="button"
            onClick={() => onSectionChange('agents')}
            className="text-mira-blue/85 underline underline-offset-2 hover:text-mira-blue"
          >
            External agents
          </button>
          .
        </div>
        <ProviderSection view={view} draft={draft} setDraft={setDraft} refetch={refetch} />
      </>
    )) ||
    null
  );
}
