export function isProviderLimitFailure(text: string): boolean {
  return /rate.?limit|usage limit|quota|429/.test(text.toLowerCase());
}
export function isFatalChatWarning(text: string): boolean {
  return ['provider error', 'stream error', 'stream timed out'].some(prefix => text.startsWith(prefix));
}
export function chatFailure(text: string): { title: string; description: string; settings: boolean } {
  const lower = text.toLowerCase();
  if (isProviderLimitFailure(text)) return { title: 'Provider limit reached', description: 'Wait for your provider allowance to become available, or choose another provider before retrying.', settings: true };
  if (/401|403|unauthorized|authentication|api.?key|not logged in|sign.?in/.test(lower)) return { title: 'Check your provider connection', description: 'Your provider or agent could not authenticate. Check its credentials or sign-in before retrying.', settings: true };
  if (/timed out|timeout/.test(lower)) return { title: 'The response timed out', description: 'The provider stopped responding. You can retry this message when the connection is ready.', settings: false };
  if (/stream error|connection|network|fetch|socket|econn/.test(lower)) return { title: 'The response was interrupted', description: 'The connection ended before the response finished. Any partial answer remains above.', settings: false };
  return { title: 'The request could not finish', description: 'Review any work already completed before retrying this message.', settings: true };
}
