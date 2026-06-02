export type TerminalKeyEvent = Pick<
  KeyboardEvent,
  "altKey" | "ctrlKey" | "metaKey" | "shiftKey" | "key" | "code"
> & {
  keyCode?: number;
  type?: string;
  isComposing?: boolean;
};

export type PlatformOpts = { isMac: boolean };

export function terminalWordNavigationSequence(
  event: TerminalKeyEvent,
): string | null {
  if (!event.altKey || event.ctrlKey || event.metaKey) return null;
  if (event.key === "ArrowLeft" || event.code === "ArrowLeft") return "\x1bb";
  if (event.key === "ArrowRight" || event.code === "ArrowRight") return "\x1bf";
  return null;
}

/** Cmd+Left/Right → readline line-start (Ctrl+A) / line-end (Ctrl+E).
 * macOS-only — Cmd doesn't exist as a navigation modifier elsewhere. */
export function terminalLineNavigationSequence(
  event: TerminalKeyEvent,
  opts: PlatformOpts,
): string | null {
  if (!opts.isMac) return null;
  if (!event.metaKey || event.altKey || event.ctrlKey) return null;
  if (event.key === "ArrowLeft" || event.code === "ArrowLeft") return "\x01";
  if (event.key === "ArrowRight" || event.code === "ArrowRight") return "\x05";
  return null;
}

/** Modifier+Backspace deletion:
 *   macOS  Cmd+Backspace    → Ctrl+U (kill-to-line-start)
 *   macOS  Option+Backspace → Ctrl+W (kill-word-backward)
 *   Other  Ctrl+Backspace   → Ctrl+W (kill-word-backward)
 */
export function terminalDeleteSequence(
  event: TerminalKeyEvent,
  opts: PlatformOpts,
): string | null {
  if (event.key !== "Backspace" && event.code !== "Backspace") return null;
  if (opts.isMac) {
    if (event.metaKey && !event.altKey && !event.ctrlKey) return "\x15";
    if (event.altKey && !event.metaKey && !event.ctrlKey) return "\x17";
    return null;
  }
  if (event.ctrlKey && !event.altKey && !event.metaKey) return "\x17";
  return null;
}

/** Tauri/wry's WebView on Android fires a keydown for soft-keyboard printable
 * characters with `keyCode === 0`; the follow-up `keypress` is often omitted,
 * and xterm.js v6's input event handler rejects the subsequent `insertText`
 * because a keydown was already seen. The only safe place to forward the
 * character is here, from the custom key handler. Returns the literal char
 * to write to the PTY, or null when the standard xterm.js path should be
 * allowed to run. */
export function androidPrintableKeySequence(
  event: TerminalKeyEvent,
): string | null {
  if (event.type !== "keydown") return null;
  if (event.isComposing) return null;
  if (event.keyCode !== 0) return null;
  if (event.ctrlKey || event.altKey || event.metaKey) return null;
  if (event.key.length !== 1) return null;
  return event.key;
}
