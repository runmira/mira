import type { ComponentProps } from 'react';
import { useSelector } from '@tanstack/react-store';
import { Sidebar } from '../components/Sidebar';
import { useAppRuntime } from '../state/RuntimeProvider';

type Props = Omit<ComponentProps<typeof Sidebar>, 'runningSessions' | 'completedSessions'>;

/** Activity changes wake this boundary, not the workspace command/event hooks. */
export function WorkspaceSidebar(props: Props) {
  const { sessionActivity } = useAppRuntime();
  const running = useSelector(sessionActivity, (state) => state.activity?.running ?? null);
  const completed = useSelector(sessionActivity, (state) => state.completed);
  return <Sidebar {...props} runningSessions={running} completedSessions={completed} />;
}
