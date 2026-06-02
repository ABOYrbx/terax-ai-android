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

  // On Android, prefer the app's Termux-style home directory. `android_init_home`
  // resolves and creates `<appDataDir>/home` in one round-trip; `android_home_dir`
  // returns it after a prior `init`. The fallback to Tauri's generic `homeDir`
  // covers the brief window before the Rust `init` has run on a cold start.
  if (isAndroid()) {
    try {
      const h = await invoke<string | null>("android_home_dir");
      if (h) {
        cached = h.replace(/\\/g, "/");
        return;
      }
    } catch {
      // ignore — fall through to the next source
    }
    try {
      const h = await homeDir();
      if (h) {
        cached = h.replace(/\\/g, "/");
        return;
      }
    } catch {
      // ignore failures and fall through to undefined
    }
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
