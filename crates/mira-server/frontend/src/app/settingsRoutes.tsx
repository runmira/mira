import { Navigate, type RouteObject } from 'react-router';
import { SettingsRouteError } from '../components/settings/SettingsRouteError';

/** Nested lazy routes share the settings layout and its unsaved draft. */
export const settingsRoute: RouteObject = {
  path: 'settings',
  lazy: async () => {
    const { SettingsSurface } = await import('../components/Settings');
    return { Component: SettingsSurface };
  },
  children: [
    { index: true, element: <Navigate to="provider" replace /> },
    {
      path: 'provider',
      ErrorBoundary: SettingsRouteError,
      lazy: () => import('../components/settings/routes/provider'),
    },
    {
      path: 'general',
      ErrorBoundary: SettingsRouteError,
      lazy: () => import('../components/settings/routes/general'),
    },
    {
      path: 'appearance',
      ErrorBoundary: SettingsRouteError,
      lazy: () => import('../components/settings/routes/appearance'),
    },
    {
      path: 'memory',
      ErrorBoundary: SettingsRouteError,
      lazy: () => import('../components/settings/routes/memory'),
    },
    {
      path: 'skills',
      ErrorBoundary: SettingsRouteError,
      lazy: () => import('../components/settings/routes/skills'),
    },
    {
      path: 'search',
      ErrorBoundary: SettingsRouteError,
      lazy: () => import('../components/settings/routes/search'),
    },
    {
      path: 'about',
      ErrorBoundary: SettingsRouteError,
      lazy: () => import('../components/settings/routes/about'),
    },
    {
      path: 'agents',
      ErrorBoundary: SettingsRouteError,
      lazy: () => import('../components/settings/routes/agents'),
    },
    {
      path: 'subagents',
      ErrorBoundary: SettingsRouteError,
      lazy: () => import('../components/settings/routes/subagents'),
    },
    {
      path: 'devices',
      ErrorBoundary: SettingsRouteError,
      lazy: () => import('../components/settings/routes/devices'),
    },
    {
      path: 'keybindings',
      ErrorBoundary: SettingsRouteError,
      lazy: () => import('../components/settings/routes/keybindings'),
    },
    {
      path: 'hooks',
      ErrorBoundary: SettingsRouteError,
      lazy: () => import('../components/settings/routes/hooks'),
    },
    {
      path: 'usage',
      ErrorBoundary: SettingsRouteError,
      lazy: () => import('../components/settings/routes/usage'),
    },
    {
      path: 'integrations',
      ErrorBoundary: SettingsRouteError,
      lazy: () => import('../components/settings/routes/integrations'),
    },
    { path: '*', element: <Navigate to="/settings/provider" replace /> },
  ],
};
