/** Whether a `ready` frame should bring the main pane back to the chat.
 *
 *  Only when it opened a different chat than the one before: then a fresh
 *  transcript should show, wherever the user was parked. The first ready
 *  after a page load and a reconnect to the same chat leave the view alone,
 *  so a reload keeps the route the URL names (Plugins, a Work item…). */
export function readyShowsChat(previousSession: string | null | undefined, readySession: string): boolean {
  return !!previousSession && previousSession !== readySession;
}
