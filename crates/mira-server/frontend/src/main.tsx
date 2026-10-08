import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { LazyMotion } from 'framer-motion';
import App from './App';
import { AuthGate } from './components/AuthGate';
import { OnboardingGate } from './components/onboarding/OnboardingGate';
import { install as installNetLog } from './lib/netLog';
import { hasHiddenTitleBar } from './lib/desktop';
import { applyTheme, watchSystemTheme } from './lib/theme';
import { applyAppearance } from './lib/appearance';
import { installInputModality } from './lib/inputModality';
import './styles.css';

// Must run before the first render so the auth and onboarding gates are
// themselves in the log — otherwise the pane opens showing an empty list
// and the first few requests are missing.
installNetLog();
installInputModality();

// Tell the stylesheet the native window is transparent and macOS is
// painting vibrancy behind it, so it can drop the `body` background that
// would otherwise cover the material. Has to happen before first paint or
// the window flashes opaque black.
if (hasHiddenTitleBar()) {
  document.documentElement.dataset.miraChrome = 'translucent';
}

// index.html already set the theme before paint; this also syncs the
// desktop window and starts following the system when that's the choice.
applyTheme();
watchSystemTheme();
applyAppearance();

// Lucide icons size via `size` prop or tailwind size-* classes (which set
// width/height in CSS and override the SVG's own width attr).
createRoot(document.getElementById('root')!).render(
  <StrictMode>
    {/* Components use the slim `m.*` elements; the animation features load
        after first paint. `strict` throws if a full `motion.*` sneaks in. */}
    <LazyMotion features={() => import('./lib/motionFeatures').then((r) => r.default)} strict>
    <AuthGate>
      <OnboardingGate>
        <App />
      </OnboardingGate>
    </AuthGate>
    </LazyMotion>
  </StrictMode>,
);
