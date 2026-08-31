// Tauri invoke wrappers — typed bridges to Rust commands
import { invoke } from "@tauri-apps/api/core";

export interface AccountInfo {
  id: string;
  uid: string;
  nickname: string;
  domain: string;
  expires_at: number;
  enabled: boolean;
  consecutive_429?: number;
  /** 冷却截止 epoch 毫秒；0 = 未冷却 */
  cooldown_until?: number;
  last_error?: string | null;
  last_used_ms?: number | null;
}

export interface CredentialStatus {
  configured: boolean;
  accounts: AccountInfo[];
}

export async function getConfig(): Promise<Record<string, unknown>> {
  return invoke("get_config");
}

export async function getCredentialStatus(): Promise<CredentialStatus> {
  return invoke("get_credential_status");
}

/** 导入账号：uid 相同则更新凭据，否则新增。返回 "added" | "updated" */
export async function importCredential(jsonStr: string): Promise<string> {
  return invoke("import_credential", { jsonStr });
}

export async function removeAccount(id: string): Promise<string> {
  return invoke("remove_account", { id });
}

export async function setAccountEnabled(id: string, enabled: boolean): Promise<string> {
  return invoke("set_account_enabled", { id, enabled });
}

export async function getApiKey(): Promise<string> {
  return invoke("get_api_key");
}

export async function toggleDesensitize(enabled: boolean): Promise<string> {
  return invoke("toggle_desensitize", { enabled });
}

export async function buildCcswitchLink(
  endpoint: string,
  name: string,
  apiKey: string,
  model?: string,
): Promise<string> {
  return invoke("build_ccswitch_link", { endpoint, name, apiKey, model });
}

export async function openCcswitchLink(url: string): Promise<boolean> {
  return invoke("open_ccswitch_link", { url });
}

export async function getVersion(): Promise<string> {
  return invoke("get_version");
}

export const GATEWAY = "http://127.0.0.1:9178";
