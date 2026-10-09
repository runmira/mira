import { ChevronDown, ChevronRight } from 'lucide-react';
/* ------------------------------------------------------------------ */
/* Animation variants                                                  */
/* ------------------------------------------------------------------ */

export const itemVariants = {
  hidden: { opacity: 0, x: -4 },
  visible: (i: number) => ({
    opacity: 1,
    x: 0,
    transition: { duration: 0.16, delay: i * 0.035 },
  }),
};

/* ------------------------------------------------------------------ */
/* Section header                                                      */
/* ------------------------------------------------------------------ */

export function SectionHeader({
  label,
  right,
  open,
  onToggle,
}: {
  label: string;
  right?: React.ReactNode;
  open: boolean;
  onToggle: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onToggle}
      className="flex w-full items-center justify-between py-1.5 text-left group"
    >
      <span className="text-[11px] font-bold uppercase tracking-widest text-fg/40 group-hover:text-fg/60 transition-colors">
        {label}
      </span>
      <div className="flex items-center gap-1.5">
        {right}
        {open ? (
          <ChevronDown className="size-3.5 text-fg/25" />
        ) : (
          <ChevronRight className="size-3.5 text-fg/25" />
        )}
      </div>
    </button>
  );
}
