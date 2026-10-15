import { Briefcase } from 'lucide-react';
import { useLocation } from 'react-router';
import { workDetail } from '../../lib/workDetail';
import { workKeyFromPath } from '../../lib/workItems';
import { useMergeWork, useWorkDetail, useWorkItems } from '../../state/work/workData';
import { PageToolbar } from '../PageToolbar';
import { WorkItemPage } from './WorkItemPage';

/** The main pane on `/work/:key`: the item the URL names. */
export function WorkView({ activeSessionId, onOpenChat }: { activeSessionId: string; onOpenChat: (id: string) => void }) {
  const key = workKeyFromPath(useLocation().pathname);
  const items = useWorkItems();
  const item = (key && items.find((i) => i.key === key)) || null;
  const { pr, files, uncommitted, branch } = useWorkDetail(item);
  const merge = useMergeWork(item);
  if (!item) {
    return (
      <div className="flex h-full min-w-0 flex-col">
        <PageToolbar title="Work" />
        <div className="grid flex-1 place-items-center px-6 text-center">
          <div className="max-w-sm">
            <Briefcase className="mx-auto mb-3 size-6 text-muted-foreground/60" />
            <p className="text-[14px] font-medium text-foreground">{key ? 'This work isn’t here anymore' : 'Pick something to work on'}</p>
            <p className="mt-1 text-[13px] text-muted-foreground">
              {key
                ? 'Its chats were archived or deleted.'
                : 'Each item is a branch or pull request, with every chat and agent that worked on it. Start one from an issue in the sidebar.'}
            </p>
          </div>
        </div>
      </div>
    );
  }
  // Wait for the PR before drawing its page, so checks don't flash empty;
  // a failed lookup still shows what the chats know.
  const loading = !!item.pr && pr.isPending && !pr.isError;
  const detail = loading
    ? undefined
    : workDetail(item, { pr: pr.data, prFiles: files.data, uncommitted: uncommitted.data, branch: branch.data });
  const focus = item.sessions.find((s) => s.id === activeSessionId)?.id
    ?? item.sessions.find((s) => s.needs_attention)?.id
    ?? item.sessions.find((s) => s.running)?.id;
  return (
    <WorkItemPage
      item={item}
      detail={detail}
      activeSessionId={focus}
      onOpenChat={onOpenChat}
      onMerge={() => merge.mutate()}
      merging={merge.isPending}
      mergeError={merge.error?.message ?? null}
    />
  );
}
