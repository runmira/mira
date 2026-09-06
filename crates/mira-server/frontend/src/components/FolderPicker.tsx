import { useEffect, useState } from 'react';
import { browse, putCwd, type BrowseView } from '../api';

type Props = {
  open: boolean;
  onClose: () => void;
  onPicked: (path: string) => void;
};

export function FolderPicker({ open, onClose, onPicked }: Props) {
  const [view, setView] = useState<BrowseView | null>(null);
  const [showHidden, setShowHidden] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [manual, setManual] = useState('');

  useEffect(() => {
    if (!open) return;
    setError(null);
    load(undefined);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open]);

  async function load(path: string | undefined) {
    try {
      const v = await browse(path, showHidden);
      setView(v);
      setManual(v.path);
      setError(null);
    } catch (e) {
      setError(String((e as Error).message));
    }
  }

  async function pick(path: string) {
    setBusy(true);
    try {
      await putCwd(path);
      onPicked(path);
      onClose();
    } catch (e) {
      setError(String((e as Error).message));
    } finally {
      setBusy(false);
    }
  }

  useEffect(() => {
    if (!open || !view) return;
    load(view.path);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [showHidden]);

  if (!open) return null;

  return (
    <div className="modal-backdrop" onClick={onClose}>
      <div className="modal picker" onClick={(e) => e.stopPropagation()}>
        <div className="modal-head">
          <div className="modal-title">Choose project folder</div>
          <button className="link" onClick={onClose} aria-label="Close">✕</button>
        </div>

        <div className="picker-quick">
          {view?.home && (
            <button className="chip" onClick={() => load(view.home!)}>Home</button>
          )}
          <button className="chip" onClick={() => load('/')}>/</button>
          {view?.parent && (
            <button className="chip" onClick={() => load(view.parent!)} title="Up one">⬆ up</button>
          )}
          <label className="picker-hidden">
            <input
              type="checkbox"
              checked={showHidden}
              onChange={(e) => setShowHidden(e.target.checked)}
            /> show hidden
          </label>
        </div>

        <div className="picker-path">
          <input
            type="text"
            value={manual}
            onChange={(e) => setManual(e.target.value)}
            onKeyDown={(e) => { if (e.key === 'Enter') { e.preventDefault(); load(manual.trim()); } }}
            spellCheck={false}
            placeholder="/absolute/path or ~/relative"
          />
        </div>

        {error && <div className="error">{error}</div>}

        <div className="picker-list">
          {view?.entries.length === 0 && !error && (
            <div className="picker-empty">Empty folder</div>
          )}
          {view?.entries.map((e) => (
            <button
              key={e.path}
              className={`picker-row ${e.is_dir ? 'dir' : 'file'}`}
              onDoubleClick={() => e.is_dir && load(e.path)}
              onClick={() => e.is_dir && load(e.path)}
              disabled={!e.is_dir}
              title={e.path}
            >
              <span className="glyph">{e.is_dir ? '🗀' : '·'}</span>
              <span className="name">{e.name}</span>
            </button>
          ))}
          {view?.truncated && (
            <div className="picker-empty">… list truncated at 500 entries</div>
          )}
        </div>

        <div className="modal-actions">
          <button className="secondary" onClick={onClose} disabled={busy}>Cancel</button>
          <button
            className="primary"
            onClick={() => view && pick(view.path)}
            disabled={busy || !view}
            title={view?.path}
          >
            {busy ? 'Setting…' : 'Select this folder'}
          </button>
        </div>
      </div>
    </div>
  );
}
