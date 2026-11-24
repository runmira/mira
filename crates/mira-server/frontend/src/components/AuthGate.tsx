import { RuntimeProvider } from '../state/RuntimeProvider';
export function AuthGate({ children }: { children: React.ReactNode }) {
  return <RuntimeProvider scope={{ backend: window.location.origin, account: 'local' }}>{children}</RuntimeProvider>;
}
