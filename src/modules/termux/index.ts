import { invoke, Channel } from "@tauri-apps/api/core";

export type BootstrapStatus = {
  installed: boolean;
  arch: string;
  prefix: string | null;
  installing: boolean;
};

export type InstalledPackage = {
  name: string;
  version: string;
  description: string;
  installed: boolean;
};

export type BootstrapEvent =
  | { progress: { message: string; percent: number } }
  | { error: { message: string } }
  | { done: Record<string, never> }
  | { log: { message: string } };

/** Check if the Termux bootstrap is installed. */
export async function isTermuxInstalled(): Promise<boolean> {
  try {
    return await invoke<boolean>("termux_is_installed");
  } catch {
    return false;
  }
}

/** Get detailed bootstrap status. */
export async function getBootstrapStatus(): Promise<BootstrapStatus | null> {
  try {
    return await invoke<BootstrapStatus>("termux_bootstrap_status");
  } catch {
    return null;
  }
}

/**
 * Install the Termux bootstrap.
 * Calls back `onEvent` with progress/log events via Channel.
 */
export async function installBootstrap(
  onEvent: (event: BootstrapEvent) => void,
): Promise<void> {
  const channel = new Channel<BootstrapEvent>();
  channel.onmessage = onEvent;
  await invoke("termux_install_bootstrap", { onEvent: channel });
}

/** Run an apt command (e.g. `["update"]`, `["install", "openssh"]`). */
export async function runApt(args: string[]): Promise<string> {
  return await invoke<string>("termux_run_apt", { args });
}

/** List installed packages. */
export async function listPackages(): Promise<InstalledPackage[]> {
  try {
    return await invoke<InstalledPackage[]>("termux_list_packages");
  } catch {
    return [];
  }
}
