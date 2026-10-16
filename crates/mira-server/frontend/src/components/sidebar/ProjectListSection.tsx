import { useLayoutEffect, useRef, type ReactNode } from 'react';
import { cn } from '@/lib/utils';

/** The header sticks on both surfaces. On native vibrancy, clip rows behind
 * it instead of painting an opaque panel over the window's material. */
export function ProjectListSection({ transparent, header, children }: {
  transparent: boolean;
  header: ReactNode;
  children: ReactNode;
}) {
  const section = useRef<HTMLDivElement>(null);
  const heading = useRef<HTMLDivElement>(null);
  const rows = useRef<HTMLDivElement>(null);
  const measure = useRef<(() => void) | null>(null);

  useLayoutEffect(() => {
    if (!transparent) return;
    const container = section.current?.closest<HTMLElement>('[data-sidebar-scroll]');
    const title = heading.current;
    const list = rows.current;
    if (!container || !title || !list) return;
    const update = () => {
      const overlap = Math.max(0, title.getBoundingClientRect().bottom - list.getBoundingClientRect().top);
      list.style.setProperty('--sidebar-projects-clip', `${overlap}px`);
    };
    measure.current = update;
    container.addEventListener('scroll', update, { passive: true });
    const observer = new ResizeObserver(update);
    // Recents can expand above Projects; selection mode can resize its header.
    for (const node of [container, title, ...container.children]) observer.observe(node);
    update();
    return () => {
      measure.current = null;
      container.removeEventListener('scroll', update);
      observer.disconnect();
    };
  }, [transparent]);
  // Data, filters and collapse changes can move the section without a scroll.
  useLayoutEffect(() => { measure.current?.(); });

  return <div ref={section} className="mt-4 flex flex-col gap-1 px-0.5">
    <div ref={heading} data-sidebar-projects-header className={cn(
      'sticky top-0 z-20 -mx-0.5 px-0.5 pb-1',
      transparent ? 'bg-transparent' : 'bg-panel',
    )}>{header}</div>
    <div ref={rows} data-sidebar-projects-rows className="flex flex-col gap-1" style={
      transparent ? { clipPath: 'inset(var(--sidebar-projects-clip, 0px) 0px 0px)' } : undefined
    }>{children}</div>
  </div>;
}
