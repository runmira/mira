import { Button } from '@/components/ui/button';
import { Dialog, DialogContent } from '@/components/ui/dialog';
import { TRAFFIC_LIGHT_INSET } from '@/lib/desktop';
import { cn } from '@/lib/utils';
import {
  Archive,
  ArrowLeft,
  CheckCheck,
  ChevronDown,
  ChevronRight,
  CircleAlert,
  Folder,
  GitBranch,
  History,
  PenLine,
  Puzzle,
  Search,
  Timer,
  Trash2,
  X,
} from 'lucide-react';
import { PluginsPanel, PullRequestPanel, SessionPeek } from '../lazyViews';
import { isSettled } from '../lib/settled';
import { settledPeriod, type SidebarFilter } from '../lib/sidebar';
import { BranchElbow } from './BranchElbow';
import { ErrorNotice } from './ErrorNotice';
import { LazyBoundary } from './LazyBoundary';
import { SETTINGS_SECTIONS } from './settings/sections';
import { dateSection } from './sidebar/groups';
import { Empty, NavItem, ShelfHeader, SidebarFolderIcon } from './sidebar/navigation';
import {
  NEEDS_MORE_KEY,
  PER_GROUP_LIMIT,
  RECENTS_LIMIT,
  RECENTS_MORE_KEY,
  RECENTS_OPEN_KEY,
  saveFlag,
  SCROLL_KEY,
  SETTLED_OPEN_KEY,
  SETTLED_PAGE,
} from './sidebar/preferences';
import { RenameDialog } from './sidebar/RenameDialog';
import { RowMenu } from './sidebar/RowMenu';
import { ArchivedRow, forkForest, SessionRow } from './sidebar/sessionRows';
import { ForkNode, Props } from './sidebar/types';
import { useSidebarController } from './sidebar/useSidebarController';
import { Collapse } from './ui/Collapse';
import { UserCard } from './UserCard';
export function Sidebar(props: Props) {
  const {
    status,
    activeSessionId,
    activeBusy,
    activeView,
    onNavigate,
    onNewChat,
    onOpenSettings,
    onSetBackgroundMode,
    activePr,
    runningSessions,
    settingsSection = 'provider',
    onSettingsSectionChange,
    onExitSettings,
  } = props;
  const {
    hiddenTitleBar,
    hasScrolled,
    projectsPinned,
    scrollRef,
    setPeek,
    setHasScrolled,
    setProjectsPinned,
    projectsToolbarRef,
    scrollRestored,
    searching,
    savedScroll,
    needsYou,
    needsMore,
    flatRow,
    setNeedsMore,
    recents,
    recentsOpen,
    toggleShelf,
    setRecentsOpen,
    live,
    recentsMore,
    setRecentsMore,
    selecting,
    groups,
    toggleSelectMode,
    sessions,
    searchRef,
    query,
    setQuery,
    filter,
    setFilter,
    selected,
    bulkArchive,
    bulkBusy,
    setConfirmBulkDelete,
    listError,
    listLoading,
    refresh,
    error,
    setError,
    collapsed,
    showMore,
    toggleCollapsed,
    removeAllInProject,
    forksHidden,
    currentSession,
    toggleForks,
    unread,
    toggleSelected,
    pickSession,
    setRenaming,
    removeSession,
    flagSession,
    changeBackgroundMode,
    toggleShowMore,
    settled,
    settledOpen,
    setSettledOpen,
    settledShown,
    setSettledShown,
    archived,
    showArchived,
    setShowArchived,
    archivedError,
    archivedLoading,
    peekMounted,
    livePeek,
    peek,
    renaming,
    applyManualRename,
    applyAiRename,
    confirmBulkDelete,
    bulkDelete,
  } = useSidebarController(props);
  return (
    <>
      <aside
        className={cn(
          'flex h-full min-w-0 flex-col',
          // On the desktop build the window is transparent and macOS paints
          // vibrancy behind it, so the sidebar contributes *no* fill of its
          // own — the OS material is the sidebar background. An earlier pass
          // laid a dark tint over the vibrancy to keep text readable, which
          // defeated the point: the material got multiplied down to near-black
          // and read as another flat dark panel. Contrast comes from macOS
          // darkening the material in dark appearance, not from us.
          hiddenTitleBar ? 'bg-transparent' : 'bg-panel',
        )}
      >
        {/* When the native title bar is hidden, this row *is* the title bar:
          it sits in the strip beside the traffic lights, carries the
          wordmark, and is the window's drag region. Otherwise it's a normal
          padded header. */}
        <div
          data-tauri-drag-region
          className={cn(
            'flex items-center justify-between px-3',
            hiddenTitleBar ? 'shrink-0' : 'pb-2 pt-3.5',
          )}
          style={
            hiddenTitleBar
              ? {
                  // Beside the traffic lights, not below them.
                  height: TRAFFIC_LIGHT_INSET.top + 8,
                  paddingLeft: TRAFFIC_LIGHT_INSET.left,
                }
              : undefined
          }
        >
          {/* No wordmark: the sidebar's own nav labels identify the app, and
            a title here just crowds the traffic lights. The row stays as
            the drag region and as the inset that keeps the lights clear. */}
        </div>

        {activeView !== 'settings' && (
          <div className="px-3 pb-2">
            {/* A raised control sitting *on* the vibrancy, so it needs enough
              body to read as a surface: a translucent fill, a backdrop blur
              to keep the wallpaper from muddying the label, and a hairline.
              The selected half is near-opaque so the active tab is
              unambiguous without relying on colour alone. */}
            <div className="grid grid-cols-2 gap-0.5 rounded-full border border-border/80 bg-shade/25 p-0.5 backdrop-blur-md">
              <span className="rounded-full bg-foreground/90 px-3.5 py-1 text-center text-[12.5px] font-medium text-background">
                Chat
              </span>
              <span
                title="Not implemented yet"
                className="cursor-not-allowed rounded-full px-3.5 py-1 text-center text-[12.5px] text-foreground/45"
              >
                Work
              </span>
            </div>
          </div>
        )}

        {activeView === 'settings' ? (
          // Settings mode — the sidebar becomes the section picker. The
          // "Back to app" pill up top pops the user back to whatever view
          // they were on before entering settings; the rest of the app
          // (projects, folder chip, status footer) is hidden to keep the
          // context single-purpose while they're configuring things.
          <div className="min-h-0 flex-1">
            <div className="h-full overflow-y-auto px-3 pb-2 pt-1">
              <button
                type="button"
                onClick={() => onExitSettings?.()}
                className="mb-3 inline-flex items-center gap-1.5 rounded-full border border-border bg-secondary/60 px-3 py-1.5 text-[12.5px] text-foreground/90 transition-colors hover:bg-secondary hover:text-foreground"
              >
                <ArrowLeft className="size-3.5" />
                Back to app
              </button>
              <div className="mb-1.5 px-1 text-[11.5px] font-semibold uppercase tracking-wider text-muted-foreground/80">
                Settings
              </div>
              <nav className="flex flex-col gap-0.5">
                {SETTINGS_SECTIONS.map((s) => {
                  const active = settingsSection === s.id;
                  const Icon = s.icon;
                  return (
                    <button
                      key={s.id}
                      type="button"
                      onClick={() => onSettingsSectionChange?.(s.id)}
                      className={cn(
                        'flex items-center gap-2.5 rounded-lg px-2.5 py-2 text-left text-[13.5px] transition-colors',
                        active
                          ? 'sidebar-active bg-fg/[0.1] text-foreground'
                          : 'text-foreground/80 hover:bg-fg/[0.05] hover:text-foreground',
                      )}
                    >
                      <Icon
                        className={cn(
                          'size-4 shrink-0',
                          active ? 'text-mira-blue' : 'text-muted-foreground',
                        )}
                      />
                      <span>{s.label}</span>
                    </button>
                  );
                })}
              </nav>
            </div>
          </div>
        ) : (
          <div className="flex min-h-0 flex-1 flex-col">
            <div className="relative z-30 shrink-0 bg-panel px-2 pb-1">
              <NavItem icon={<PenLine className="size-3.5" />} onClick={onNewChat}>
                New thread
              </NavItem>
              {hasScrolled && !projectsPinned && (
                <span aria-hidden className="sidebar-scroll-fade" />
              )}
            </div>
            <div
              ref={scrollRef}
              onScroll={(e) => {
                const node = e.currentTarget;
                setPeek(null);
                setHasScrolled(node.scrollTop > 0);
                setProjectsPinned(
                  !!projectsToolbarRef.current &&
                    projectsToolbarRef.current.getBoundingClientRect().top <=
                      node.getBoundingClientRect().top + 1,
                );
                if (!scrollRestored.current || searching) return;
                savedScroll.current = node.scrollTop;
                try {
                  localStorage.setItem(SCROLL_KEY, String(node.scrollTop));
                } catch {
                  /* optional storage */
                }
              }}
              className="min-h-0 flex-1 overflow-y-auto px-1.5 pb-2 [overflow-anchor:none]"
            >
              <nav className="flex flex-col gap-0.5 px-0.5">
                <NavItem
                  icon={<GitBranch className="size-3.5" />}
                  active={activeView === 'pull-request'}
                  onClick={() => onNavigate('pull-request')}
                  onIntent={PullRequestPanel.preload}
                >
                  Pull request
                </NavItem>
                <NavItem
                  icon={<Puzzle className="size-3.5" />}
                  active={activeView === 'plugins'}
                  onClick={() => onNavigate('plugins')}
                  onIntent={PluginsPanel.preload}
                >
                  Plugins
                </NavItem>
                <NavItem
                  icon={<Timer className="size-3.5" />}
                  disabled
                  active={activeView === 'scheduled'}
                >
                  Scheduled
                </NavItem>
              </nav>

              {!searching && needsYou.length > 0 && (
                <div className="mt-4 flex flex-col gap-0.5 px-0.5">
                  <div className="flex items-center gap-1.5 px-2 py-1 text-[11.5px] font-semibold text-amber-700 dark:text-amber-400">
                    <CircleAlert className="size-3" /> Needs you
                    <span className="ml-auto text-[10.5px] tabular-nums opacity-70">
                      {needsYou.length}
                    </span>
                  </div>
                  <div className="flex flex-col gap-0.5 pl-1">
                    {(needsMore ? needsYou : needsYou.slice(0, 2)).map((s) =>
                      flatRow(s, 'attention'),
                    )}
                    {needsYou.length > 2 && (
                      <button
                        type="button"
                        onClick={() => {
                          saveFlag(NEEDS_MORE_KEY, !needsMore);
                          setNeedsMore(!needsMore);
                        }}
                        className="px-2 py-1 text-left text-[11px] text-muted-foreground hover:text-foreground"
                      >
                        {needsMore ? 'Show less' : `View ${needsYou.length - 2} more chats`}
                      </button>
                    )}
                  </div>
                </div>
              )}

              {!searching && recents.length > 0 && (
                <div className="mt-4 flex flex-col gap-0.5 px-0.5">
                  <ShelfHeader
                    icon={<History className="size-3 shrink-0" />}
                    label="Recents"
                    open={recentsOpen}
                    onToggle={() => toggleShelf(RECENTS_OPEN_KEY, setRecentsOpen)}
                  />
                  <Collapse open={recentsOpen}>
                    <div className="flex flex-col gap-0.5 pl-1">
                      {recents.map((s) => flatRow(s, 'recent'))}
                      {live.filter((s) => !s.needs_attention && !s.failure_reason).length >
                        RECENTS_LIMIT && (
                        <button
                          type="button"
                          onClick={() => {
                            saveFlag(RECENTS_MORE_KEY, !recentsMore);
                            setRecentsMore(!recentsMore);
                          }}
                          className="px-2 py-1 text-left text-[11px] text-muted-foreground hover:text-foreground"
                        >
                          {recentsMore ? 'Show less' : 'More recent chats'}
                        </button>
                      )}
                    </div>
                  </Collapse>
                </div>
              )}

              <div className="mt-4 flex flex-col gap-1 px-0.5">
                <div
                  ref={projectsToolbarRef}
                  className="sticky top-0 z-20 -mx-0.5 bg-panel px-0.5 pb-1"
                >
                  <div className="mb-1 flex items-center justify-between px-2 py-1.5">
                    <div className="flex items-center gap-2 text-[14px] font-semibold tracking-tight text-foreground">
                      <Folder className="size-3.5 text-muted-foreground" aria-hidden />
                      Projects
                    </div>
                    {!selecting && groups.length > 0 && (
                      <button
                        type="button"
                        onClick={toggleSelectMode}
                        className="sidebar-select rounded-full border border-border/60 px-2.5 py-1 text-[11px] font-medium text-foreground/75 transition-colors hover:bg-fg/[0.05] hover:text-foreground"
                      >
                        Select
                      </button>
                    )}
                  </div>

                  {!selecting && (sessions.length > 0 || searching) && (
                    <label className="sidebar-search mx-1 mb-1 flex items-center gap-1.5 rounded-xl border border-border/60 px-1.5 py-1.5 text-[12px] transition-shadow focus-within:ring-2 focus-within:ring-mira-blue/15">
                      <button
                        type="button"
                        aria-label="Focus chat search"
                        onClick={() => searchRef.current?.focus()}
                        className="flex size-6 shrink-0 items-center justify-center rounded-lg text-muted-foreground transition-colors hover:bg-fg/[0.05] hover:text-foreground"
                      >
                        <Search className="size-3.5" aria-hidden />
                      </button>
                      {/* Plain text, not type=search: that adds the browser's own
                  clear button beside ours. */}
                      <input
                        ref={searchRef}
                        type="text"
                        enterKeyHint="search"
                        value={query}
                        onChange={(e) => setQuery(e.target.value)}
                        onKeyDown={(e) => {
                          if (e.key === 'Escape') setQuery('');
                        }}
                        placeholder="Search chats, branches, PRs"
                        aria-label="Search chats"
                        className="min-w-0 flex-1 bg-transparent text-foreground outline-none placeholder:text-muted-foreground/60"
                      />
                      {query && (
                        <button
                          type="button"
                          onClick={() => setQuery('')}
                          aria-label="Clear search"
                          className="flex size-6 shrink-0 items-center justify-center rounded-full text-muted-foreground/70 transition-colors hover:bg-fg/[0.06] hover:text-foreground"
                        >
                          <X className="size-3.5" />
                        </button>
                      )}
                    </label>
                  )}

                  {!selecting && sessions.length > 0 && (
                    <div className="mx-1 mb-2 flex gap-1" aria-label="Filter chats">
                      {(['all', 'running', 'waiting', 'settled'] as SidebarFilter[]).map(
                        (value) => (
                          <button
                            type="button"
                            key={value}
                            aria-pressed={filter === value}
                            onClick={() => setFilter(value)}
                            className={cn(
                              'rounded-full px-2 py-1 text-[10.5px] transition-colors',
                              filter === value
                                ? 'sidebar-active bg-fg/[0.08] font-medium text-foreground'
                                : 'text-muted-foreground hover:bg-fg/[0.04] hover:text-foreground',
                            )}
                          >
                            {
                              {
                                all: 'All',
                                running: 'Working',
                                waiting: 'Needs you',
                                settled: 'Settled',
                              }[value]
                            }
                          </button>
                        ),
                      )}
                    </div>
                  )}

                  {selecting && (
                    <div className="flex items-center gap-1 rounded-lg border border-border/70 bg-panel px-2.5 py-1.5 shadow-sm">
                      <span className="text-[12px] font-medium text-foreground">
                        {selected.size} selected
                      </span>
                      <div className="ml-auto flex items-center gap-0.5">
                        <button
                          type="button"
                          onClick={() => void bulkArchive()}
                          disabled={bulkBusy || selected.size === 0}
                          className="inline-flex items-center gap-1 rounded-md px-2 py-1 text-[12px] text-foreground/85 transition-colors hover:bg-fg/[0.05] hover:text-foreground disabled:opacity-40"
                        >
                          <Archive className="size-3.5" /> Archive
                        </button>
                        <button
                          type="button"
                          onClick={() => setConfirmBulkDelete(true)}
                          disabled={bulkBusy || selected.size === 0}
                          className="inline-flex items-center gap-1 rounded-md px-2 py-1 text-[12px] text-red-400/90 transition-colors hover:bg-red-500/10 hover:text-red-300 disabled:opacity-40"
                        >
                          <Trash2 className="size-3.5" /> Delete
                        </button>
                        <button
                          type="button"
                          onClick={toggleSelectMode}
                          disabled={bulkBusy}
                          title="Cancel selection"
                          className="inline-flex items-center rounded-md p-1 text-muted-foreground transition-colors hover:bg-fg/[0.05] hover:text-foreground disabled:opacity-40"
                        >
                          <X className="size-3.5" />
                        </button>
                      </div>
                    </div>
                  )}
                  {projectsPinned && <span aria-hidden className="sidebar-scroll-fade" />}
                </div>

                {listError && (
                  <ErrorNotice
                    title="Couldn't load chats"
                    description="Check that Mira is running, then try again."
                    details={listError}
                    pending={listLoading}
                    onRetry={refresh}
                  />
                )}
                {error && (
                  <ErrorNotice
                    title="Couldn't complete this action"
                    description="Refresh the list to check its current state before trying the action again."
                    details={error}
                    retryLabel="Refresh chats"
                    onRetry={() => {
                      setError(null);
                      refresh();
                    }}
                  />
                )}
                {!error && !listError && !listLoading && groups.length === 0 && (
                  <Empty>{searching ? 'No chats match' : 'No saved chats yet'}</Empty>
                )}

                {groups.map((g) => {
                  const isCollapsed = !searching && collapsed.has(g.cwd);
                  const expandedAll = searching || showMore.has(g.cwd);
                  const roots = forkForest(g.sessions);
                  const visible = expandedAll ? roots : roots.slice(0, PER_GROUP_LIMIT);
                  const waiting = g.sessions.filter(
                    (x) => x.needs_attention && x.id !== activeSessionId,
                  ).length;
                  // Date sections only when they say something: rows spanning
                  // more than one period.
                  const sections = new Set(visible.map((r) => dateSection(r.session))).size > 1;
                  const overflow = roots.length - visible.length;

                  return (
                    <div key={g.cwd} className="flex flex-col">
                      <div
                        className="group flex items-center gap-1.5 rounded-md px-2 py-1 text-[14.5px] font-semibold text-foreground/90 transition-colors hover:bg-fg/[0.05] hover:text-foreground"
                        title={g.cwd}
                      >
                        <button
                          type="button"
                          onClick={() => toggleCollapsed(g.cwd)}
                          aria-expanded={!isCollapsed}
                          className="flex min-w-0 flex-1 items-center gap-1.5 text-left"
                        >
                          {isCollapsed ? (
                            <ChevronRight className="size-3 shrink-0 text-muted-foreground/60" />
                          ) : (
                            <ChevronDown className="size-3 shrink-0 text-muted-foreground/60" />
                          )}
                          <SidebarFolderIcon
                            name={g.label}
                            open={!isCollapsed}
                            className="size-3.5"
                            current={g.isCurrent}
                          />
                          <span className={cn('truncate', g.isCurrent && 'text-foreground')}>
                            {g.label}
                          </span>
                          {isCollapsed && waiting > 0 && (
                            <span
                              className="ml-0.5 inline-flex items-center gap-0.5 rounded-full bg-amber-500/15 px-1.5 text-[10.5px] font-medium tabular-nums text-amber-600 dark:text-amber-400"
                              title={`${waiting} chat${waiting === 1 ? '' : 's'} waiting on you`}
                            >
                              <CircleAlert className="size-2.5" />
                              {waiting}
                            </span>
                          )}
                        </button>
                        <RowMenu
                          items={[
                            {
                              label: 'Delete all sessions',
                              danger: true,
                              confirm: `Delete all ${g.sessions.length} session${g.sessions.length === 1 ? '' : 's'} in ${g.label}?`,
                              icon: <Trash2 className="size-3.5" />,
                              onSelect: () => removeAllInProject(g.cwd),
                            },
                          ]}
                        />
                      </div>

                      <Collapse open={!isCollapsed}>
                        <div className="flex flex-col gap-0.5 pl-1">
                          {visible.map((root, rootIndex) => {
                            const section = dateSection(root.session);
                            const header =
                              sections &&
                              (rootIndex === 0 ||
                                dateSection(visible[rootIndex - 1].session) !== section) ? (
                                <div
                                  key={`h-${section}`}
                                  className="px-2 pb-0.5 pt-2 text-[10.5px] font-medium uppercase tracking-wider text-muted-foreground/55 first:pt-0.5"
                                >
                                  {section}
                                </div>
                              ) : null;
                            const renderNode = (node: ForkNode, depth: number): React.ReactNode => {
                              const s = node.session;
                              const open = !forksHidden.has(s.id);
                              return (
                                <div key={s.id} className="flex flex-col gap-0.5">
                                  <SessionRow
                                    session={currentSession(s)}
                                    depth={depth}
                                    forkCount={node.forks.length}
                                    forksOpen={!forksHidden.has(s.id)}
                                    onToggleForks={() => toggleForks(s.id)}
                                    // One selection model across the sidebar: the
                                    // session highlight only shows while the chat
                                    // view is actually open — picking Plugins /
                                    // PR / Scheduled leaves the row highlighted
                                    // otherwise (and New thread swaps in a fresh
                                    // id server-side, which drops it naturally).
                                    active={s.id === activeSessionId && activeView === 'chat'}
                                    activeBusy={runningSessions == null && activeBusy}
                                    unread={unread.has(s.id)}
                                    launched={depth > 0 && !s.forked_from && !!s.launched_by}
                                    selecting={selecting}
                                    checked={selected.has(s.id)}
                                    onToggleSelect={() => toggleSelected(s.id)}
                                    onPeek={(rect) => setPeek(rect ? { session: s, rect } : null)}
                                    onPick={() => pickSession(s.id)}
                                    onRename={() => setRenaming(s)}
                                    onDelete={() => removeSession(s.id)}
                                    onPin={(p) => void flagSession(s.id, { pinned: p })}
                                    onArchive={() => void flagSession(s.id, { archived: true })}
                                    settled={isSettled(currentSession(s))}
                                    onSettle={(v) => void flagSession(s.id, { settled: v })}
                                    onSetBackgroundMode={
                                      onSetBackgroundMode
                                        ? (mode) => void changeBackgroundMode(s.id, mode)
                                        : undefined
                                    }
                                  />
                                  <Collapse open={node.forks.length > 0 && open}>
                                    {/* Branch lines: a rail down from the chat's icon,
                                  with a rounded elbow into each fork. */}
                                    <div className="ml-[15px] flex flex-col gap-0.5 pl-3">
                                      {node.forks.map((f, i) => (
                                        <div key={f.session.id} className="relative">
                                          {i < node.forks.length - 1 && (
                                            <span
                                              aria-hidden
                                              className="pointer-events-none absolute -bottom-0.5 -left-3 top-0 border-l border-fg/15"
                                            />
                                          )}
                                          <BranchElbow className="-left-3 top-0" />
                                          {renderNode(f, depth + 1)}
                                        </div>
                                      ))}
                                    </div>
                                  </Collapse>
                                </div>
                              );
                            };
                            return (
                              <div key={root.session.id} className="flex flex-col gap-0.5">
                                {header}
                                {renderNode(root, 0)}
                              </div>
                            );
                          })}
                          {overflow > 0 && (
                            <button
                              onClick={() => toggleShowMore(g.cwd)}
                              className="px-2 py-1 text-left text-[12.5px] text-muted-foreground/80 transition-colors hover:text-foreground"
                            >
                              Show {overflow} more
                            </button>
                          )}
                          {expandedAll && g.sessions.length > PER_GROUP_LIMIT && (
                            <button
                              onClick={() => toggleShowMore(g.cwd)}
                              className="px-2 py-1 text-left text-[12.5px] text-muted-foreground/80 transition-colors hover:text-foreground"
                            >
                              Show less
                            </button>
                          )}
                        </div>
                      </Collapse>
                    </div>
                  );
                })}

                {/* Settled: chats whose work is done — their PR merged or closed,
              or the user put them away. Out of the folders, still a click
              away; writing to one brings it back. */}
                {!searching && settled.length > 0 && (
                  <div className="mt-4 flex flex-col gap-0.5 px-0.5">
                    <ShelfHeader
                      icon={<CheckCheck className="size-3 shrink-0" />}
                      label="Settled"
                      count={settled.length}
                      open={settledOpen}
                      onToggle={() => toggleShelf(SETTLED_OPEN_KEY, setSettledOpen)}
                    />
                    <Collapse open={settledOpen}>
                      <div className="flex flex-col gap-0.5 pl-1">
                        {settled.slice(0, settledShown).map((s, i, rows) => (
                          <div key={s.id}>
                            {(i === 0 || settledPeriod(s) !== settledPeriod(rows[i - 1])) && (
                              <div className="px-2 pb-1 pt-2 text-[10.5px] font-medium text-muted-foreground/60">
                                {settledPeriod(s)}
                              </div>
                            )}
                            {flatRow(s, 'settled')}
                          </div>
                        ))}
                        {settled.length > settledShown && (
                          <button
                            onClick={() => setSettledShown((n) => n + SETTLED_PAGE * 2)}
                            className="px-2 py-1 text-left text-[12.5px] text-muted-foreground/80 transition-colors hover:text-foreground"
                          >
                            Show {Math.min(settled.length - settledShown, SETTLED_PAGE * 2)} more
                          </button>
                        )}
                      </div>
                    </Collapse>
                  </div>
                )}

                {/* Archived view (issue #58) — sessions hidden from the main
              list live here. Clicking a row opens it; Restore puts it back
              in its folder; Delete is permanent. */}
                {!selecting && !searching && (
                  <div className="mt-4 flex flex-col gap-0.5 px-0.5">
                    <ShelfHeader
                      icon={<Archive className="size-3 shrink-0" />}
                      label="Archived"
                      count={archived?.length || undefined}
                      open={showArchived}
                      onToggle={() => setShowArchived((v) => !v)}
                    />
                    {showArchived && archivedError && (
                      <ErrorNotice
                        title="Couldn't load archived chats"
                        description="Try again when Mira is connected."
                        details={archivedError}
                        pending={archivedLoading}
                        onRetry={refresh}
                      />
                    )}
                    {showArchived && (
                      <div className="flex flex-col gap-0.5 pl-1">
                        {archived === null && !archivedError && <Empty>Loading…</Empty>}
                        {archived !== null &&
                          archived.length === 0 &&
                          !archivedError &&
                          !archivedLoading && <Empty>Nothing archived</Empty>}
                        {archived?.map((s) => (
                          <ArchivedRow
                            key={s.id}
                            session={s}
                            active={s.id === activeSessionId && activeView === 'chat'}
                            onOpen={() => void pickSession(s.id)}
                            onRestore={() => void flagSession(s.id, { archived: false })}
                            onDelete={() => removeSession(s.id)}
                          />
                        ))}
                      </div>
                    )}
                  </div>
                )}
              </div>
            </div>
          </div>
        )}

        <UserCard status={status} onOpenSettings={onOpenSettings} />
        {/* One hover card for every row in the list. */}
        {peekMounted && (
          <LazyBoundary>
            <SessionPeek
              target={livePeek}
              pr={
                peek
                  ? (peek.session.pr ??
                    (peek.session.id === activeSessionId ? (activePr ?? null) : null))
                  : null
              }
              onDismiss={() => setPeek(null)}
            />
          </LazyBoundary>
        )}
      </aside>
      <RenameDialog
        session={renaming}
        onClose={() => setRenaming(null)}
        onManual={applyManualRename}
        onAi={applyAiRename}
      />
      {/* Bulk delete confirm (issue #58 acceptance: bulk actions confirm
        before deleting). Archive is reversible, so it fires directly. */}
      <Dialog open={confirmBulkDelete} onOpenChange={(o) => !o && setConfirmBulkDelete(false)}>
        <DialogContent className="max-w-sm p-0 gap-0">
          <div className="flex flex-col gap-4 p-5">
            <div>
              <div className="text-[15px] font-semibold text-foreground">
                Delete {selected.size} session{selected.size === 1 ? '' : 's'}?
              </div>
              <div className="mt-1 text-[12.5px] text-muted-foreground">
                This permanently removes the transcripts. This cannot be undone.
              </div>
            </div>
            <div className="flex items-center justify-end gap-2">
              <Button
                variant="outline"
                onClick={() => setConfirmBulkDelete(false)}
                disabled={bulkBusy}
              >
                Cancel
              </Button>
              <Button variant="destructive" onClick={() => void bulkDelete()} disabled={bulkBusy}>
                {bulkBusy ? 'Deleting…' : 'Delete'}
              </Button>
            </div>
          </div>
        </DialogContent>
      </Dialog>
    </>
  );
}

export { type MainView } from './sidebar/types';
