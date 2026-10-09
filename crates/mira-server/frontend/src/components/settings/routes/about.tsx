import { AboutSection } from '../AboutSection';
import { useSettingsDraft } from '../SettingsContext';

export function Component() {
  const { view } = useSettingsDraft();
  return (view && <AboutSection view={view} />) || null;
}
