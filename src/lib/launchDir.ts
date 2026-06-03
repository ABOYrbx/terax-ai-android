import { invoke } from "@tauri-apps/api/core";
import { platform } from "@tauri-apps/plugin-os";

let cached: string | undefined;

export async function initLaunchDir(): Promise<void> {
  const dir =
    (await invoke<string | null>("get_launch_dir").catch(() => null)) ??
    (await invoke<string>("workspace_current_dir").catch(() => null));

  if (dir) {
    cached = dir.replace(/\\/g, "/");
    return;
  }

  // On Android, start at "/" (root) so the breadcrumb doesn't show a
  // long app-private path. The terminal shell starts at "/" and the user
  // can `cd` into any accessible directory from there.
  if (isAndroid()) {
    cached = "/";
    return;
  }

  cached = undefined;
}

export function getLaunchDir(): string | undefined {
  return cached;
}

let androidPltCache: boolean | null = null;
function isAndroid(): boolean {
  if (androidPltCache !== null) return androidPltCache;
  try {
    androidPltCache = platform() === "android";
  } catch {
    androidPltCache = false;
  }
  return androidPltCache;
}
