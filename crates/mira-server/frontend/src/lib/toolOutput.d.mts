export type AnsiSegment = {
  text: string;
  style: {
    color?: string;
    backgroundColor?: string;
    fontWeight?: string;
    fontStyle?: string;
    textDecoration?: string;
    opacity?: string;
    filter?: string;
  };
};

export function ansiToSegments(input: string): AnsiSegment[];
export function parseBashOutput(content: string): {
  output: string;
  exitCode: number | null;
  timedOut: boolean;
};
export function foldOutputLines(
  content: string,
  limit?: number,
  context?: number,
): { lines: string[]; hidden: number };
