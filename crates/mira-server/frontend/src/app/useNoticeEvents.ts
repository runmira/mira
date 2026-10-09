import { listCommands, listSkills } from '../api';
import type { ServerMsg } from '../types';

import { useStableCallback } from './shared';
import type { WorkspaceRuntime } from './WorkspaceRuntime';
export function useNoticeEvents(
  context: Pick<
    WorkspaceRuntime,
    | 'setEnvironment'
    | 'setEnvironments'
    | 'setEnvSwitching'
    | 'setEntries'
    | 'setCommands'
    | 'setSkills'
    | 'setExtensionsVersion'
    | 'loadEngines'
    | 'setSkillsVersion'
  >,
) {
  const {
    setEnvironment,
    setEnvironments,
    setEnvSwitching,
    setEntries,
    setCommands,
    setSkills,
    setExtensionsVersion,
    loadEngines,
    setSkillsVersion,
  } = context;
  return useStableCallback((msg: ServerMsg) => {
    switch (msg.type) {
      case 'environment_status':
        setEnvironment(msg.status);
        setEnvironments(msg.environments);
        break;

      case 'environment_progress':
        setEnvSwitching(msg.text);
        break;

      case 'environment_switched': {
        setEnvSwitching(null);
        setEnvironment(msg.status);
        const notes: string[] = msg.error
          ? [`[environment] couldn't switch to ${msg.to}`, msg.error]
          : msg.from === msg.to
            ? []
            : [`[environment] ${msg.from} → ${msg.to}`, ...msg.lines];
        if (msg.conflicts.length > 0) {
          notes.push(`Merge conflicts to resolve: ${msg.conflicts.join(', ')}`);
        }
        if (notes.length > 0) {
          setEntries((prev) => [...prev, { kind: 'warning', text: notes.join('\n') }]);
        }
        break;
      }

      case 'warning':
        setEntries((prev) => {
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
        listCommands()
          .then(setCommands)
          .catch(() => {});
        listSkills()
          .then(setSkills)
          .catch(() => {});
        setExtensionsVersion((n) => n + 1);
        // Agent install/auth state can move under us (an adapter was
        // installed, a CLI signed in) — refresh the engine list.
        loadEngines();
        break;

      case 'skills_reloaded':
        // A skill file appeared / changed / vanished. Refetch the
        // roster so the composer palette + the Settings panel pick
        // up the new state without a click.
        listSkills()
          .then(setSkills)
          .catch(() => {});
        setSkillsVersion((n) => n + 1);
        break;

      case 'error':
        setEntries((prev) => [...prev, { kind: 'error', text: msg.text }]);
        break;

      case 'memory_learned':
        setEntries((prev) => [
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
