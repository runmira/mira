import { jsonOrThrow, post } from './request';

export type CodexAccountView = {
  connection_id: string;
  phase: 'signed_out' | 'pending' | 'authenticated' | 'failed' | 'cancelled' | 'unsupported';
  account: {kind:string;email?:string|null;plan?:string|null} | null;
  requires_openai_auth: boolean | null;
  requirements: Record<string,unknown> | null;
  requirements_supported: boolean;
  login_id: string | null;
  auth_url: string | null;
  user_code: string | null;
  error: string | null;
  revision: number;
};
export async function getCodexAccount(instance:string,refresh=false): Promise<CodexAccountView> {
  return jsonOrThrow(await fetch(`/api/engines/${encodeURIComponent(instance)}/account${refresh?'?refresh=1':''}`, {cache:'no-store'}),'Check Codex account');
}
export function startCodexLogin(instance:string,flow:'browser'|'device'): Promise<CodexAccountView> {
  return post(`/api/engines/${encodeURIComponent(instance)}/account/login`,{flow},'Start Codex sign-in');
}
export async function cancelCodexLogin(instance:string,connectionId:string,loginId:string): Promise<CodexAccountView> {
  return jsonOrThrow(await fetch(`/api/engines/${encodeURIComponent(instance)}/account/login`,{method:'DELETE',cache:'no-store',headers:{'content-type':'application/json'},body:JSON.stringify({connection_id:connectionId,login_id:loginId})}),'Cancel Codex sign-in');
}
