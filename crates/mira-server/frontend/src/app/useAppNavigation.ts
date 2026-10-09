import type { Dispatch, SetStateAction } from 'react';
import { useCallback, useRef } from 'react';
import { useLocation, useNavigate } from 'react-router';
import type { MainView } from '../components/Sidebar';
import { SETTINGS_SECTIONS, type SettingsSectionId } from '../components/settings/sections';

/** URL owns navigation; session state stays mounted in the root route. */
export function useAppNavigation() {
  const { pathname } = useLocation();
  const navigate = useNavigate();
  const segment = pathname.split('/')[1];
  const mainView: MainView =
    segment === 'settings' ||
    segment === 'plugins' ||
    segment === 'pull-request' ||
    segment === 'scheduled'
      ? segment
      : 'chat';
  const section = pathname.split('/')[2];
  const settingsSection = SETTINGS_SECTIONS.find((item) => item.id === section)?.id ?? 'provider';
  const returnTo = useRef<MainView>('chat');
  const current = useRef({ mainView, settingsSection });
  current.current = { mainView, settingsSection };
  const setMainView: Dispatch<SetStateAction<MainView>> = useCallback(
    (value) => {
      const next = typeof value === 'function' ? value(current.current.mainView) : value;
      if (next === 'settings' && current.current.mainView !== 'settings')
        returnTo.current = current.current.mainView;
      // Update immediately so two navigation commands in the same event compose.
      current.current.mainView = next;
      void navigate(
        next === 'settings' ? `/settings/${current.current.settingsSection}` : `/${next}`,
      );
    },
    [navigate],
  );
  const setSettingsSection = useCallback(
    (next: SettingsSectionId) => {
      current.current.settingsSection = next;
      if (current.current.mainView === 'settings') void navigate(`/settings/${next}`);
    },
    [navigate],
  );
  const openSettings = useCallback(() => setMainView('settings'), [setMainView]);
  const exitSettings = useCallback(
    () => setMainView(returnTo.current === 'settings' ? 'chat' : returnTo.current),
    [setMainView],
  );
  return { mainView, setMainView, settingsSection, setSettingsSection, openSettings, exitSettings };
}
