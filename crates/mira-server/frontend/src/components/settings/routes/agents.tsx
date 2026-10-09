import { AcpAgentsSection } from '../AcpAgentsSection';
import { ImportChatsCard } from '../ImportChatsCard';
import { useSettingsDraft } from '../SettingsContext';

export function Component() {
  const { acpAgents, acpRefreshing, onAcpRefresh, onAcpStart, acpDriver, acpError } =
    useSettingsDraft();
  return (
    <>
      <AcpAgentsSection
        agents={acpAgents ?? []}
        refreshing={acpRefreshing ?? false}
        onRefresh={onAcpRefresh ?? (() => {})}
        onStart={onAcpStart ?? (() => {})}
        activeKind={acpDriver ?? null}
        error={acpError ?? null}
      />
      <ImportChatsCard />
    </>
  );
}
