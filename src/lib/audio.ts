import { invoke } from "@tauri-apps/api/core";

export type DeviceState =
  | "active"
  | "unplugged"
  | "disabled"
  | "not_present"
  | "unknown";

export interface AudioDevice {
  id: string;
  name: string;
  state: DeviceState;
  is_default_console: boolean;
  is_default_multimedia: boolean;
  is_default_communications: boolean;
}

export type Role = "console" | "multimedia" | "communications";

export function listDevices(): Promise<AudioDevice[]> {
  return invoke<AudioDevice[]>("list_devices");
}

export function setDefault(deviceId: string, roles: Role[]): Promise<void> {
  return invoke("set_default", { deviceId, roles });
}
