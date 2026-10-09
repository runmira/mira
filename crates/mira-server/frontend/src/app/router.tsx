import { createHashRouter, Navigate, RouterProvider } from 'react-router';
import { Application } from './Application';
import { settingsRoute } from './settingsRoutes';

// Hash URLs work in the native shell and static web hosts without rewrites.
// The root stays mounted when nested settings routes change, retaining chats.
const router = createHashRouter([
  {
    path: '/',
    Component: Application,
    HydrateFallback: () => (
      <div
        role="status"
        className="grid h-screen place-items-center bg-background text-sm text-muted-foreground"
      >
        Opening Mira…
      </div>
    ),
    children: [
      { index: true, element: <Navigate to="/chat" replace /> },
      ...['chat', 'plugins', 'pull-request', 'scheduled'].map((path) => ({ path, element: null })),
      settingsRoute,
      { path: '*', element: <Navigate to="/chat" replace /> },
    ],
  },
]);

export function ApplicationRouter() {
  return <RouterProvider router={router} />;
}
