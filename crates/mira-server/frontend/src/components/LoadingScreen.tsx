import miraLogo from '../assets/mira-logo.png';

export function LoadingScreen() {
  return <div data-window-drag-region role="status" aria-label="Loading Mira" className="flex h-screen items-center justify-center">
    <img src={miraLogo} alt="" draggable={false} className="size-16 rounded-2xl object-contain motion-safe:animate-pulse" />
  </div>;
}
