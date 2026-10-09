import { IntegrationsSection } from '../../Integrations';
import { useSettingsDraft } from '../SettingsContext';

export function Component() {
  const { view, onSectionChange, githubReturn } = useSettingsDraft();
  return (
    (view && (
      <IntegrationsSection
        onOpenKeys={() => onSectionChange('search')}
        githubReturn={githubReturn ?? null}
      />
    )) ||
    null
  );
}
