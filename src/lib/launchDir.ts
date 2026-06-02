import { invoke } from "@tauri-apps/api/core";
import { homeDir } from "@tauri-apps/api/path";
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

  // On Android, prefer the app home directory as a Termux-like workspace.
  try {
    const plt = platform();
    if (plt === "android") {
      const h = await homeDir();
      if (h) {
        cached = h.replace(/\\/g, "/");
        return;
      }
    }
  } catch {
    // ignore failures and fall through to undefined
  }

  cached = undefined;
}

export function getLaunchDir(): string | undefined {
  return cached;
}
