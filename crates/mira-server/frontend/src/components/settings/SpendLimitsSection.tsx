import { useEffect, useState } from 'react';
import { DollarSign } from 'lucide-react';
import { SectionInput } from '../ui/input';
import { useAppRuntime } from '../../state/RuntimeProvider';
import { useResource } from '../../state/resources/hooks';
import type { SpendCaps } from '../../types';
import { TRow, TSection } from './SettingsFields';

const fields = [['per_session', 'Per session'], ['per_day', 'Per day'], ['per_month', 'Per month']] as const;
export function SpendLimitsSection() {
  const runtime = useAppRuntime();
  const settings = useResource('settings');
  const [values, setValues] = useState<Record<string, string>>({});
  const [project, setProject] = useState('');
  const [projectValues, setProjectValues] = useState<Record<string, string>>({});
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [saved, setSaved] = useState(false);
  useEffect(() => {
    if (!settings.data) return;
    setValues(Object.fromEntries(fields.map(([key]) => [key, String(settings.data?.spend_limits?.[key] ?? '')])));
  }, [settings.data?.spend_limits]);
  function selectProject(path: string) {
    setProject(path);
    setProjectValues(Object.fromEntries(fields.map(([key]) => [key, String(settings.data?.spend_limits?.projects?.[path]?.[key] ?? '')])));
    setSaved(false);
  }
  async function save() {
    if (!settings.data || saving) return;
    setError(null); setSaved(false);
    function parse(values: Record<string, string>): SpendCaps {
      return Object.fromEntries(fields.map(([key]) => {
        const text = values[key]?.trim() ?? '';
        const n = text ? Number(text) : null;
        if (n !== null && (!Number.isFinite(n) || n <= 0)) throw new Error('Enter positive dollar amounts, or leave blank for no limit.');
        return [key, n];
      }));
    }
    try {
      const caps = parse(values);
      const projects = { ...settings.data.spend_limits?.projects };
      if (project.trim()) {
        if (!project.startsWith('/') && !/^[A-Za-z]:[\\/]/.test(project)) throw new Error('Use the full project folder path.');
        const projectCaps = parse(projectValues);
        if (Object.values(projectCaps).every(v => v == null)) delete projects[project];
        else projects[project] = projectCaps;
      }
      setSaving(true);
      await runtime.resources.saveSettings({ spend_limits: { ...caps, projects } });
      setSaved(true);
    } catch (e) { setError(e instanceof Error ? e.message : String(e)); }
    finally { setSaving(false); }
  }
  const disabled = !settings.data || saving;
  return <TSection icon={<DollarSign className="size-3.5" />} title="Spend limits" description="Dollar caps for chats, subagents and background runs. Daily and monthly totals reset at midnight UTC. A request already in flight may finish above the cap. Subscription usage is excluded.">
    {fields.map(([key, title]) => <TRow key={key} title={title} description="Leave blank for no limit." control={<SectionInput aria-label={`${title} spend limit in dollars`} type="number" min="0.01" step="0.01" disabled={disabled} value={values[key] ?? ''} onChange={e => { setValues(v => ({ ...v, [key]: e.target.value })); setSaved(false); }} placeholder="No limit" className="h-8 sm:w-40" />} />)}
    <TRow title="Project folder (optional)" description="Project caps apply alongside your overall caps. Enter an existing folder to edit its limits." control={<SectionInput aria-label="Project folder for spend limits" list="spend-projects" value={project} disabled={disabled} onChange={e => selectProject(e.target.value)} placeholder="/path/to/project" className="h-8 sm:w-64" />} />
    <datalist id="spend-projects">{Object.keys(settings.data?.spend_limits?.projects ?? {}).map(path => <option key={path} value={path} />)}</datalist>
    {project && fields.map(([key, title]) => <TRow key={key} title={`Project: ${title.toLowerCase()}`} control={<SectionInput aria-label={`Project ${title.toLowerCase()} spend limit in dollars`} type="number" min="0.01" step="0.01" disabled={disabled} value={projectValues[key] ?? ''} onChange={e => { setProjectValues(v => ({ ...v, [key]: e.target.value })); setSaved(false); }} placeholder="No project limit" className="h-8 sm:w-40" />} />)}
    <div className="flex items-center gap-3 px-3 py-2">
      <button type="button" className="rounded-md border px-3 py-1.5 text-xs disabled:opacity-50" disabled={disabled} onClick={() => void save()}>{saving ? 'Saving…' : 'Save spend limits'}</button>
      {saved && <span role="status" className="text-xs text-muted-foreground">Saved</span>}
      {(error || settings.error) && <span role="alert" className="text-xs text-destructive">{error ?? 'Could not load spend limits.'}</span>}
    </div>
  </TSection>;
}
