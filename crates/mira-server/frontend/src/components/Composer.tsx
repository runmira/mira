import { useEffect, useMemo, useRef, useState } from 'react';
import { listModels, type ModelInfo } from '../api';
import type { Mode } from '../types';

const MODES: { value: Mode; label: string; desc: string }[] = [
  { value: 'plan',   label: 'Plan only',   desc: 'Reads and searches. No edits, no commands.' },
  { value: 'manual', label: 'Ask each time', desc: 'Prompt before every edit or command.' },
  { value: 'auto',   label: 'Auto edits',  desc: 'Auto-approve edits. Prompt on commands.' },
  { value: 'edit',   label: 'Auto everything', desc: 'Edits + commands run unless a rule blocks.' },
  { value: 'yolo',   label: 'Yolo',        desc: 'No gating at all.' },
];

type Props = {
  disabled: boolean;
  busy: boolean;
  mode: Mode;
  model: string;
  cwd: string;
  onSend: (text: string) => void;
  onSetMode: (m: Mode) => void;
  onSetModel: (m: string) => void;
  onOpenPicker: () => void;
  onInterrupt: () => void;
};

export function Composer({
  disabled, busy, mode, model, cwd,
  onSend, onSetMode, onSetModel, onOpenPicker, onInterrupt,
}: Props) {
  const [text, setText] = useState('');
  const [modePop, setModePop] = useState(false);
  const [modelPop, setModelPop] = useState(false);

  function submit() {
    const trimmed = text.trim();
    if (!trimmed || disabled || busy) return;
    onSend(trimmed);
    setText('');
  }

  const modeLabel = MODES.find((m) => m.value === mode)?.label ?? mode;

  return (
    <div className="composer-wrap">
      <div className="composer-above">
        <button className="link-chip" onClick={onOpenPicker} title={cwd || 'Choose a folder'}>
          <span className="glyph">🗀</span>
          <span>{cwd ? shortenPath(cwd) : 'Choose project'}</span>
        </button>
      </div>

      <form
        className="composer"
        onSubmit={(e) => { e.preventDefault(); submit(); }}
      >
        <textarea
          className="composer-input"
          value={text}
          onChange={(e) => setText(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === 'Enter' && !e.shiftKey) { e.preventDefault(); submit(); }
          }}
          placeholder={disabled ? 'Waiting for connection…' : 'Work with mira'}
          disabled={disabled}
          rows={1}
        />

        <div className="composer-row">
          <button
            type="button"
            className="chip disabled"
            title="Attachments not implemented yet"
            disabled
          >
            <span className="glyph">＋</span>
          </button>

          <ModePopover
            open={modePop}
            onOpen={() => setModePop((v) => !v)}
            onClose={() => setModePop(false)}
            mode={mode}
            label={modeLabel}
            onPick={(m) => { onSetMode(m); setModePop(false); }}
          />

          <span className="spacer" />

          <ModelPopover
            open={modelPop}
            onOpen={() => setModelPop((v) => !v)}
            onClose={() => setModelPop(false)}
            model={model}
            onPick={(m) => { onSetModel(m); setModelPop(false); }}
          />

          {busy ? (
            <button
              type="button"
              className="send stop"
              onClick={onInterrupt}
              title="Stop"
              aria-label="Stop"
            >■</button>
          ) : (
            <button
              type="submit"
              className="send"
              disabled={disabled || !text.trim()}
              title="Send"
              aria-label="Send"
            >↑</button>
          )}
        </div>
      </form>
    </div>
  );
}

/* ---------- popovers ---------- */

function ModePopover({
  open, onOpen, onClose, mode, label, onPick,
}: {
  open: boolean;
  onOpen: () => void;
  onClose: () => void;
  mode: Mode;
  label: string;
  onPick: (m: Mode) => void;
}) {
  const ref = useOutsideClick<HTMLDivElement>(onClose, open);
  return (
    <div className="popover-wrap" ref={ref}>
      <button type="button" className="chip" onClick={onOpen}>
        <span className="glyph">◐</span>
        <span>{label}</span>
      </button>
      {open && (
        <div className="popover">
          <div className="popover-label">Approval</div>
          <div className="popover-list">
            {MODES.map((m) => (
              <button
                key={m.value}
                type="button"
                className={`opt ${m.value === mode ? 'on' : ''}`}
                onClick={() => onPick(m.value)}
              >
                <span>{m.label}</span>
                {m.value === mode && <span className="check">✓</span>}
                <div className="desc">{m.desc}</div>
              </button>
            ))}
          </div>
        </div>
      )}
    </div>
  );
}

function ModelPopover({
  open, onOpen, onClose, model, onPick,
}: {
  open: boolean;
  onOpen: () => void;
  onClose: () => void;
  model: string;
  onPick: (m: string) => void;
}) {
  const ref = useOutsideClick<HTMLDivElement>(onClose, open);
  const [models, setModels] = useState<ModelInfo[] | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [query, setQuery] = useState('');
  const [activeIdx, setActiveIdx] = useState(0);
  const inputRef = useRef<HTMLInputElement | null>(null);
  const listRef = useRef<HTMLDivElement | null>(null);

  // Lazy-fetch the catalog the first time the popover opens.
  useEffect(() => {
    if (!open || models !== null) return;
    listModels()
      .then((v) => { setModels(v.models); setLoadError(null); })
      .catch((e) => { setModels([]); setLoadError(String((e as Error).message)); });
  }, [open, models]);

  useEffect(() => { if (open) setTimeout(() => inputRef.current?.focus(), 0); }, [open]);
  useEffect(() => { if (open) { setQuery(''); setActiveIdx(0); } }, [open]);

  // Keep the active row in view as arrow keys move through the grouped list.
  useEffect(() => {
    if (!open) return;
    const el = listRef.current?.querySelector('.mp-row.active') as HTMLElement | null;
    if (el) el.scrollIntoView({ block: 'nearest' });
  }, [activeIdx, open]);

  const filtered = useMemo(() => {
    if (!models) return [];
    const q = query.trim().toLowerCase();
    if (!q) return models;
    return models.filter((m) => {
      const hay = `${m.id} ${m.display_name ?? ''} ${m.owned_by ?? ''}`.toLowerCase();
      return q.split(/\s+/).every((tok) => hay.includes(tok));
    });
  }, [models, query]);

  const pending = models === null;
  const empty = !pending && filtered.length === 0;
  const hasFreeText = query.trim() && (empty || !filtered.some((m) => m.id === query.trim()));

  function commit(id: string) {
    if (id.trim()) onPick(id.trim());
  }

  // Group visible rows by provider (part before the first `/` in id, or the
  // `owned_by` field when there's no slash). Rendered as titled sections.
  const groups = useMemo(() => groupModels(filtered.slice(0, 200)), [filtered]);
  const flatVisible = useMemo(() => groups.flatMap((g) => g.models), [groups]);

  function onKey(e: React.KeyboardEvent<HTMLInputElement>) {
    if (e.key === 'ArrowDown') {
      e.preventDefault();
      const next = Math.min(activeIdx + 1, Math.max(flatVisible.length - 1, 0));
      setActiveIdx(next);
    } else if (e.key === 'ArrowUp') {
      e.preventDefault();
      const next = Math.max(activeIdx - 1, 0);
      setActiveIdx(next);
    } else if (e.key === 'Enter') {
      e.preventDefault();
      if (flatVisible[activeIdx]) commit(flatVisible[activeIdx].id);
      else if (query.trim()) commit(query);
    } else if (e.key === 'Escape') {
      onClose();
    }
  }

  return (
    <div className="popover-wrap" ref={ref}>
      <button type="button" className="chip model" onClick={onOpen}>
        <span className={`vendor-dot vendor-${vendorOf(model)}`} />
        <span className="chip-label">{prettyLabel(model) || 'model'}</span>
        <span className="caret">▾</span>
      </button>
      {open && (
        <div className="popover popover-models" role="listbox">
          <div className="mp-search">
            <span className="mp-search-icon" aria-hidden>⌕</span>
            <input
              type="text"
              ref={inputRef}
              value={query}
              onChange={(e) => { setQuery(e.target.value); setActiveIdx(0); }}
              onKeyDown={onKey}
              placeholder={pending ? 'loading models…' : 'Search models'}
              spellCheck={false}
              autoComplete="off"
            />
            {!pending && models && models.length > 0 && (
              <span className="mp-count">{filtered.length}/{models.length}</span>
            )}
          </div>

          {loadError && <div className="mp-hint error">{loadError}</div>}
          {pending && !loadError && (
            <div className="mp-loading">
              <span className="mp-spinner" /> fetching catalog…
            </div>
          )}

          <div className="mp-list" ref={listRef}>
            {groups.map((group) => (
              <div key={group.name} className="mp-group">
                <div className="mp-group-title">
                  <span className={`vendor-dot vendor-${group.vendor}`} />
                  <span>{group.name}</span>
                  <span className="mp-group-count">{group.models.length}</span>
                </div>
                {group.models.map((m) => {
                  const flatIdx = flatVisible.indexOf(m);
                  const selected = m.id === model;
                  const active = flatIdx === activeIdx;
                  return (
                    <button
                      key={m.id}
                      type="button"
                      role="option"
                      aria-selected={selected}
                      className={`mp-row ${selected ? 'selected' : ''} ${active ? 'active' : ''}`}
                      onMouseEnter={() => setActiveIdx(flatIdx)}
                      onClick={() => commit(m.id)}
                      title={m.id}
                    >
                      <div className="mp-row-main">
                        <div className="mp-row-name">
                          {m.display_name || prettyLabel(m.id)}
                          {selected && <span className="mp-check">✓</span>}
                        </div>
                        <div className="mp-row-slug">{m.id}</div>
                      </div>
                      {m.context_length && (
                        <span className="mp-ctx-pill">{formatCtx(m.context_length)}</span>
                      )}
                    </button>
                  );
                })}
              </div>
            ))}

            {hasFreeText && (
              <button
                type="button"
                className="mp-row free-text"
                onClick={() => commit(query)}
              >
                <div className="mp-row-main">
                  <div className="mp-row-name">Use <code>{query.trim()}</code></div>
                  <div className="mp-row-slug">Send this exact id to the provider</div>
                </div>
                <span className="mp-enter-hint">↵</span>
              </button>
            )}
            {!pending && filtered.length === 0 && !hasFreeText && (
              <div className="mp-hint">no matches</div>
            )}
            {filtered.length > 200 && (
              <div className="mp-hint">{filtered.length - 200} more — narrow with search</div>
            )}
          </div>

          <div className="mp-foot">
            <span><kbd>↑↓</kbd> nav</span>
            <span><kbd>↵</kbd> pick</span>
            <span><kbd>esc</kbd> close</span>
          </div>
        </div>
      )}
    </div>
  );
}

type Group = { name: string; vendor: string; models: ModelInfo[] };

/**
 * Group models by their vendor prefix — the part before the first slash
 * ("openai/gpt-5" → "openai"), falling back to `owned_by` for ids without
 * one. Groups are ordered by size (biggest first) so busy providers show
 * up on top; models within a group keep their alphabetical order.
 */
function groupModels(models: ModelInfo[]): Group[] {
  const byKey = new Map<string, ModelInfo[]>();
  for (const m of models) {
    const key = vendorOf(m.id) || (m.owned_by ?? 'other');
    const bucket = byKey.get(key);
    if (bucket) bucket.push(m);
    else byKey.set(key, [m]);
  }
  return [...byKey.entries()]
    .map(([name, list]) => ({ name: prettyVendor(name), vendor: vendorClass(name), models: list }))
    .sort((a, b) => b.models.length - a.models.length);
}

const VENDOR_CLASSES: Record<string, string> = {
  openai: 'openai', anthropic: 'anthropic', google: 'google', 'google-vertex': 'google',
  meta: 'meta', 'meta-llama': 'meta', mistralai: 'mistral', mistral: 'mistral',
  deepseek: 'deepseek', xai: 'xai', qwen: 'qwen', 'nousresearch': 'nous',
  microsoft: 'microsoft', amazon: 'amazon', cohere: 'cohere', groq: 'groq',
  perplexity: 'perplexity',
};

function vendorOf(id: string): string {
  const slash = id.indexOf('/');
  if (slash > 0) return id.slice(0, slash).toLowerCase();
  return id.split('-')[0].toLowerCase();
}

function vendorClass(key: string): string {
  return VENDOR_CLASSES[key.toLowerCase()] ?? 'other';
}

function prettyVendor(key: string): string {
  const overrides: Record<string, string> = {
    openai: 'OpenAI', anthropic: 'Anthropic', google: 'Google', xai: 'xAI',
    meta: 'Meta', 'meta-llama': 'Meta', mistralai: 'Mistral', deepseek: 'DeepSeek',
    qwen: 'Qwen', amazon: 'Amazon', cohere: 'Cohere', groq: 'Groq',
    perplexity: 'Perplexity', microsoft: 'Microsoft',
  };
  return overrides[key.toLowerCase()] ?? key.charAt(0).toUpperCase() + key.slice(1);
}

function prettyLabel(id: string): string {
  if (!id) return '';
  // Strip vendor prefix for the chip so the header stays tight.
  const slash = id.indexOf('/');
  return slash > 0 ? id.slice(slash + 1) : id;
}

function formatCtx(n: number): string {
  if (n >= 1000) return `${Math.round(n / 1000)}k`;
  return String(n);
}

/* ---------- helpers ---------- */

function shortenPath(p: string): string {
  if (!p) return '';
  const parts = p.split('/');
  if (parts.length <= 3) return p;
  return '…/' + parts.slice(-2).join('/');
}

function useOutsideClick<T extends HTMLElement>(onClose: () => void, active: boolean) {
  const ref = useRef<T | null>(null);
  useEffect(() => {
    if (!active) return;
    function handler(e: MouseEvent) {
      if (!ref.current) return;
      if (!ref.current.contains(e.target as Node)) onClose();
    }
    document.addEventListener('mousedown', handler);
    return () => document.removeEventListener('mousedown', handler);
  }, [active, onClose]);
  return ref;
}
