import { createContext, useContext, useRef, useState, type Dispatch, type SetStateAction } from 'react';
export const TranscriptSessionContext = createContext('');
export const TranscriptRowContext = createContext('');
const states = new Map<string, boolean>();
/** State belongs to a transcript item, rather than its temporary virtual DOM mount. */
export function useTranscriptDisclosure(part: string, initial = false): [boolean, Dispatch<SetStateAction<boolean>>] {
  const row = useContext(TranscriptRowContext);
  const key = useRef(`${row}:${part}`).current;
  const [value, update] = useState(() => row ? states.get(key) ?? initial : initial);
  const setValue: Dispatch<SetStateAction<boolean>> = next => update(previous => {
    const resolved = typeof next === 'function' ? next(previous) : next;
    if (row) {
      states.delete(key); states.set(key, resolved);
      if (states.size > 10000) states.delete(states.keys().next().value!);
    }
    return resolved;
  });
  return [value, setValue];
}
