import { useState } from 'react';
import { createRoot } from 'react-dom/client';
import { Moon, RotateCcw, Sun } from 'lucide-react';
import { ComputerSection } from './components/settings/ComputerSection';
import { SETTINGS_SECTIONS } from './components/settings/sections';
import { cn } from './lib/utils';
import { applyTheme } from './lib/theme';
import { applyAppearance } from './lib/appearance';
import miraLogo from './assets/mira-logo.png';
import './styles.css';

applyTheme();
applyAppearance();

/** Isolated design review: no backend, authentication or API calls. */
function Preview() {
  const [dark, setDark] = useState(document.documentElement.classList.contains('dark'));
  const [revision, setRevision] = useState(0);
  const [scale, setScale] = useState(1.25);
  function toggleTheme() {
    const next = !dark; setDark(next);
    document.documentElement.dataset.theme = next ? 'dark' : 'light';
    document.documentElement.classList.toggle('dark', next);
    document.documentElement.style.colorScheme = next ? 'dark' : 'light';
  }
  return <div style={{ zoom: scale, height: `calc(100dvh / ${scale})` }} className="flex min-h-0 bg-background text-foreground">
    <aside className="hidden w-56 shrink-0 flex-col border-r border-border/50 bg-mira-sidebar sm:flex">
      <div className="flex h-14 shrink-0 items-center gap-2.5 px-5"><img src={miraLogo} alt="" className="size-6 rounded-lg" /><span className="text-[13.5px] font-medium">Mira</span></div>
      <div className="min-h-0 flex-1 overflow-y-auto px-3 pb-5 pt-3">
        <div className="mb-2 px-2.5 text-[11.5px] font-semibold uppercase tracking-wider text-muted-foreground/80">Settings</div>
        <nav aria-label="Settings preview" className="flex flex-col gap-0.5">
          {SETTINGS_SECTIONS.map(({ id, label, icon: Icon }) => <div key={id} aria-current={id === 'computer' ? 'page' : undefined}
            className={cn('flex items-center gap-2.5 rounded-lg px-2.5 py-2 text-[13.5px]', id === 'computer' ? 'sidebar-active bg-fg/[0.1] text-foreground' : 'text-muted-foreground')}>
            <Icon className="size-4 shrink-0" />{label}
          </div>)}
        </nav>
      </div>
      <div className="border-t border-border/50 px-5 py-3 text-[11.5px] text-muted-foreground">Computer settings · Design preview</div>
    </aside>
    <main className="flex min-w-0 flex-1 flex-col">
      <header className="flex h-14 shrink-0 items-center justify-between gap-3 border-b border-border/50 px-5 sm:px-6">
        <h1 className="text-[15px] font-semibold"><span className="text-muted-foreground sm:hidden">Settings / </span>Computer</h1>
        <div className="flex items-center gap-1">
          <select aria-label="Preview size" value={scale} onChange={(event) => setScale(Number(event.target.value))}
            className="mr-1 h-7 rounded-md border border-border/60 bg-transparent px-1.5 text-[11px] text-muted-foreground outline-none focus-visible:ring-1 focus-visible:ring-ring">
            <option value={1}>100%</option><option value={1.25}>125%</option><option value={1.5}>150%</option>
          </select>
          <button type="button" onClick={() => setRevision((r) => r + 1)} aria-label="Reset preview" title="Reset preview"
            className="rounded-md p-2 text-muted-foreground hover:bg-fg/5 focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"><RotateCcw className="size-3.5" /></button>
          <button type="button" onClick={toggleTheme} aria-label={`Switch to ${dark ? 'light' : 'dark'} theme`} title={`Switch to ${dark ? 'light' : 'dark'} theme`}
            className="rounded-md p-2 text-muted-foreground hover:bg-fg/5 focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring">{dark ? <Sun className="size-3.5" /> : <Moon className="size-3.5" />}</button>
        </div>
      </header>
      <div className="min-h-0 flex-1 overflow-y-auto"><div className="mx-auto w-full max-w-2xl px-5 py-6 sm:px-6"><ComputerSection key={revision} preview /></div></div>
    </main>
  </div>;
}

createRoot(document.getElementById('root')!).render(<Preview />);
