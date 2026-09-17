import { invoke } from "@tauri-apps/api/core";
import type { UpdaterAdapter } from "./appUpdate";

export async function desktopUpdaterAdapter(): Promise<UpdaterAdapter> {
  const { check } = await import("@tauri-apps/plugin-updater");
  return {
    check: (options) => check(options),
    relaunch: () => invoke("relaunch_app"),
  };
}
