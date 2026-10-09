import {
  ACCENT_SWATCH,
  ACCENTS,
  setAccent,
  setUiScale,
  useAccent,
  useUiScale,
  type UiScale,
} from '@/lib/appearance';
import { useDiffOptions } from '@/lib/diffPrefs';
import { applyReduceMotion, PREF_KEYS, useBoolPref, useStringPref } from '@/lib/prefs';
import { useTheme, useThemePref, type ThemePref } from '@/lib/theme';
import { cn } from '@/lib/utils';
import { parseDiffFromFile } from '@pierre/diffs';
import { Check, GitCompare, Palette, Sparkle } from 'lucide-react';
import { StyledDiffCodeView } from '../diffs/StyledDiffCodeView';
import { TRow, TSection, TSwitch } from './SettingsFields';
/* ---------- shared bits: settings layout ---------- */

/**
 * Settings design philosophy, in Mira tokens:
 * - Sections are headed by a quiet muted label row — never a big card
 *   title. Explanatory copy lives on individual rows, not the header.
 * - Rows are title + one-line description on the left, a compact control
 *   flush right. They stack on narrow screens, snap to a two-column grid
 *   past `sm:`.
 * - Rows live in a flat grouped card with hairline dividers; controls
 *   share sizing (inputs/selects h-8, text-[13px]) so every row baselines.
 */
export const THEME_CHOICES: { value: ThemePref; label: string; hint: string }[] = [
  { value: 'system', label: 'System', hint: 'Match macOS / your OS' },
  { value: 'light', label: 'Light', hint: 'Bright and paper-like' },
  { value: 'dark', label: 'Dark', hint: 'The classic Mira look' },
];

/** A tiny drawing of the app in one theme, for the theme cards. */
export function ThemeSwatch({ theme }: { theme: 'light' | 'dark' }) {
  const dark = theme === 'dark';
  return (
    <span
      className={cn('flex h-full w-full overflow-hidden', dark ? 'bg-[#202020]' : 'bg-[#eef0f3]')}
    >
      <span className="flex w-[30%] flex-col gap-1 p-1.5">
        <span className={cn('h-1.5 w-3/4 rounded-full', dark ? 'bg-white/25' : 'bg-black/15')} />
        <span className={cn('h-1.5 w-1/2 rounded-full', dark ? 'bg-white/15' : 'bg-black/10')} />
        <span className={cn('h-1.5 w-2/3 rounded-full', dark ? 'bg-white/15' : 'bg-black/10')} />
      </span>
      <span
        className={cn(
          'my-1.5 mr-1.5 flex flex-1 flex-col gap-1 rounded-[5px] border p-1.5',
          dark ? 'border-white/10 bg-[#0c0c0c]' : 'border-black/10 bg-white',
        )}
      >
        <span className={cn('h-1.5 w-4/5 rounded-full', dark ? 'bg-white/30' : 'bg-black/20')} />
        <span className={cn('h-1.5 w-3/5 rounded-full', dark ? 'bg-white/15' : 'bg-black/10')} />
        <span className="mt-auto h-2.5 w-full rounded-[3px] bg-[#7aa2f7]/70" />
      </span>
    </span>
  );
}

/** A short made-up change, for previewing diff settings. */
export const SAMPLE_DIFF = (() => {
  const before = [
    "import { fetchUser } from '../api';",
    '',
    'export async function greet(id: string) {',
    '  const user = await fetchUser(id);',
    "  return 'Hello, ' + user.name + '!';",
    '}',
    '',
  ].join('\n');
  const after = [
    "import { fetchUser } from '../api';",
    '',
    'export async function greet(id: string, locale = "en") {',
    '  const user = await fetchUser(id);',
    '  if (!user) return null;',
    '  return `Hello, ${user.displayName}!`;',
    '}',
    '',
  ].join('\n');
  try {
    return parseDiffFromFile(
      { name: 'greet.ts', contents: before },
      { name: 'greet.ts', contents: after },
      { context: Infinity },
    );
  } catch {
    return null;
  }
})();

export function Segmented<T extends string>({
  value,
  onChange,
  options,
}: {
  value: T;
  onChange: (v: T) => void;
  options: { value: T; label: string }[];
}) {
  return (
    <div
      role="radiogroup"
      className="inline-flex rounded-lg border border-border/70 bg-fg/[0.03] p-0.5"
    >
      {options.map((o) => (
        <button
          key={o.value}
          type="button"
          role="radio"
          aria-checked={value === o.value}
          onClick={() => onChange(o.value)}
          className={cn(
            'rounded-md px-2.5 py-1 text-[12px] font-medium transition-colors',
            value === o.value
              ? 'bg-background text-foreground shadow-sm ring-1 ring-border/70'
              : 'text-muted-foreground hover:text-foreground',
          )}
        >
          {o.label}
        </button>
      ))}
    </div>
  );
}

export function DiffAppearance() {
  const [layout, setLayout] = useStringPref(PREF_KEYS.diffLayout, 'unified');
  const [wrap, setWrap] = useBoolPref(PREF_KEYS.diffWrap, false);
  const [inline, setInline] = useStringPref(PREF_KEYS.diffInline, 'word');
  const [markers, setMarkers] = useStringPref(PREF_KEYS.diffMarkers, 'bars');
  const [lineNumbers, setLineNumbers] = useBoolPref(PREF_KEYS.diffLineNumbers, true);
  const [tint, setTint] = useBoolPref(PREF_KEYS.diffTint, true);
  const options = useDiffOptions();
  return (
    <TSection
      icon={<GitCompare className="size-3.5" />}
      title="Diffs"
      description="How changes look in the file viewer and the review drawer. Stored in this browser only."
    >
      {SAMPLE_DIFF && (
        <div className="p-3">
          <div className="h-[178px] overflow-hidden rounded-lg border border-border/70">
            <StyledDiffCodeView
              className="h-full min-h-0 overflow-auto"
              items={[{ id: 'preview', type: 'diff', fileDiff: SAMPLE_DIFF, collapsed: false }]}
              options={options}
              renderHeaderFilenameSuffix={() => null}
              renderHeaderPrefix={() => null}
            />
          </div>
        </div>
      )}
      <TRow
        title="Layout"
        description="Unified stacks old and new lines; side by side puts them in two columns."
        control={
          <Segmented
            value={layout === 'split' ? 'split' : 'unified'}
            onChange={setLayout}
            options={[
              { value: 'unified', label: 'Unified' },
              { value: 'split', label: 'Side by side' },
            ]}
          />
        }
      />
      <TRow
        title="Highlight changes within a line"
        description="Marks the exact words or characters that changed, not just the line."
        control={
          <Segmented
            value={
              (['word', 'char', 'none'] as const).includes(inline as 'word')
                ? (inline as 'word' | 'char' | 'none')
                : 'word'
            }
            onChange={setInline}
            options={[
              { value: 'word', label: 'Words' },
              { value: 'char', label: 'Characters' },
              { value: 'none', label: 'Off' },
            ]}
          />
        }
      />
      <TRow
        title="Change markers"
        description="What sits beside a changed line: a colored bar, or + and − signs."
        control={
          <Segmented
            value={
              (['bars', 'classic', 'none'] as const).includes(markers as 'bars')
                ? (markers as 'bars' | 'classic' | 'none')
                : 'bars'
            }
            onChange={setMarkers}
            options={[
              { value: 'bars', label: 'Bars' },
              { value: 'classic', label: '+ / −' },
              { value: 'none', label: 'None' },
            ]}
          />
        }
      />
      <TRow
        title="Wrap long lines"
        description="Off: long lines scroll sideways."
        control={<TSwitch checked={wrap} onChange={setWrap} label="Wrap long lines" />}
      />
      <TRow
        title="Line numbers"
        control={<TSwitch checked={lineNumbers} onChange={setLineNumbers} label="Line numbers" />}
      />
      <TRow
        title="Tint changed lines"
        description="Green and red backgrounds on added and removed lines."
        control={<TSwitch checked={tint} onChange={setTint} label="Tint changed lines" />}
      />
    </TSection>
  );
}

export function AppearanceSection() {
  const [pref, setPref] = useThemePref();
  const theme = useTheme();
  const accent = useAccent();
  const scale = useUiScale();
  const [reduceMotion, setReduceMotionState] = useBoolPref(PREF_KEYS.reduceMotion, false);
  return (
    <div className="flex flex-col gap-2.5">
      <TSection
        icon={<Palette className="size-3.5" />}
        title="Theme"
        description="Light, dark or your system's, plus accent and size. Stored in this browser only."
      >
        <div className="grid grid-cols-1 gap-3 p-4 sm:grid-cols-3">
          {THEME_CHOICES.map((c) => {
            const on = pref === c.value;
            return (
              <button
                key={c.value}
                type="button"
                role="radio"
                aria-checked={on}
                onClick={() => setPref(c.value)}
                className={cn(
                  'group flex flex-col gap-2.5 rounded-xl border p-2 text-left transition-[border-color,box-shadow]',
                  on
                    ? 'border-mira-blue shadow-[0_0_0_3px_rgb(var(--mira-blue)/0.18)]'
                    : 'border-border hover:border-fg/25',
                )}
              >
                <span className="relative block aspect-[16/10] overflow-hidden rounded-lg border border-border/60">
                  {c.value === 'system' ? (
                    <span className="absolute inset-0 flex">
                      <span className="relative w-1/2 overflow-hidden">
                        <span className="absolute inset-y-0 left-0 w-[200%]">
                          <ThemeSwatch theme="light" />
                        </span>
                      </span>
                      <span className="relative w-1/2 overflow-hidden">
                        <span className="absolute inset-y-0 right-0 w-[200%]">
                          <ThemeSwatch theme="dark" />
                        </span>
                      </span>
                    </span>
                  ) : (
                    <ThemeSwatch theme={c.value} />
                  )}
                </span>
                <span className="flex items-center gap-2 px-1 pb-0.5">
                  <span
                    className={cn(
                      'grid size-4 shrink-0 place-items-center rounded-full border transition-colors',
                      on ? 'border-mira-blue bg-mira-blue' : 'border-fg/25',
                    )}
                  >
                    {on && <span className="size-1.5 rounded-full bg-mira-on-accent" />}
                  </span>
                  <span className="min-w-0">
                    <span className="block text-[13px] font-medium text-foreground">{c.label}</span>
                    <span className="block text-[11.5px] text-muted-foreground">{c.hint}</span>
                  </span>
                </span>
              </button>
            );
          })}
        </div>
        <TRow
          title="Accent color"
          description="Buttons, switches, links and selection."
          control={
            <div role="radiogroup" aria-label="Accent color" className="flex items-center gap-1.5">
              {ACCENTS.map((a) => {
                const on = accent === a;
                const color = ACCENT_SWATCH[a][theme === 'light' ? 1 : 0];
                return (
                  <button
                    key={a}
                    type="button"
                    role="radio"
                    aria-checked={on}
                    aria-label={a}
                    title={a[0].toUpperCase() + a.slice(1)}
                    onClick={() => setAccent(a)}
                    className={cn(
                      'grid size-6 place-items-center rounded-full transition-transform hover:scale-110',
                      on && 'ring-2 ring-offset-2 ring-offset-background',
                    )}
                    style={{
                      backgroundColor: color,
                      ...(on ? { ['--tw-ring-color' as string]: color } : {}),
                    }}
                  >
                    {on && <Check className="size-3.5 text-white drop-shadow" strokeWidth={3} />}
                  </button>
                );
              })}
            </div>
          }
        />
        <TRow
          title="Interface size"
          description="Scales text and spacing across the whole app."
          control={
            <Segmented
              value={String(scale) as '0.9' | '1' | '1.1' | '1.2'}
              onChange={(v) => setUiScale(Number(v) as UiScale)}
              options={[
                { value: '0.9', label: 'Smaller' },
                { value: '1', label: 'Default' },
                { value: '1.1', label: 'Larger' },
                { value: '1.2', label: 'Largest' },
              ]}
            />
          }
        />
      </TSection>

      <DiffAppearance />

      <TSection icon={<Sparkle className="size-3.5" />} title="Motion">
        <TRow
          title="Reduce motion"
          description="Turn off shimmer, pulses and transitions across the whole app."
          control={
            <TSwitch
              checked={reduceMotion}
              onChange={(v) => {
                setReduceMotionState(v);
                applyReduceMotion();
              }}
              label="Reduce motion"
            />
          }
        />
      </TSection>
    </div>
  );
}
