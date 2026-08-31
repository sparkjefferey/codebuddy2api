// Tauri invoke wrappers — typed bridges to Rust commands
import { invoke } from "@tauri-apps/api/core";

export interface CredentialStatus {
  configured: boolean;
  uid?: string;
  nickname?: string;
  domain?: string;
  expires_at?: number;
}

export async function getConfig(): Promise<Record<string, unknown>> {
  return invoke("get_config");
}

export async function getCredentialStatus(): Promise<CredentialStatus> {
  return invoke("get_credential_status");
}

export async function importCredential(jsonStr: string): Promise<string> {
  return invoke("import_credential", { jsonStr });
}

export async function clearCredential(): Promise<string> {
  return invoke("clear_credential");
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