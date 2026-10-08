/**
 * How diffs look (Settings → Appearance → Diffs): one place that turns the
 * preferences into the diff renderer's options, so the file viewer, the
 * review drawer and the settings preview always agree.
 */
import { useMemo } from 'react';
import { PREF_KEYS, useBoolPref, useStringPref } from './prefs';
import { useIsPhone } from './mobile';

export type DiffInline = 'word' | 'char' | 'none';
export type DiffMarkers = 'bars' | 'classic' | 'none';

export function useDiffOptions() {
  const [layout] = useStringPref(PREF_KEYS.diffLayout, 'unified');
  const [wrap] = useBoolPref(PREF_KEYS.diffWrap, false);
  const [inline] = useStringPref(PREF_KEYS.diffInline, 'word');
  const [markers] = useStringPref(PREF_KEYS.diffMarkers, 'bars');
  const [lineNumbers] = useBoolPref(PREF_KEYS.diffLineNumbers, true);
  const [tint] = useBoolPref(PREF_KEYS.diffTint, true);
  // Side by side leaves ~20 characters a side on a phone.
  const phone = useIsPhone();
  return useMemo(
    () => ({
      diffStyle: layout === 'split' && !phone ? ('split' as const) : ('unified' as const),
      overflow: wrap ? ('wrap' as const) : ('scroll' as const),
      // `word-alt` is the renderer's word mode that also joins nearby
      // changed words, which reads better than plain `word`.
      lineDiffType: inline === 'char' ? ('char' as const) : inline === 'none' ? ('none' as const) : ('word-alt' as const),
      diffIndicators: (markers === 'classic' || markers === 'none' ? markers : 'bars') as DiffMarkers,
      disableLineNumbers: !lineNumbers,
      disableBackground: !tint,
    }),
    [layout, phone, wrap, inline, markers, lineNumbers, tint],
  );
}
