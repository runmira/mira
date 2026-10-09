import { useFileIcons } from '@/lib/fileIcons';
import { cn } from '@/lib/utils';
import { CircleAlert, File as FileIcon, Info, LoaderCircle, X } from 'lucide-react';
import { useMemo, useState } from 'react';
import { groupAgentRuns } from '../lib/turnActivity';
import type { Entry } from '../transcript/entries';
import type { DiffPreview, ToolCall, ToolResult } from '../types';
import { extractPrompt, faceStateFor, stripAgentIdMarker, useSubagentIdentity } from './AgentCard';
import { DelegateCard, isDelegateTaskName } from './DelegateCard';
import { FilePanelBody, type FilePanelTab } from './FilePanel';
import { PanelLauncher } from './PanelLauncher';
import { PanelNewTabButton } from './RightPanelButton';
import { SubagentFace } from './SubagentFace';
import type { ToolStatus } from './ToolCard';
import { ToolCard } from './ToolCard';
import { ToolGroup } from './ToolGroup';
import { BrowserPane } from './panes/BrowserPane';
import { DevToolsPane } from './panes/DevToolsPane';
import { WhiteboardPane } from './panes/WhiteboardPane';
import { TOOL_PANE_DEFS, type ToolPaneTab } from './panes/toolPanes';
import { ReviewCard, SubagentResult } from './subagents/StructuredResult';
/** Right-side pane that shows one or more subagents in detail. Opens
 *  when the user clicks an AgentCard row in the transcript and adds a
 *  tab; further clicks add more tabs. Tabs are dismissible; when the
 *  last one closes the whole pane closes.
 *
 *  Visual model: reads like a mini chat, not a form. Background matches
 *  the main pane (bg-background), separated only by a subtle border to
 *  the left. The active tab is a grey rounded-full capsule; the "i"
 *  button in the header reveals the agent's prompt on demand. */
export type SubagentTab = {
  callId: string;
  call: ToolCall;
  status: ToolStatus;
  result: ToolResult | null;
  /** Live child transcript, built up from `subagent_*` WS frames. Same
   *  Entry shape as the parent transcript so the panel body can reuse
   *  parent-side rendering conventions. Empty until events start flowing. */
  streamEntries: Entry[];
  streamDone: boolean;
  /** Set when the child fired a review-required request. The panel
   *  renders an inline card with Approve / Deny; both actions route
   *  through `onReview`, which posts the `PromptResponse` and clears
   *  this field. Null when no review is pending. */
  pendingReview: null | { promptId: string; summary: string };
};

type Props = {
  tabs: SubagentTab[];
  /** File viewer tabs — live alongside agent tabs in the same strip. */
  fileTabs: FilePanelTab[];
  /** Utility panes (browser / whiteboard / devtools). Singletons by kind. */
  toolTabs: ToolPaneTab[];
  /** Whiteboard "Send" — the panel hands a PNG data URL up so the parent
   *  can turn it into a composer attachment. */
  onWhiteboardSend?: (pngDataUrl: string) => void;
  /** "+" menu actions. `onOpenPane` mirrors the toolbar's so both entry
   *  points land in the same place. */
  onOpenPane?: (kind: import('./panes/toolPanes').ToolPaneKind) => void;
  /** Opens the project file picker, for the "File…" row. */
  onBrowseFile?: () => void;
  /** Body for the panes that need the chat's state (Ask aside, Processes,
   *  Tests, Activity, Devices). Null falls through to the built-ins. */
  renderPane?: (kind: import('./panes/toolPanes').ToolPaneKind) => React.ReactNode | null;
  /** Active panel id — either a subagent callId or a file tab id (= path). */
  activeCallId: string | null;
  /** Current working directory — passed through to the file panel for
   *  breadcrumb rendering and the file-tree root. */
  cwd: string;
  onSelectTab: (id: string) => void;
  onCloseTab: (id: string) => void;
  onClose: () => void;
  /** Called on mousedown on the left resize handle. The parent (App)
   *  owns the panel width state and wires up the drag listeners. */
  onResizeStart: (e: React.MouseEvent) => void;
  /** Fired when the user answers a review-required prompt. `note` is
   *  optional and, on approval, gets prepended to the child's summary;
   *  on denial it becomes the tool-error body. */
  onReview: (parentCallId: string, promptId: string, approved: boolean, note?: string) => void;
  /** Open or refresh a file tab. Used by the file panel's explorer so
   *  clicking a file there opens a new tab rather than replacing the view. */
  onOpenFile?: (path: string, diff: DiffPreview | null) => void;
};

export function SubagentPanel({
  tabs,
  fileTabs,
  toolTabs,
  onWhiteboardSend,
  onOpenPane,
  onBrowseFile,
  renderPane,
  activeCallId,
  cwd,
  onSelectTab,
  onCloseTab,
  onClose,
  onReview,
  onResizeStart,
  onOpenFile,
}: Props) {
  if (tabs.length === 0 && fileTabs.length === 0 && toolTabs.length === 0) return null;

  // Fall back to first available tab when nothing is explicitly active.
  const effectiveActiveId =
    activeCallId ??
    (tabs.length > 0 ? tabs[0].callId : (fileTabs[0]?.id ?? toolTabs[0]?.id ?? null));
  const effectiveAgent = tabs.find((t) => t.callId === effectiveActiveId);
  const effectiveFile = fileTabs.find((t) => t.id === effectiveActiveId);
  const effectiveTool = toolTabs.find((t) => t.id === effectiveActiveId);
  const customPane = effectiveTool ? (renderPane?.(effectiveTool.kind) ?? null) : null;

  return (
    <aside className="relative flex h-full min-w-0 flex-col overflow-hidden bg-transparent">
      {/* Left-edge drag handle — 5px wide, invisible until hovered */}
      <div
        className="absolute left-0 top-0 z-20 h-full w-[5px] cursor-col-resize max-md:hidden transition-colors hover:bg-mira-blue/30 active:bg-mira-blue/50"
        onMouseDown={onResizeStart}
        title="Drag to resize panel"
      />
      {/* tab strip — h-11 matches the main pane's header so the two
          dividers line up exactly across the vertical border. */}
      <div className="right-panel-header flex h-11 shrink-0 items-center gap-1 border-b border-border/60 pl-2 pr-1.5">
        <div className="flex min-w-0 flex-1 items-center gap-1 overflow-x-auto [scrollbar-width:none] [&::-webkit-scrollbar]:hidden">
          {tabs.map((t) => (
            <TabCapsule
              key={t.callId}
              tab={t}
              active={t.callId === effectiveActiveId}
              onSelect={() => onSelectTab(t.callId)}
              onClose={() => onCloseTab(t.callId)}
            />
          ))}
          {fileTabs.map((t) => (
            <FileTabCapsule
              key={t.id}
              tab={t}
              active={t.id === effectiveActiveId}
              onSelect={() => onSelectTab(t.id)}
              onClose={() => onCloseTab(t.id)}
            />
          ))}
          {toolTabs.map((t) => (
            <ToolTabCapsule
              key={t.id}
              tab={t}
              active={t.id === effectiveActiveId}
              onSelect={() => onSelectTab(t.id)}
              onClose={() => onCloseTab(t.id)}
            />
          ))}
        </div>
        {/* Outside the scrolling strip, so it stays reachable however
            many tabs are open. */}
        {onOpenPane && (
          <PanelNewTabButton
            toolTabs={toolTabs}
            activeId={effectiveActiveId}
            onOpenPane={onOpenPane}
            onOpenFile={() => {
              onBrowseFile?.();
            }}
          />
        )}
        <button
          type="button"
          onClick={onClose}
          title="Close panel"
          aria-label="Close panel"
          className="shrink-0 rounded-md p-1.5 touch:p-2.5 text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
        >
          <X className="size-4" />
        </button>
      </div>

      {/* active tab body */}
      <div className="min-h-0 flex-1 overflow-hidden">
        {effectiveTool ? (
          customPane !== null ? (
            customPane
          ) : effectiveTool.kind === 'new' ? (
            <PanelLauncher
              onOpenPane={onOpenPane ?? (() => {})}
              onOpenFile={() => onBrowseFile?.()}
              openFiles={fileTabs.map((t) => ({
                id: t.id,
                label: t.path.split('/').pop() ?? t.path,
              }))}
              onOpenFileTab={(id) => onSelectTab(id)}
            />
          ) : effectiveTool.kind === 'browser' ? (
            <BrowserPane />
          ) : effectiveTool.kind === 'whiteboard' ? (
            <WhiteboardPane onSendToChat={(png) => onWhiteboardSend?.(png)} />
          ) : effectiveTool.kind === 'devtools' ? (
            <DevToolsPane />
          ) : null
        ) : effectiveFile ? (
          <FilePanelBody tab={effectiveFile} cwd={cwd} onOpenFile={onOpenFile} />
        ) : effectiveAgent ? (
          <div className="h-full overflow-y-auto">
            <TabBody tab={effectiveAgent} onReview={onReview} />
          </div>
        ) : null}
      </div>
    </aside>
  );
}

/** Rectangular tab. Active gets a grey `bg-secondary` fill;
 *  inactive is transparent with hover feedback. Icon + color come from
 *  the agent's identity so tabs are visually distinct at a glance. */
function TabCapsule({
  tab,
  active,
  onSelect,
  onClose,
}: {
  tab: SubagentTab;
  active: boolean;
  onSelect: () => void;
  onClose: () => void;
}) {
  const identity = useSubagentIdentity(tab.call);
  return (
    <div
      role="tab"
      aria-selected={active}
      onClick={onSelect}
      className={cn(
        'group inline-flex cursor-pointer items-center gap-1.5 rounded px-2.5 py-1 text-[12.5px] transition-colors',
        active
          ? 'right-panel-tab-active text-foreground'
          : 'text-muted-foreground hover:bg-accent/50 hover:text-foreground',
      )}
    >
      <SubagentFace
        id={identity.seed}
        face={identity.face}
        size={16}
        animate={false}
        state={faceStateFor(tab.status, tab.result?.is_error === true)}
      />
      <span className="font-medium" style={active ? { color: identity.color } : undefined}>
        {identity.name}
      </span>
      <button
        type="button"
        aria-label="Close tab"
        onClick={(e) => {
          e.stopPropagation();
          onClose();
        }}
        className={cn(
          'ml-0.5 rounded p-0.5 text-muted-foreground/60 transition-opacity hover:bg-background/60 hover:text-foreground',
          active ? 'opacity-100' : 'opacity-0 group-hover:opacity-100',
        )}
      >
        <X className="size-3" />
      </button>
    </div>
  );
}

/** Rectangular tab for a file viewer window. Shows the short filename + close
 *  button. Breadcrumb detail lives inside the `FilePanelBody` header. */
function FileTabCapsule({
  tab,
  active,
  onSelect,
  onClose,
}: {
  tab: FilePanelTab;
  active: boolean;
  onSelect: () => void;
  onClose: () => void;
}) {
  const shortName = tab.path.split('/').pop() ?? tab.path;
  return (
    <div
      role="tab"
      aria-selected={active}
      onClick={onSelect}
      className={cn(
        'group inline-flex cursor-pointer items-center gap-1.5 rounded px-2.5 py-1 text-[12.5px] transition-colors',
        active
          ? 'right-panel-tab-active text-foreground'
          : 'text-muted-foreground hover:bg-accent/50 hover:text-foreground',
      )}
    >
      {/* The tab showed one generic document glyph for every file. It now
          resolves the same themed icon the explorer uses, so a `.tsx` tab
          and a `.md` tab are distinguishable at a glance. Falls back to the
          old glyph while the icon chunk loads. */}
      <TabFileIcon name={shortName} className="size-3.5" />
      <span className={cn('font-medium font-mono', active && 'text-foreground')}>{shortName}</span>
      <button
        type="button"
        aria-label="Close tab"
        onClick={(e) => {
          e.stopPropagation();
          onClose();
        }}
        className={cn(
          'ml-0.5 rounded p-0.5 text-muted-foreground/60 transition-opacity hover:bg-background/60 hover:text-foreground',
          active ? 'opacity-100' : 'opacity-0 group-hover:opacity-100',
        )}
      >
        <X className="size-3" />
      </button>
    </div>
  );
}

/** Themed file icon for a tab, falling back to a plain glyph while the icon
 *  set is still loading. */
function TabFileIcon({ name, className }: { name: string; className?: string }) {
  const { fileIcon } = useFileIcons();
  const dataUri = fileIcon(name);
  if (dataUri) {
    return <img src={dataUri} alt="" className={cn('shrink-0', className)} draggable={false} />;
  }
  return <FileIcon className={cn('shrink-0 text-muted-foreground/70', className)} />;
}

/** Utility-pane tab. Same capsule geometry as the file tab but carries a
 *  kind icon instead of a file glyph. */
function ToolTabCapsule({
  tab,
  active,
  onSelect,
  onClose,
}: {
  tab: ToolPaneTab;
  active: boolean;
  onSelect: () => void;
  onClose: () => void;
}) {
  const def = TOOL_PANE_DEFS[tab.kind];
  return (
    <div
      role="tab"
      aria-selected={active}
      onClick={onSelect}
      title={def.blurb}
      className={cn(
        'group inline-flex shrink-0 cursor-pointer whitespace-nowrap items-center gap-1.5 rounded px-2.5 py-1 text-[12.5px] transition-colors',
        active
          ? 'right-panel-tab-active text-foreground'
          : 'text-muted-foreground hover:bg-fg/[0.05] hover:text-foreground',
      )}
    >
      <span className={cn('shrink-0', active ? 'text-foreground/90' : 'text-muted-foreground/70')}>
        {def.icon}
      </span>
      <span className="font-medium">{tab.title}</span>
      <button
        type="button"
        aria-label={`Close ${def.title}`}
        onClick={(e) => {
          e.stopPropagation();
          onClose();
        }}
        className={cn(
          'ml-0.5 rounded p-0.5 text-muted-foreground/60 transition-opacity hover:bg-background/60 hover:text-foreground',
          active ? 'opacity-100' : 'opacity-0 group-hover:opacity-100',
        )}
      >
        <X className="size-3" />
      </button>
    </div>
  );
}

/** Chat-like body: mirrors what the parent conversation looks like. No
 *  form scaffolding (no "PROMPT" / "SUMMARY" headers, no boxed pre).
 *  The prompt lives behind the "i" toggle in the header since Codex's
 *  agent windows don't show it and the user typically already knows
 *  what they asked for. */
function TabBody({
  tab,
  onReview,
}: {
  tab: SubagentTab;
  onReview: (parentCallId: string, promptId: string, approved: boolean, note?: string) => void;
}) {
  const identity = useSubagentIdentity(tab.call);
  const prompt = extractPrompt(tab.call.function.arguments);
  // Strip the `[mira-agent-id:X]\n` marker before displaying — it's an
  // internal handle for reload hydration, not user-facing text.
  const summary = tab.result?.content ? stripAgentIdMarker(tab.result.content).trim() : '';
  const isError = tab.result?.is_error === true;
  const isRunning = tab.status === 'running' || tab.status === 'pending';
  const [infoOpen, setInfoOpen] = useState(false);

  return (
    <div className="mx-auto flex max-w-2xl flex-col gap-4 px-5 pb-8 pt-5">
      <header className="flex items-center gap-2.5">
        <SubagentFace
          id={identity.seed}
          face={identity.face}
          size={40}
          state={faceStateFor(tab.status, tab.result?.is_error === true)}
        />
        <div className="flex min-w-0 flex-col leading-tight">
          <span className="text-[15px] font-semibold" style={{ color: identity.color }}>
            {identity.name}
          </span>
          <span className="text-[11px] text-muted-foreground">
            Subagent{identity.type ? ` · ${identity.type}` : ''}
          </span>
        </div>
        <button
          type="button"
          onClick={() => setInfoOpen((v) => !v)}
          title={infoOpen ? 'Hide agent info' : 'About this agent'}
          className={cn(
            'ml-1 shrink-0 rounded-full p-1 transition-colors',
            infoOpen
              ? 'bg-secondary text-foreground'
              : 'text-muted-foreground hover:bg-accent hover:text-foreground',
          )}
        >
          <Info className="size-4" fill={infoOpen ? 'currentColor' : 'none'} />
        </button>
        <div className="ml-auto">
          <StatusPill status={tab.status} isError={isError} />
        </div>
      </header>

      {infoOpen && (
        <div className="rounded-lg border border-border/60 bg-card/40 p-4">
          <div className="mb-2 flex items-center gap-2">
            <Info className="size-4" style={{ color: identity.color }} fill="currentColor" />
            <span className="text-[13px] font-medium text-foreground">About {identity.name}</span>
          </div>
          <p className="mb-3 text-[12px] leading-relaxed text-muted-foreground">
            A bounded subagent spawned by the parent — cold context, its own tools, and one summary
            back to the parent when done.
          </p>
          <div className="mb-1.5 text-[10.5px] font-semibold uppercase tracking-wider text-muted-foreground/70">
            Task
          </div>
          <div className="max-h-[40vh] overflow-y-auto whitespace-pre-wrap text-[13px] leading-relaxed text-foreground/85">
            {prompt || 'No task was provided.'}
          </div>
        </div>
      )}

      {tab.pendingReview && (
        <ReviewCard
          review={tab.pendingReview}
          identityTextClass="text-foreground"
          onApprove={(note) => onReview(tab.callId, tab.pendingReview!.promptId, true, note)}
          onDeny={(note) => onReview(tab.callId, tab.pendingReview!.promptId, false, note)}
        />
      )}

      {/* Live child transcript — renders like a mini chat as tokens
          arrive. Falls through to the final summary block only when
          there's no live stream (e.g. resumed sessions where the events
          weren't captured). Uses the same `groupAgentRuns` folding as
          the parent transcript so consecutive same-name tool calls
          collapse into ToolGroup chips here too. */}
      {tab.streamEntries.length > 0 ? (
        <StreamedEntries
          entries={tab.streamEntries}
          isRunning={isRunning}
          streamDone={tab.streamDone}
        />
      ) : (
        <>
          {isRunning && !summary && (
            <div className="flex items-center gap-2 text-[13px] text-muted-foreground">
              <LoaderCircle className="size-3.5 animate-spin text-mira-blue" />
              <span>Working…</span>
            </div>
          )}

          {summary && !isError && (
            <div className="max-w-full">
              <SubagentResult text={summary} />
            </div>
          )}

          {summary && isError && (
            <div className="max-w-full whitespace-pre-wrap text-[13px] text-destructive">
              {summary}
            </div>
          )}
        </>
      )}
    </div>
  );
}

/** Folds and renders a subagent's live transcript. Applies the same
 *  `groupAgentRuns` grouping as the main pane so consecutive reads/greps
 *  collapse into a single `ToolGroup` chip. Non-groupable entries pass
 *  through to `SubagentEntryView`. */
function StreamedEntries({
  entries,
  isRunning,
  streamDone,
}: {
  entries: Entry[];
  isRunning: boolean;
  streamDone: boolean;
}) {
  const groups = useMemo(() => groupAgentRuns(entries), [entries]);
  return (
    <div className="flex flex-col gap-1">
      {groups.map((item, i) => {
        if (item.kind === 'tool-group') {
          return (
            <ToolGroup
              key={`g-${i}`}
              entries={item.entries.map((e) => ({
                call: e.call,
                preview: e.preview,
                status: e.status,
                result: e.result,
              }))}
            />
          );
        }
        if (item.kind === 'agent-group') {
          // Nested `agent` spawns inside a subagent are rare (depth cap = 2)
          // but possible. Render each as a plain tool row rather than
          // recursing into another AgentCard/AgentGroup here — the parent
          // panel already owns the tab lifecycle, and a nested spawn is
          // usually just visual noise inside a specialist child.
          return (
            <div key={`g-${i}`} className="flex flex-col gap-0.5">
              {item.entries.map((e, j) => (
                <SubagentEntryView key={`ag-${i}-${j}`} entry={e} />
              ))}
            </div>
          );
        }
        return <SubagentEntryView key={`e-${i}`} entry={item.entry} />;
      })}
      {isRunning && !streamDone && (
        <div className="mt-1 flex items-center gap-2 text-[12px] text-muted-foreground">
          <LoaderCircle className="size-3 animate-spin text-mira-blue" />
          <span>Working…</span>
        </div>
      )}
    </div>
  );
}

/** Renders a single entry from a live subagent transcript. Assistant
 *  text goes through `AssistantContent` so it looks native; tool calls
 *  use the shared `ToolCard` compact row. Warning frames get a small
 *  amber chip so `hit max_rounds`, verify failures, etc. are visible. */
function SubagentEntryView({ entry }: { entry: Entry }) {
  switch (entry.kind) {
    case 'msg': {
      if (entry.msg.role !== 'assistant') return null;
      const text = (entry.msg.content ?? '').trim();
      if (!text) return null;
      return (
        <div className="max-w-full">
          <SubagentResult text={text} />
        </div>
      );
    }
    case 'tool':
      if (isDelegateTaskName(entry.call.function.name)) {
        return (
          <DelegateCard
            call={entry.call}
            status={entry.status}
            result={entry.result}
            startedAt={entry.startedAt}
            steps={entry.delegateSteps}
          />
        );
      }
      return (
        <ToolCard
          call={entry.call}
          preview={entry.preview}
          status={entry.status}
          result={entry.result}
          onDecide={() => {
            /* subagents auto-approve — no user gate here */
          }}
        />
      );
    case 'warning': {
      // Progress emissions from the child's `progress` tool ride the
      // warning stream with a `[progress]` prefix so we get a distinct
      // "in-flight status" chip vs. the amber warning tone. Both share
      // the same shape (subtle secondary fill, no colored stroke) — the
      // semantic hint lives in the icon + accent text.
      const progress = entry.text.match(/^\[progress\]\s*(.*)$/);
      if (progress) {
        return (
          <div className="inline-flex w-fit items-start gap-1.5 rounded-md bg-secondary/60 px-2 py-1 text-[11.5px] text-foreground/80">
            <span className="font-semibold text-mira-blue">…</span>
            <span className="break-words">{progress[1]}</span>
          </div>
        );
      }
      return (
        <div className="inline-flex w-fit items-start gap-1.5 rounded-md bg-secondary/60 px-2 py-1 text-[11.5px] text-foreground/80">
          <span className="font-semibold text-amber-400">!</span>
          <span className="break-words">{entry.text}</span>
        </div>
      );
    }
    case 'error':
      return <div className="font-mono text-[11.5px] text-destructive">error: {entry.text}</div>;
  }
}

function StatusPill({ status, isError }: { status: ToolStatus; isError: boolean }) {
  // All pills share the same shape (soft secondary fill, no border);
  // the semantic hint is a single accent-colored glyph or word so the
  // scan is still 1-2ms without leaning on colored strokes.
  const base =
    'inline-flex items-center gap-1 rounded-full bg-secondary/70 px-2 py-0.5 text-[11px]';
  if (isError) {
    return (
      <span className={cn(base, 'text-destructive')}>
        <CircleAlert className="size-3" fill="currentColor" />
        error
      </span>
    );
  }
  switch (status) {
    case 'pending':
      return <span className={cn(base, 'text-amber-300')}>awaiting approval</span>;
    case 'running':
      return (
        <span className={cn(base, 'text-mira-blue')}>
          <LoaderCircle className="size-3 animate-spin" />
          working
        </span>
      );
    case 'denied':
      return <span className={cn(base, 'text-muted-foreground')}>denied</span>;
    case 'complete':
      return <span className={cn(base, 'text-emerald-400')}>done</span>;
  }
}

export type { FilePanelTab };
