import { GeneralSection } from '../GeneralSection';
import { useSettingsDraft } from '../SettingsContext';

export function Component() {
  const { view, draft, setDraft } = useSettingsDraft();
  return (view && <GeneralSection draft={draft} setDraft={setDraft} />) || null;
}
