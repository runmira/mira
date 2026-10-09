import { cn } from '@/lib/utils';
import { Check, FolderOpen, Info, RotateCw } from 'lucide-react';
import type { SettingsView } from '../../types';
import { TRow, TSection } from './SettingsFields';
/* ---------- section: about ---------- */

export function AboutSection({ view }: { view: SettingsView }) {
  return (
    <TSection
      icon={<Info className="size-3.5" />}
      title="About"
      description="Where Mira reads and writes settings on this machine."
    >
      <TRow
        title="Config file"
        control={
          <div className="flex min-w-0 items-center gap-2 rounded-md border border-border/50 bg-background/40 px-2.5 py-1.5 text-[12.5px] sm:w-64">
            <FolderOpen className="size-3.5 shrink-0 text-muted-foreground" />
            <code className="min-w-0 truncate font-mono text-foreground/85">
              {view.config_path}
            </code>
          </div>
        }
      />
      <TRow
        title="Status"
        control={
          <div
            className={cn(
              'inline-flex items-center gap-1.5 rounded-full px-2.5 py-1 text-[12.5px]',
              view.configured
                ? 'bg-emerald-500/10 text-emerald-400'
                : 'bg-amber-500/10 text-amber-300',
            )}
          >
            {view.configured ? <Check className="size-3.5" /> : <RotateCw className="size-3.5" />}
            {view.configured ? 'Configured — provider ready' : 'Needs API key'}
          </div>
        }
      />
    </TSection>
  );
}
