/** One left gutter for every row in the agent stream.
 *
 *  The transcript used to set this padding per component — `px-2` on a
 *  grouped tool row, `px-1` on a single tool row, nothing on an assistant
 *  message or the "Worked for" chip. That put the first glyph of each row
 *  at 0 / 4 / 8px, so the column read as a ragged edge rather than a line.
 *
 *  Every stream row now takes its padding from here, so a message, a tool
 *  call, a status line and the duration chip all start at the same x. The
 *  value is a Tailwind class (not a number) so it composes through `cn`
 *  and can be overridden per call site — e.g. a row that needs to bleed its
 *  hover background to the edge can pass `px-0` and win the merge. */
export const STREAM_ROW_GUTTER = 'px-2';

/** Rows whose hover target should span the full column width instead of
 *  stopping at the gutter. Use only where the background replaces the
 *  gutter's whitespace visually (e.g. a full-bleed divider), never on a
 *  row that carries text. */
export const STREAM_ROW_BLEED = 'px-0';