import type { Dispatch, SetStateAction } from 'react';
import { createContext, useContext } from 'react';
import { useOutletContext } from 'react-router';
import type { SettingsView } from '../../types';
import type { Draft, SurfaceProps } from './types';

export const SettingsNavigationContext = createContext<SurfaceProps | null>(null);
export function useSettingsNavigation() {
  const context = useContext(SettingsNavigationContext);
  if (!context) throw new Error('Settings must render inside the application layout');
  return context;
}
export type SettingsDraftContext = SurfaceProps & {
  view: SettingsView | null;
  draft: Draft;
  setDraft: Dispatch<SetStateAction<Draft>>;
  refetch: () => void;
};
export function useSettingsDraft() {
  return useOutletContext<SettingsDraftContext>();
}
