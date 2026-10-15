import type { UiStorage, RuntimeScope } from './createAppRuntime';
import type { createRuntimeStore } from './runtimeStore';
import { selectActiveSession } from './runtimeStore';
import type { createWorkspaceUiStore } from './workspaceUiStore';
import { toolPaneKindOf, type ToolPaneTab } from '../components/panes/toolPanes';
import { reviewTurnSources } from '../lib/reviewSources';

type Layout = { width: number; files: string[]; tools: ToolPaneTab[]; active: string | null; sources: Record<string,string> };
const empty = (): Layout => ({width:390,files:[],tools:[],active:null,sources:{}});
/** Store only navigation metadata. File diffs, transcript text and comments stay in runtime memory. */
export function createPanelLayoutService(storage: UiStorage | undefined, scope: RuntimeScope,
  sessions: ReturnType<typeof createRuntimeStore>['source'], ui: ReturnType<typeof createWorkspaceUiStore>) {
  let workspace = '';
  let owner: string | null = null;
  let restoring = false;
  let layout = empty();
  let pendingSource: string | null = null;
  let timer: ReturnType<typeof setTimeout> | undefined;
  const writes = new Map<string,string>();
  const records = new Map<string,string>();
  function flush() {
    clearTimeout(timer); timer = undefined;
    for (const [id,value] of writes) { try { storage?.setItem(id,value); } catch { /* unavailable storage */ } }
    writes.clear();
  }
  const key = (cwd: string) => `mira.panel-layout.v1:${JSON.stringify([scope.backend,scope.account,cwd])}`;
  function read(cwd: string): Layout {
    try {
      const value = JSON.parse(records.get(key(cwd)) ?? writes.get(key(cwd)) ?? storage?.getItem(key(cwd)) ?? 'null');
      if (value?.version !== 1) return empty();
      const files = Array.isArray(value.files) ? [...new Set(value.files.filter((path: unknown) => typeof path === 'string' && path.length > 0 && path.length < 2048).slice(0,24) as string[])] : [];
      const tools: ToolPaneTab[] = [];
      for (const candidate of Array.isArray(value.tools) ? value.tools.slice(0,24) : []) {
        if (!candidate || typeof candidate.id !== 'string') continue;
        const kind = toolPaneKindOf(candidate.id);
        if (!kind || kind === 'new' || kind !== candidate.kind || typeof candidate.title !== 'string') continue;
        if (kind === 'pr' && (!candidate.pr || typeof candidate.pr.owner !== 'string' || typeof candidate.pr.repo !== 'string' || !/^[\w.-]+$/.test(candidate.pr.owner) || !/^[\w.-]+$/.test(candidate.pr.repo) || !Number.isSafeInteger(candidate.pr.number) || candidate.pr.number < 1)) continue;
        if (tools.some(tab => tab.id === candidate.id)) continue;
        tools.push({id:candidate.id,kind,title:candidate.title.slice(0,120),...(kind === 'pr' ? {pr:{owner:candidate.pr.owner,repo:candidate.pr.repo,number:candidate.pr.number}} : {})});
      }
      const sources: Record<string,string> = {};
      for (const [id,source] of Object.entries(value.sources ?? {}).slice(-24)) if (typeof source === 'string' && /^turn:\d+$/.test(source)) sources[id] = source;
      return {width:Number.isFinite(value.width) ? Math.max(280,Math.min(800,value.width)) : 390,files,tools,active:typeof value.active === 'string' ? value.active : null,sources};
    } catch { return empty(); }
  }
  function save() {
    if (restoring || !workspace || !owner) return;
    const state = ui.source.get();
    const turn = state.reviewTurn;
    if (!pendingSource) {
      if (turn?.session === owner && turn.turnKey && /^turn:\d+$/.test(turn.turnKey)) layout.sources[owner] = turn.turnKey;
      else delete layout.sources[owner];
    }
    layout = {width:state.rightPanelWidth,files:state.fileTabs.slice(-24).map(tab => tab.path),tools:state.toolTabs.filter(tab => tab.kind !== 'new').slice(-24),active:state.activeAgentTab,sources:Object.fromEntries(Object.entries(layout.sources).slice(-24))};
    const serialized = JSON.stringify({version:1,...layout});
    records.delete(key(workspace)); records.set(key(workspace),serialized);
    while (records.size > 24) records.delete(records.keys().next().value!);
    writes.set(key(workspace),serialized);
    clearTimeout(timer); timer = setTimeout(flush,100);
  }
  function sync() {
    const state = sessions.get();
    const session = selectActiveSession(state);
    const nextOwner = state.active ?? state.selected;
    if (!nextOwner || !session.cwd) return;
    if (workspace !== session.cwd || owner !== nextOwner) {
      save();
      const changedWorkspace = workspace !== session.cwd;
      workspace = session.cwd; owner = nextOwner;
      if (changedWorkspace) layout = read(workspace);
      pendingSource = layout.sources[owner] ?? null;
      restoring = true;
      if (changedWorkspace) {
        ui.update('rightPanelWidth',layout.width);
        ui.update('agentTabs',[]);
        ui.update('fileTabs',layout.files.map(path => ({id:path,path,diff:null})));
        ui.update('toolTabs',layout.tools);
        const ids = [...layout.files,...layout.tools.map(tab=>tab.id)];
        ui.update('activeAgentTab',layout.active && ids.includes(layout.active) ? layout.active : ids.at(-1) ?? null);
      }
      ui.update('reviewTurn',null);
      ui.update('reviewFocus',null);
      restoring = false;
    }
    if (pendingSource && session.entries.length && session.turnDiffs.length) {
      const source = reviewTurnSources(session.entries,session.turnDiffs).find(source => source.key === pendingSource);
      restoring = true;
      if (source) { ui.update('reviewTurn',{session:owner!,summary:source.summary,turnKey:source.key}); pendingSource = null; }
      else if (!session.historyCursor && !session.historyLoading) pendingSource = null;
      restoring = false;
    }
  }
  const offSessions = sessions.subscribe(sync);
  const offUi = ui.source.subscribe(save);
  sync();
  return { dispose() { save(); flush(); offSessions.unsubscribe(); offUi.unsubscribe(); } };
}
