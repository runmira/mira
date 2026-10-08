import { PREF_KEYS, useBoolPref } from '../../lib/prefs';
import { AnimatePresence, motion, useReducedMotion } from 'framer-motion';
import { cn } from '@/lib/utils';

/** Shared open/close animation for every collapsible block in the UI.
 *
 * Mirrors the pattern ContextPanel established: the panel grows from
 * height 0 to auto while fading in (160 ms), and collapses back in
 * 120 ms. `overflow-hidden` is required on the motion element so the
 * children clip while the height is animating. Any vertical padding /
 * margin / indent must live on an inner wrapper or the collapse will
 * leave a visible strip — hence `className` is appended, not replaced.
 */
export function Collapse({
  open,
  children,
  className,
}: {
  open: boolean;
  children: React.ReactNode;
  className?: string;
}) {
  const systemReducedMotion = useReducedMotion();
  const [preferReducedMotion] = useBoolPref(PREF_KEYS.reduceMotion, false);
  const reduceMotion = systemReducedMotion || preferReducedMotion;
  return (
    <AnimatePresence initial={false}>
      {open && (
        <motion.div
          initial={{ height: 0, opacity: 0 }}
          animate={{ height: 'auto', opacity: 1, transition: { duration: reduceMotion ? 0 : 0.24, ease: [0.22, 1, 0.36, 1] } }}
          exit={{ height: 0, opacity: 0, transition: { duration: reduceMotion ? 0 : 0.18, ease: 'easeInOut' } }}
          className={cn('overflow-hidden', className)}
        >
          {children}
        </motion.div>
      )}
    </AnimatePresence>
  );
}
