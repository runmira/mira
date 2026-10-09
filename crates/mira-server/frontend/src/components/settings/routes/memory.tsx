import { MemorySection } from '../MemorySection';
import { useSettingsDraft } from '../SettingsContext';

export function Component() {
  const { view, draft, setDraft } = useSettingsDraft();
  return (view && <MemorySection draft={draft} setDraft={setDraft} />) || null;
}
