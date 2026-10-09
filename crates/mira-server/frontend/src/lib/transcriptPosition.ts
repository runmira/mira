export type Anchor = { key: string; offset: number; bottom: boolean };

export const positions = new Map<string, Anchor>();

export function savedPosition(identity: string): Anchor | undefined {
  try {
    return (
      positions.get(identity) ??
      JSON.parse(sessionStorage.getItem(`mira:scroll:${identity}`) ?? 'null') ??
      undefined
    );
  } catch {
    return undefined;
  }
}

export function hasTranscriptPosition(identity: string): boolean {
  return !!savedPosition(identity) && !savedPosition(identity)?.bottom;
}
