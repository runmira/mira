import { useEffect, useState } from 'react';
import { listSessions, loadSession } from '../api';
import type { SessionSummary } from '../types';
import type { WsStatus } from '../ws';

type Props = {
  status: WsStatus;
  cwd: string;
  activeSessionId: string;
  refreshKey: number;
  onNewChat: () => void;
  onOpenSettings: () => void;
  onOpenPicker: () => void;
  onSessionLoaded: () => void;
};

export function Sidebar({
  status,
  cwd,
  activeSessionId,
  refreshKey,
  onNewChat,
  onOpenSettings,
  onOpenPicker,
  onSessionLoaded,
}: Props) {
  const [sessions, setSessions] = useState<SessionSummary[]>([]);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    listSessions()
      .then((s) => { setSessions(s); setError(null); })
      .catch((e) => setError(String(e.message ?? e)));
  }, [refreshKey]);

  async function pickSession(id: string) {
    try {
      await loadSession(id);
      onSessionLoaded();
    } catch (e) {
      setError(String((e as Error).message));
    }
  }

  return (
    <aside className="sidebar">
      <div className="sidebar-head">
        <span className="sidebar-brand">Mira</span>
        <button className="sidebar-collapse" title="Collapse (soon)" disabled>▐</button>
      </div>

      <div className="sidebar-scroll">
        <div className="sidebar-nav">
          <button className="nav-item" onClick={onNewChat}>
            <span className="glyph">✎</span>
            <span>New chat</span>
          </button>
          <button className="nav-item disabled" title="Not implemented yet" disabled>
            <span className="glyph">⧉</span>
            <span>Plugins</span>
          </button>
          <button className="nav-item disabled" title="Not implemented yet" disabled>
            <span className="glyph">◷</span>
            <span>Scheduled</span>
          </button>
        </div>

        <div className="sidebar-section">
          <div className="sidebar-section-title">Folder</div>
          <button
            className="chat-row"
            onClick={onOpenPicker}
            title={cwd || 'Choose a folder'}
          >
            <span className="label">{shortenPath(cwd) || 'Choose folder…'}</span>
            <span className="age">change</span>
          </button>
        </div>

        <div className="sidebar-section">
          <div className="sidebar-section-title">Chats</div>
          {error && <div className="sidebar-empty">error: {error}</div>}
          {!error && sessions.length === 0 && (
            <div className="sidebar-empty">No saved chats in this folder</div>
          )}
          {sessions.map((s) => (
            <button
              key={s.id}
              className={`chat-row ${s.id === activeSessionId ? 'active' : ''}`}
              onClick={() => pickSession(s.id)}
              title={s.first_user_message ?? s.id}
            >
              <span className="label">{s.first_user_message ?? 'Untitled'}</span>
              <span className="age">{timeAgo(s.updated_at)}</span>
            </button>
          ))}
        </div>
      </div>

      <div className="sidebar-foot">
        <span className={`status-dot ${status}`} />
        <span className="meta" title={cwd}>{shortenPath(cwd) || 'connecting…'}</span>
        <button className="sidebar-collapse" onClick={onOpenSettings} title="Settings">⚙</button>
      </div>
    </aside>
  );
}

function timeAgo(unixSecs: number): string {
  const now = Math.floor(Date.now() / 1000);
  const diff = Math.max(0, now - unixSecs);
  if (diff < 60) return `${diff}s`;
  if (diff < 3600) return `${Math.floor(diff / 60)}m`;
  if (diff < 86400) return `${Math.floor(diff / 3600)}h`;
  const days = Math.floor(diff / 86400);
  if (days < 7) return `${days}d`;
  return `${Math.floor(days / 7)}w`;
}

function shortenPath(p: string): string {
  if (!p) return '';
  const parts = p.split('/');
  if (parts.length <= 3) return p;
  return '…/' + parts.slice(-2).join('/');
}
