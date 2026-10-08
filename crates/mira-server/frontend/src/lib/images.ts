import type { ImageAttachment } from '../types';

/** Where to load an image from: its server URL for large images in loaded
 *  history (fetched on demand, cached), else an inline data URL. */
export function imageSrc(img: ImageAttachment): string {
  return img.url && !img.data ? img.url : `data:${img.media_type};base64,${img.data}`;
}
