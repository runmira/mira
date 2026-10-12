/** Composer attachment strip: the thumbnail + hover card pair.
 *
 *  Exists to check one thing — that the capture's window name and OCR text
 *  are still reachable now that they no longer print under the thumbnail.
 *  Mirrors the JSX in `Composer.tsx` (same components, same props) rather
 *  than booting the composer, whose image state is hydrated from IndexedDB
 *  and so can't be seeded from a page. Not shipped.
 */
import { createRoot } from 'react-dom/client';
import { ImageAttachmentMeta } from './components/ImageAttachmentDetails';
import { HoverCard } from './components/ui/hover-card';
import './styles.css';

// 1×1 transparent PNG — enough to prove the strip renders, without
// checking a real screenshot into the repo.
const PIXEL =
  'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==';

const image = {
  media_type: 'image/png',
  data: PIXEL,
  source: {
    name: 'screenshot-2026-10-12-at-22.41.07.png',
    window: 'Simulator — iPhone 17 Pro',
    width: 1206,
    height: 2622,
    captured_at: new Date().toISOString(),
    accessible_text:
      'Home screen. Icons: Messages, Calendar, Photos, Camera, Maps, Weather. ' +
      'Dock: Phone, Safari, Messages, Music. Battery 94%.',
  },
};

function Strip() {
  const src = `data:${image.media_type};base64,${image.data}`;
  return (
    <div className="flex flex-wrap gap-2 px-1.5 pt-1">
      <div className="group relative">
        <HoverCard
          side="top"
          trigger={
            <img src={src} alt="" className="size-14 cursor-zoom-in rounded-lg border border-border object-cover" />
          }
        >
          <ImageAttachmentMeta image={image} />
        </HoverCard>
        <button
          type="button"
          aria-label="Remove image"
          className="absolute -right-1.5 -top-1.5 flex size-5 items-center justify-center rounded-full border border-border bg-background text-[11px] leading-none text-muted-foreground"
        >
          ×
        </button>
      </div>
    </div>
  );
}

createRoot(document.getElementById('root')!).render(
  <div className="mx-auto max-w-3xl px-8 pt-12">
    <p className="mb-3 text-xs text-muted-foreground">
      Hover (or tab to) the thumbnail — the window name and captured text
      open upward instead of printing under the image.
    </p>
    {/* Pin to the bottom so the upward popover is exercised against a real
        viewport edge, as it is in the composer. */}
    <div className="mt-96 rounded-xl border border-border/50 p-3">
      <Strip />
    </div>
  </div>,
);