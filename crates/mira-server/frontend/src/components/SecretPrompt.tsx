import { useState } from 'react';
import { KeyRound } from 'lucide-react';
import { Button } from './ui/button';
import { Input } from './ui/input';

/** An agent's request for a secret (an API key, a token). */
export type SecretRequest = {
  promptId: string;
  name: string;
  reason: string;
  /** A project .env file the value will also be written to. */
  dotenv?: string | null;
};

/**
 * A private input for a secret an agent asked for. The value goes to a file
 * only the user can read; the agent gets the path, never the value, and it
 * never enters the transcript.
 */
export function SecretPrompt({
  request,
  onReply,
}: {
  request: SecretRequest;
  onReply: (promptId: string, value: string | null) => void;
}) {
  const [value, setValue] = useState('');
  const submit = () => {
    if (value) onReply(request.promptId, value);
  };
  return (
    <div className="space-y-2 px-3 pb-3 text-[12.5px]">
      <p className="text-muted-foreground">{request.reason}</p>
      <form
        className="flex items-center gap-2"
        onSubmit={(e) => {
          e.preventDefault();
          submit();
        }}
      >
        <KeyRound className="size-3.5 shrink-0 text-muted-foreground" aria-hidden />
        <Input
          type="password"
          autoComplete="off"
          spellCheck={false}
          aria-label={request.name}
          placeholder={request.name}
          value={value}
          onChange={(e) => setValue(e.target.value)}
          className="h-8 font-mono text-[12px]"
        />
        <Button type="submit" size="sm" disabled={!value}>
          Save
        </Button>
        <Button type="button" size="sm" variant="ghost" onClick={() => onReply(request.promptId, null)}>
          Decline
        </Button>
      </form>
      <p className="text-[11px] text-muted-foreground/80">
        Stored privately on this machine. The agent gets a file path to use in commands, not the value
        {request.dotenv ? `, and it will also be written to ${request.dotenv}` : ''}.
      </p>
    </div>
  );
}
