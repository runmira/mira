import { KeysSection } from '../KeysSection';
import { useSettingsDraft } from '../SettingsContext';

export function Component() {
  const { view, draft, setDraft } = useSettingsDraft();
  return (view && <KeysSection view={view} draft={draft} setDraft={setDraft} />) || null;
}
