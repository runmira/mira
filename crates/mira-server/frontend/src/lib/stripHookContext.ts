/** Drop the blocks added to a prompt for the model: prompt hooks'
 *  `<hook-context>` and the turn's `<memory-context>`.
 *
 *  Generated context blocks follow a specific pattern: they are preceded by
 *  `\n\n` and the opening tag is followed by `\n`. This distinguishes them
 *  from literal user-authored markers (e.g., a user typing "Check <hook-context>
 *  usage") which should remain visible in the transcript.
 */
export function stripHookContext(content: string | null | undefined): string | null | undefined {
  // Generated blocks are always: \n\n<tag>\n...content...\n</tag>
  // Look for the separator pattern to distinguish from user-authored markers.
  const cuts = ['\n\n<hook-context>\n', '\n\n<memory-context>\n']
    .map((marker) => content?.indexOf(marker) ?? -1)
    .filter((i) => i >= 0);
  return cuts.length ? content!.slice(0, Math.min(...cuts)).trimEnd() : content;
}
