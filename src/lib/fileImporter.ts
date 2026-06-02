import { invoke } from "@tauri-apps/api/core";
import { homeDir } from "@tauri-apps/api/path";
import { platform } from "@tauri-apps/plugin-os";

async function getAppHome(): Promise<string> {
  // On Android, the app's Termux-style home lives at <appDataDir>/home. Using
  // the raw Tauri `homeDir()` would dump imports one level above the user's
  // home (e.g. into the app's files root) — outside the directory the
  // explorer shows as the workspace root.
  if (platform() === "android") {
    const h = await invoke<string | null>("android_home_dir").catch(() => null);
    if (h) return h.replace(/\\/g, "/");
  }
  const h = await homeDir();
  return h.replace(/\\/g, "/");
}

export async function importFileToPath(targetPath: string): Promise<void> {
  return new Promise((resolve, reject) => {
    const input = document.createElement("input");
    input.type = "file";
    input.onchange = async () => {
      const file = input.files?.[0];
      if (!file) {
        reject(new Error("no file chosen"));
        return;
      }
      try {
        const buf = await file.arrayBuffer();
        const bytes = new Uint8Array(buf);
        // Convert to base64
        let binary = "";
        const chunkSize = 0x8000;
        for (let i = 0; i < bytes.length; i += chunkSize) {
          const chunk = bytes.subarray(i, i + chunkSize);
          binary += String.fromCharCode.apply(null, Array.from(chunk));
        }
        const base64 = btoa(binary);
        await invoke("fs_write_file_base64", { path: targetPath, content_base64: base64, workspace: null, source: "file-import" });
        resolve();
      } catch (e) {
        reject(e);
      }
    };
    input.click();
  });
}

export async function importFileToHome(filename: string): Promise<void> {
  const home = await getAppHome();
  const target = home.replace(/\/$/, "") + "/" + filename.replace(/^\//, "");
  return importFileToPath(target);
}
