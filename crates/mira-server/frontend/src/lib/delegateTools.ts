export function isDelegateTaskName(name: string): boolean {
  return name === 'delegate_task' || name.endsWith('__delegate_task');
}
