import { useSettingsDraft } from '../SettingsContext';
import { SkillsSection } from '../SkillsSection';

export function Component() {
  const { view, skillsVersion } = useSettingsDraft();
  return (view && <SkillsSection version={skillsVersion ?? 0} />) || null;
}
