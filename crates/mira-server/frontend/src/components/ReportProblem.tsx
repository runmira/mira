/**
 * "Report a problem": builds the diagnostics bundle (version, platform,
 * config, recent logs, optionally this chat) with secrets removed, shows
 * every file in full, and lets the user save it and open a GitHub issue.
 * Nothing is sent anywhere by Mira; the user attaches the zip themselves.
 *
 * Same bundle as `mira doctor --bundle` (crates/mira-diagnostics).
 */
import { useEffect, useState } from 'react';
import { Bug, Check, Download, ExternalLink, FileText, FolderOpen, Loader2 } from 'lucide-react';
import { Dialog, DialogContent, DialogTitle } from './ui/dialog';
import { Button } from './ui/button';
import { isDesktop, openExternal } from '../lib/desktop';
import { openInEditor } from '../lib/editors';
import { cn } from '@/lib/utils';

interface BundleFile {
  name: string;
  contents: string;
  note?: string;
}

interface Preview {
  files: BundleFile[];
  total_bytes: number;
  file_name: string;
  crash_reports_on: boolean;
  issue_url: string;
}

function size(bytes: number): string {
  if (bytes >= 1024 * 1024) return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
  if (bytes >= 1024) return `${Math.round(bytes / 1024)} KB`;
  return `${bytes} B`;
}

function query(sessionId: string | null): string {
  return sessionId ? `?session=${encodeURIComponent(sessionId)}` : '';
}

async function errorOf(r: Response): Promise<string> {
  try {
    const j = await r.json();
    if (j?.error) return j.error;
  } catch {
    /* not json */
  }
  return `${r.status} ${r.statusText}`;
}

export function ReportProblem({
  open,
  onOpenChange,
  sessionId,
  chatTitle,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /** The chat on screen, offered (unticked) for inclusion. */
  sessionId?: string;
  chatTitle?: string;
}) {
  const [includeChat, setIncludeChat] = useState(false);
  const [preview, setPreview] = useState<Preview | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [viewing, setViewing] = useState<string>('README.txt');
  const [saving, setSaving] = useState(false);
  const [saved, setSaved] = useState<{ path: string; reveal_with: string } | null>(null);

  const session = includeChat && sessionId ? sessionId : null;

  useEffect(() => {
    if (!open) return;
    let live = true;
    setPreview(null);
    setError(null);
    setSaved(null);
    fetch(`/api/diagnostics${query(session)}`, { cache: 'no-store' })
      .then(async (r) => {
        if (!r.ok) throw new Error(await errorOf(r));
        return (await r.json()) as Preview;
      })
      .then((p) => {
        if (!live) return;
        setPreview(p);
        setViewing((v) => (p.files.some((f) => f.name === v) ? v : p.files[0]?.name ?? ''));
      })
      .catch((e) => live && setError((e as Error).message));
    return () => {
      live = false;
    };
  }, [open, session]);

  async function save() {
    setSaving(true);
    setError(null);
    try {
      if (isDesktop()) {
        // The desktop webview doesn't download files; the server writes it.
        const r = await fetch(`/api/diagnostics/save${query(session)}`, { method: 'POST' });
        if (!r.ok) throw new Error(await errorOf(r));
        setSaved(await r.json());
      } else {
        const r = await fetch(`/api/diagnostics/bundle${query(session)}`, { cache: 'no-store' });
        if (!r.ok) throw new Error(await errorOf(r));
        const a = document.createElement('a');
        a.href = URL.createObjectURL(await r.blob());
        a.download = preview?.file_name ?? 'mira-diagnostics.zip';
        a.click();
        URL.revokeObjectURL(a.href);
        setSaved({ path: a.download, reveal_with: '' });
      }
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setSaving(false);
    }
  }

  const file = preview?.files.find((f) => f.name === viewing);

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="flex max-h-[min(88vh,760px)] max-w-4xl flex-col gap-3 p-5">
        <div className="flex flex-col gap-1 pr-6">
          <DialogTitle className="flex items-center gap-2 text-[16px] font-semibold">
            <Bug className="size-4 text-muted-foreground" />
            Report a problem
          </DialogTitle>
          <p className="text-[12.5px] leading-relaxed text-muted-foreground">
            This bundle has Mira's version, your settings and recent logs, with API keys, tokens and your home folder
            removed. Nothing is sent: save it, check it, and attach it to a GitHub issue.
          </p>
        </div>

        {sessionId && (
          <label className="flex cursor-pointer items-start gap-2.5 rounded-lg border border-fg/[0.08] bg-fg/[0.03] px-3 py-2">
            <input
              type="checkbox"
              className="mt-0.5 accent-mira-blue"
              checked={includeChat}
              onChange={(e) => setIncludeChat(e.target.checked)}
            />
            <span className="min-w-0 text-[12.5px]">
              <span className="block truncate text-foreground">Include this chat{chatTitle ? `: ${chatTitle}` : ''}</span>
              <span className="block text-muted-foreground">
                Helps most when the problem happened here. It holds your messages and the agent's tool output.
              </span>
            </span>
          </label>
        )}

        <div className="flex min-h-0 flex-1 flex-col gap-2 sm:flex-row">
          <ul className="flex shrink-0 flex-row gap-1 overflow-x-auto sm:w-64 sm:flex-col sm:overflow-y-auto" aria-label="Files in the bundle">
            {!preview && !error && (
              <li className="flex items-center gap-2 px-2 py-1.5 text-[12.5px] text-muted-foreground">
                <Loader2 className="size-3.5 animate-spin" /> Collecting…
              </li>
            )}
            {preview?.files.map((f) => (
              <li key={f.name}>
                <button
                  type="button"
                  onClick={() => setViewing(f.name)}
                  className={cn(
                    'flex w-full min-w-40 items-center gap-2 rounded-md px-2 py-1.5 text-left text-[12px] transition-colors',
                    viewing === f.name ? 'bg-fg/[0.08] text-foreground' : 'text-muted-foreground hover:bg-fg/[0.04]',
                  )}
                >
                  <FileText className="size-3.5 shrink-0" />
                  <span className="min-w-0 flex-1 truncate font-mono" title={f.name}>
                    {f.name}
                  </span>
                  <span className="shrink-0 tabular-nums text-[11px] text-muted-foreground/70">{size(f.contents.length)}</span>
                </button>
              </li>
            ))}
          </ul>
          <div className="flex min-h-48 min-w-0 flex-1 flex-col overflow-hidden rounded-lg border border-fg/[0.08] bg-shade/30">
            {file?.note && (
              <div className="border-b border-fg/[0.06] px-3 py-1 text-[11px] text-muted-foreground">Only the {file.note}</div>
            )}
            <pre className="min-h-0 flex-1 overflow-auto whitespace-pre-wrap break-words p-3 font-mono text-[11.5px] leading-relaxed text-foreground/85">
              {error ? error : file?.contents ?? ''}
            </pre>
          </div>
        </div>

        {preview && !preview.crash_reports_on && (
          <p className="text-[11.5px] text-muted-foreground">
            Crash reports are off. To have Mira save a report when it crashes, run{' '}
            <code className="font-mono text-foreground/80">mira config set diagnostics.crash_reports true</code>. They stay on
            this computer.
          </p>
        )}

        <div className="flex flex-wrap items-center justify-end gap-2">
          {saved && (
            <span className="mr-auto flex min-w-0 items-center gap-1.5 text-[12px] text-emerald-400">
              <Check className="size-3.5 shrink-0" />
              <span className="truncate" title={saved.path}>
                Saved {saved.path}
              </span>
              {saved.reveal_with && (
                <button
                  type="button"
                  className="ml-1 inline-flex shrink-0 items-center gap-1 text-muted-foreground hover:text-foreground"
                  onClick={() => openInEditor(saved.path, saved.reveal_with).catch((e) => setError((e as Error).message))}
                >
                  <FolderOpen className="size-3.5" /> Show
                </button>
              )}
            </span>
          )}
          <Button variant="outline" size="sm" disabled={!preview || saving} onClick={save}>
            {saving ? <Loader2 className="animate-spin" /> : <Download />}
            Save bundle{preview ? ` (${size(preview.total_bytes)})` : ''}
          </Button>
          <Button size="sm" disabled={!preview} onClick={() => preview && openExternal(preview.issue_url)}>
            <ExternalLink /> Open a GitHub issue
          </Button>
        </div>
      </DialogContent>
    </Dialog>
  );
}
