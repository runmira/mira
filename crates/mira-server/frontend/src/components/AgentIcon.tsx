import { useEffect, useState } from 'react';
import { agentFaviconUrls, agentMonogram } from '../lib/acpAgents';
import { modelIconUrls, modelVendor, needsLightTile, prettyVendor, providerIconUrls } from '../lib/models';
import { cn } from '@/lib/utils';

type IconSize = 'xs' | 'sm' | 'md';

const BOX: Record<IconSize, string> = {
  // 16px: the height of one 13–14px line of text, so a row's icon and its
  // label share a centre line instead of the icon pushing the row taller.
  xs: 'size-4 rounded-[4px]',
  sm: 'size-5 rounded-md',
  md: 'size-9 rounded-[10px]',
};
const IMG: Record<IconSize, string> = {
  xs: 'size-[14px] rounded-[3px]',
  sm: 'size-[14px] rounded-[2px]',
  md: 'size-5 rounded-[3px]',
};
const MONO: Record<IconSize, string> = {
  xs: 'text-[7.5px]',
  sm: 'text-[8px]',
  md: 'text-[11px]',
};

/**
 * One brand mark, everywhere: agents, providers and models share the same
 * fallback chain (vendor icon → Google's favicon service → monogram) and
 * the same sizes, so a session row, the picker rail and Settings never
 * disagree about what something looks like.
 */
export function BrandIcon({
  urls,
  name,
  size = 'md',
  tile = true,
  className,
}: {
  urls: string[];
  name: string;
  size?: IconSize;
  /**
   * Light tile behind the mark. Kept where favicons need a light ground
   * (near-white glyphs vanish on dark). Off in dense dark surfaces like
   * the sidebar, where a white square shouts — the mark floats directly,
   * and the monogram switches to a light tone to stay legible.
   */
  tile?: boolean;
  className?: string;
}) {
  const [idx, setIdx] = useState(0);
  const key = urls.join('|');
  // A different brand in the same slot starts its chain from the top.
  useEffect(() => setIdx(0), [key]);
  const src = urls[idx];
  return (
    <span
      aria-hidden
      className={cn(
        'inline-grid shrink-0 place-items-center overflow-hidden',
        tile
          ? 'border border-border/70 bg-white shadow-[0_1px_2px_rgba(0,0,0,0.18)]'
          : 'border-transparent bg-transparent',
        BOX[size],
        className,
      )}
    >
      {src ? (
        <img
          src={src}
          alt=""
          draggable={false}
          className={cn('block object-contain', IMG[size])}
          onError={() => setIdx((i) => i + 1)}
        />
      ) : (
        <span
          className={cn(
            'font-bold leading-none tracking-tight',
            tile ? 'text-neutral-700' : 'text-foreground/75',
            MONO[size],
          )}
        >
          {agentMonogram(name)}
        </span>
      )}
    </span>
  );
}

/** An external agent's mark. */
export function AgentIcon({
  kind,
  name,
  size = 'md',
  tile = true,
  className,
}: {
  kind: string;
  name: string;
  size?: IconSize;
  tile?: boolean;
  className?: string;
}) {
  return <BrandIcon urls={agentFaviconUrls(kind)} name={name} size={size} tile={tile} className={className} />;
}

/** A model's vendor mark (Anthropic for `claude-…`, OpenAI for `gpt-…`). */
export function ModelIcon({
  model,
  size = 'sm',
  tile = false,
  className,
}: {
  model: string | null | undefined;
  size?: IconSize;
  tile?: boolean;
  className?: string;
}) {
  // An unrecognised model has no brand to show; a neutral dot reads as
  // "a model" where a two-letter monogram of "Other" would read as noise.
  if (modelVendor(model) === 'other') {
    return (
      <span aria-hidden className={cn('inline-grid shrink-0 place-items-center', BOX[size], className)}>
        <span className="size-2 rounded-full bg-muted-foreground/45" />
      </span>
    );
  }
  return (
    <BrandIcon
      urls={modelIconUrls(model)}
      name={prettyVendor(modelVendor(model))}
      size={size}
      tile={tile}
      className={className}
    />
  );
}

/** A provider instance's mark (OpenRouter, Anthropic, …), falling back to
 *  the vendor of the model it serves. */
export function ProviderIcon({
  instance,
  name,
  model,
  size = 'sm',
  tile = false,
  className,
}: {
  instance: string | null | undefined;
  name: string;
  model?: string | null;
  size?: IconSize;
  tile?: boolean;
  className?: string;
}) {
  return (
    <BrandIcon
      urls={providerIconUrls(instance, model)}
      name={name}
      size={size}
      tile={tile || needsLightTile(instance)}
      className={className}
    />
  );
}
