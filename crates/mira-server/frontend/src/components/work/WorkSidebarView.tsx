import { useQuery } from '@tanstack/react-query';
import { useState } from 'react';
import { useLocation, useNavigate } from 'react-router';
import { prRef, workKeyFromPath, workPath } from '../../lib/workItems';
import { usePrefetchPr } from '../../state/pullRequests/queries';
import { useAppRuntime } from '../../state/RuntimeProvider';
import { useStartWork, useWorkItems, workQueries } from '../../state/work/workData';
import { useDebounced } from '../composer/IssuePalette';
import { StartWork } from './StartWork';
import { WorkSidebar } from './WorkSidebar';

/** The Work side of the sidebar, wired up: items from the work queries,
 *  the selection from the URL. */
export function WorkSidebarView({ sessionId, onOpenChat }: { sessionId: string; onOpenChat: (id: string) => void }) {
  const items = useWorkItems();
  const { pathname } = useLocation();
  const navigate = useNavigate();
  const [starting, setStarting] = useState(false);
  const prefetch = usePrefetchPr();
  return (
    <>
      <WorkSidebar
        items={items}
        activeKey={workKeyFromPath(pathname)}
        onPick={(key) => void navigate(workPath(key))}
        onIntent={(item) => {
          const ref = item.pr && prRef(item.pr);
          if (ref) prefetch(ref);
        }}
        onStartWork={() => setStarting(true)}
      />
      {starting && <StartWorkView sessionId={sessionId} onClose={() => setStarting(false)} onOpened={onOpenChat} />}
    </>
  );
}

/** Mounted only while open, so its search and draft state start fresh. */
function StartWorkView({ sessionId, onClose, onOpened }: { sessionId: string; onClose: () => void; onOpened: (id: string) => void }) {
  const runtime = useAppRuntime();
  const [query, setQuery] = useState('');
  // GitHub's search API allows 30 requests a minute: wait for typing to settle.
  const settled = useDebounced(query.trim().replace(/^#/, ''), 160);
  const search = useQuery({ ...workQueries.issues(runtime, sessionId, settled ?? ''), enabled: settled !== null });
  const start = useStartWork(sessionId, (id) => {
    onClose();
    onOpened(id);
  });
  const result = search.data;
  return (
    <StartWork
      open
      onOpenChange={(open) => !open && onClose()}
      repo={result?.owner && result.repo ? `${result.owner}/${result.repo}` : null}
      query={query}
      onQueryChange={setQuery}
      issues={result?.items ?? []}
      loading={settled === null || search.isFetching}
      error={search.error?.message ?? result?.error ?? null}
      starting={start.isPending}
      startError={start.error?.message ?? null}
      onStart={(issue) => {
        if (!result?.owner || !result.repo) return;
        start.mutate({ issue, owner: result.owner, repo: result.repo });
      }}
    />
  );
}
