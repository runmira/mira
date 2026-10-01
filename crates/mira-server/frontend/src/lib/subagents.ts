/**
 * Mira's subagents — the helpers Mira's model delegates to — as the app
 * sees them: one shared roster, fetched once and refreshed after edits, so
 * Settings, the transcript and the subagent panel all show the same names
 * and faces.
 */
import { useEffect, useState } from 'react';
import type { FaceSpec } from '../components/SubagentFace';

export type Subagent = {
  /** Stable id the model uses (`explore`, `reviewer`). */
  name: string;
  /** Persona name shown everywhere (`Scout`). */
  display_name?: string | null;
  description: string;
  category?: string | null;
  tools?: string[] | null;
  model?: string | null;
  max_rounds?: number | null;
  /** The instructions (the agent file's body). */
  system_prompt_addendum?: string | null;
  parallel_safe?: boolean | null;
  route_approvals_to_parent?: boolean | null;
  worktree?: boolean | null;
  review_required?: boolean | null;
  enabled?: boolean | null;
  face?: FaceSpec | null;
  /** `builtin`, `plugin`, `user`, `project`. */
  source?: string | null;
  has_builtin: boolean;
  customized: boolean;
};

export type SubagentsView = { subagents: Subagent[]; tools: string[] };

export type SubagentEdit = Partial<{
  name: string;
  display_name: string;
  description: string;
  category: string;
  tools: string[];
  model: string;
  max_rounds: number;
  worktree: boolean;
  route_approvals_to_parent: boolean;
  review_required: boolean;
  parallel_safe: boolean;
  enabled: boolean;
  face: FaceSpec;
  instructions: string;
}>;

async function call(method: string, url: string, body?: unknown): Promise<SubagentsView> {
  const r = await fetch(url, {
    method,
    headers: body ? { 'content-type': 'application/json' } : undefined,
    body: body ? JSON.stringify(body) : undefined,
  });
  const json = (await r.json().catch(() => ({}))) as SubagentsView & { error?: string };
  if (!r.ok) throw new Error(json.error ?? `HTTP ${r.status}`);
  return json;
}

let cache: SubagentsView | null = null;
let inflight: Promise<SubagentsView> | null = null;
const listeners = new Set<(v: SubagentsView) => void>();

function publish(v: SubagentsView) {
  cache = v;
  for (const l of listeners) l(v);
}

export function refreshSubagents(): Promise<SubagentsView> {
  if (!inflight) {
    inflight = call('GET', '/api/subagents')
      .then((v) => {
        publish(v);
        return v;
      })
      .finally(() => {
        inflight = null;
      });
  }
  return inflight;
}

export async function saveSubagent(name: string, edit: SubagentEdit) {
  publish(await call('PUT', `/api/subagents/${encodeURIComponent(name)}`, edit));
}

export async function createSubagent(edit: SubagentEdit) {
  publish(await call('POST', '/api/subagents', edit));
}

export async function deleteSubagent(name: string) {
  publish(await call('DELETE', `/api/subagents/${encodeURIComponent(name)}`));
}

/** The roster, kept current across the app. */
export function useSubagents(): SubagentsView | null {
  const [v, setV] = useState<SubagentsView | null>(cache);
  useEffect(() => {
    listeners.add(setV);
    if (!cache) void refreshSubagents().catch(() => {});
    return () => {
      listeners.delete(setV);
    };
  }, []);
  return v;
}

/** One subagent by id, from the shared roster. */
export function useSubagent(name: string | null | undefined): Subagent | null {
  const v = useSubagents();
  if (!name) return null;
  return v?.subagents.find((s) => s.name === name) ?? null;
}

/** The name to show for a subagent id. */
export function personaName(s: Subagent | null | undefined, fallback: string): string {
  return s?.display_name || (fallback ? fallback.charAt(0).toUpperCase() + fallback.slice(1) : 'Subagent');
}
