import { clsx, type ClassValue } from 'clsx';
import { twMerge } from 'tailwind-merge';

/** shadcn's canonical class-name helper: merges Tailwind class strings and
 *  resolves conflicts so `cn('p-2', condition && 'p-4')` yields `p-4`. */
export function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs));
}

/** Last path segment — what people call a project. Trailing slashes are
 *  ignored so `/a/b/` and `/a/b` name the same thing. */
export function basename(path: string): string {
  const trimmed = path.replace(/\/+$/, '');
  const i = trimmed.lastIndexOf('/');
  return i >= 0 ? trimmed.slice(i + 1) || path : trimmed;
}

/** The segment above the basename, or `''` at the filesystem root. Two
 *  projects can share a name, so UIs that show both say the parent. */
export function parentName(path: string): string {
  const parts = path.replace(/\/+$/, '').split('/');
  return parts.length >= 2 ? parts[parts.length - 2] : '';
}
