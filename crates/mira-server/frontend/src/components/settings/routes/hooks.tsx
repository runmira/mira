import { HooksSection } from '../../Hooks';
import { useSettingsDraft } from '../SettingsContext';

export function Component() {
  const { view } = useSettingsDraft();
  return (view && <HooksSection />) || null;
}
