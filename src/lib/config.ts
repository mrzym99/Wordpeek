import { invoke } from "@tauri-apps/api/core";
import type { AppConfig } from "../types";

/** 以后端当前配置为基底，只改指定字段后整体写回——避免把表单里未保存的编辑一并落盘 */
export async function patchConfig(patch: Partial<AppConfig>) {
  const cfg = await invoke<AppConfig>("get_config");
  await invoke("save_config", { config: { ...cfg, ...patch } });
}
