import { useAppRuntime } from '../state/RuntimeProvider';
import type { ServerMsg } from '../types';

import { useStableCallback } from './shared';
import type { WorkspaceRuntime } from './WorkspaceRuntime';
export function useNoticeEvents(
  context: Pick<
    WorkspaceRuntime,
    | 'recordEnvironment'
    | 'recordEnvironments'
    | 'recordEnvironmentTransition'
    | 'updateTranscript'
    | 'setExtensionsVersion'
    | 'loadEngines'
    | 'setSkillsVersion'
  >,
) {
  const appRuntime = useAppRuntime();
  const {
    recordEnvironment,
    recordEnvironments,
    recordEnvironmentTransition,
    updateTranscript,
    setExtensionsVersion,
    loadEngines,
    setSkillsVersion,
  } = context;
  return useStableCallback((msg: ServerMsg) => {
    switch (msg.type) {
      case 'environment_status':
        recordEnvironment(msg.status);
        recordEnvironments(msg.environments);
        break;

      case 'environment_progress':
        recordEnvironmentTransition(msg.text);
        break;

      case 'environment_switched': {
        recordEnvironmentTransition(null);
        recordEnvironment(msg.status);
        const notes: string[] = msg.error
          ? [`[environment] couldn't switch to ${msg.to}`, msg.error]
          : msg.from === msg.to
            ? []
            : [`[environment] ${msg.from} → ${msg.to}`, ...msg.lines];
        if (msg.conflicts.length > 0) {
          notes.push(`Merge conflicts to resolve: ${msg.conflicts.join(', ')}`);
        }
        if (notes.length > 0) {
          updateTranscript((prev) => [...prev, { kind: 'warning', text: notes.join('\n') }]);
        }
        break;
      }

      case 'warning':
        updateTranscript((prev) => {
          // A verify result replaces its own "running …" line.
          const last = prev[prev.length - 1];
          if (
            /^\[verify\]/.test(msg.text) &&
            last?.kind === 'warning' &&
            /^\[verify\]\s*running/i.test(last.text)
          ) {
            return [...prev.slice(0, -1), { kind: 'warning', text: msg.text }];
          }
          return [...prev, { kind: 'warning', text: msg.text }];
        });
        break;

      case 'extensions_changed':
        // An MCP server connected/dropped or a plugin changed.
        void appRuntime.resources.refresh('commands').catch(() => {});
        void appRuntime.resources.refresh('skills').catch(() => {});
        setExtensionsVersion((n) => n + 1);
        // Agent install/auth state can move under us (an adapter was
        // installed, a CLI signed in) — refresh the engine list.
        loadEngines();
        break;

      case 'skills_reloaded':
        // A skill file appeared / changed / vanished. Refetch the
        // roster so the composer palette + the Settings panel pick
        // up the new state without a click.
        void appRuntime.resources.refresh('skills').catch(() => {});
        setSkillsVersion((n) => n + 1);
        break;

      case 'error':
        updateTranscript((prev) => [...prev, { kind: 'error', text: msg.text }]);
        break;

      case 'memory_learned':
        updateTranscript((prev) => [
          ...prev,
          {
            kind: 'warning',
            text: `[memory] remembered ${msg.count} thing${msg.count === 1 ? '' : 's'}`,
          },
        ]);
        break;
    }
  });
}
