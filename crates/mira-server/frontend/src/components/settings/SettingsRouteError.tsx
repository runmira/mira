import { useRevalidator, useRouteError } from 'react-router';
import { ErrorNotice } from '../ErrorNotice';

/** A failed section chunk stays inside the layout, keeping the shared draft. */
export function SettingsRouteError() {
  const error = useRouteError();
  const revalidator = useRevalidator();
  return (
    <ErrorNotice
      title="Couldn't open this settings section"
      description="Check your connection and try again. Your unsaved settings are still here."
      details={error instanceof Error ? error.message : String(error)}
      pending={revalidator.state === 'loading'}
      onRetry={() => void revalidator.revalidate()}
    />
  );
}
