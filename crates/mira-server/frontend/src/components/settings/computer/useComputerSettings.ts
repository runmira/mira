import { useCallback, useEffect, useRef, useState } from 'react';
import { getComputerSettings, updateComputerSettings, getComputerStatus, listComputerWindows, unpairComputer, revokeComputerClient,
  type ComputerView, type ComputerUpdate, type ComputerWindow, type ComputerIdentity, type PairedComputer } from './api';
const EXAMPLE: ComputerView = { enabled: false, target: 'local', delivery: 'background', window: null, computers: [], clients: [], can_change: true };
const EXAMPLE_WINDOWS: ComputerWindow[] = [{ id: 'example-window', epoch: '1', title: 'Mira', app: 'Mira' }];
const EXAMPLE_IDENTITY: ComputerIdentity = { name: 'This computer', platform: 'macOS', machine_id: 'preview', background_input: true, limitation: '', backend: 'cua' as const };

export function useComputerSettings(preview: boolean) {
  const [view, setView] = useState<ComputerView | null>(preview ? EXAMPLE : null);
  const [error, setError] = useState('');
  const [pending, setPending] = useState(false);
  const [identity, setIdentity] = useState<ComputerIdentity | null>(preview ? EXAMPLE_IDENTITY : null);
  const [windows, setWindows] = useState<ComputerWindow[]>(preview ? EXAMPLE_WINDOWS : []);
  const [connectionError, setConnectionError] = useState('');
  const [checking, setChecking] = useState(false);
  const mutation = useRef(false);
  const generation = useRef(0);
  const mounted = useRef(true);
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; generation.current++; }; }, []);
  const refresh = useCallback(async () => {
    if (preview) return;
    try { const next = await getComputerSettings(); if (mounted.current) { setView(next); setError(''); } }
    catch (e) { if (mounted.current) setError((e as Error).message); }
  }, [preview]);
  useEffect(() => { void refresh(); }, [refresh]);
  useEffect(() => {
    if (preview || view?.can_change !== false) return;
    const timer=setInterval(() => { void refresh(); },2000);
    return () => clearInterval(timer);
  },[preview,view?.can_change,refresh]);
  const target = view?.target;
  const check = useCallback(async () => {
    if (!target) return;
    const current = ++generation.current; setChecking(true); setConnectionError(''); setIdentity(null); setWindows([]);
    try {
      const status = preview ? EXAMPLE_IDENTITY : await getComputerStatus(target);
      if (current === generation.current && mounted.current) setIdentity(status);
      const nextWindows = preview ? EXAMPLE_WINDOWS : await listComputerWindows(target);
      if (current === generation.current && mounted.current) { setIdentity(status); setWindows(nextWindows); }
    } catch (e) { if (current === generation.current && mounted.current) setConnectionError((e as Error).message); }
    finally { if (current === generation.current && mounted.current) setChecking(false); }
  }, [target, preview]);
  useEffect(() => { void check(); return () => { generation.current++; }; }, [check]);

  async function change(patch: Partial<ComputerUpdate>) {
    if (!view || mutation.current) return;
    mutation.current = true; setPending(true); setError('');
    try {
      const next = { enabled: view.enabled, target: view.target, delivery: view.delivery, window: view.window, ...patch };
      const result = preview ? { ...view, ...next } : await updateComputerSettings(next);
      if (mounted.current) setView(result);
    } catch (e) {
      if (mounted.current) setError((e as Error).message);
      if (!preview) { try { const current=await getComputerSettings(); if(mounted.current)setView(current); } catch { /* preserve the original actionable error */ } }
    }
    finally { mutation.current = false; if (mounted.current) setPending(false); }
  }
  async function paired(computer: PairedComputer) {
    if (preview) setView((old) => old && ({ ...old, computers: [computer], target: computer.id, enabled: false, window: null }));
    else { await refresh(); }
  }
  async function remove(id: string, client = false) {
    if (mutation.current) return;
    mutation.current = true; setPending(true); setError('');
    try {
      if (!preview) { if (client) await revokeComputerClient(id); else await unpairComputer(id); await refresh(); }
      else setView((old) => old && ({ ...old, computers: old.computers.filter((c) => c.id !== id), target: 'local', enabled: false, window: null }));
    } catch (e) { if (mounted.current) setError((e as Error).message); }
    finally { mutation.current = false; if (mounted.current) setPending(false); }
  }
  return { view, error, pending, identity, windows, connectionError, checking, refresh, check, change, paired, remove };
}
