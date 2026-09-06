import { clsx, type ClassValue } from 'clsx';
import { twMerge } from 'tailwind-merge';

/** shadcn's canonical class-name helper: merges Tailwind class strings and
 *  resolves conflicts so `cn('p-2', condition && 'p-4')` yields `p-4`. */
export function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs));
}
