import { useEffect } from 'react';

/** Full-screen image preview; click anywhere or press Esc to close. */
export function ImageLightbox({ src, onClose }: { src: string | null; onClose: () => void }) {
  useEffect(() => {
    if (!src) return;
    const onKey = (e: KeyboardEvent) => { if (e.key === 'Escape') onClose(); };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [src, onClose]);
  if (!src) return null;
  return (
    <div
      className="fixed inset-0 z-[60] flex animate-fade-in cursor-zoom-out items-center justify-center bg-black/80 p-8"
      onClick={onClose}
    >
      <img src={src} alt="" className="max-h-full max-w-full rounded-lg shadow-2xl" />
    </div>
  );
}
