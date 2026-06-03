import { describe, expect, it } from "vitest";

import {
  androidPrintableKeySequence,
  terminalDeleteSequence,
  terminalLineNavigationSequence,
  terminalWordNavigationSequence,
  type TerminalKeyEvent,
} from "./keymap";

const evt = (partial: Partial<TerminalKeyEvent>): TerminalKeyEvent => ({
  altKey: false,
  ctrlKey: false,
  metaKey: false,
  shiftKey: false,
  key: "",
  code: "",
  ...partial,
});

describe("terminalWordNavigationSequence", () => {
  it("maps Option+Left to readline word-left", () => {
    expect(
      terminalWordNavigationSequence(
        evt({ altKey: true, key: "ArrowLeft", code: "ArrowLeft" }),
      ),
    ).toBe("\x1bb");
  });

  it("maps Option+Right to readline word-right", () => {
    expect(
      terminalWordNavigationSequence(
        evt({ altKey: true, key: "ArrowRight", code: "ArrowRight" }),
      ),
    ).toBe("\x1bf");
  });

  it("does not remap plain arrows", () => {
    expect(
      terminalWordNavigationSequence(
        evt({ key: "ArrowLeft", code: "ArrowLeft" }),
      ),
    ).toBeNull();
  });
});

describe("terminalLineNavigationSequence", () => {
  it("maps Cmd+Left to readline line-start on macOS", () => {
    expect(
      terminalLineNavigationSequence(
        evt({ metaKey: true, key: "ArrowLeft", code: "ArrowLeft" }),
        { isMac: true },
      ),
    ).toBe("\x01");
  });

  it("maps Cmd+Right to readline line-end on macOS", () => {
    expect(
      terminalLineNavigationSequence(
        evt({ metaKey: true, key: "ArrowRight", code: "ArrowRight" }),
        { isMac: true },
      ),
    ).toBe("\x05");
  });

  it("does not remap Cmd+Arrow off macOS", () => {
    expect(
      terminalLineNavigationSequence(
        evt({ metaKey: true, key: "ArrowLeft", code: "ArrowLeft" }),
        { isMac: false },
      ),
    ).toBeNull();
  });

  it("does not remap Cmd+Option+Arrow (selection-style combos pass through)", () => {
    expect(
      terminalLineNavigationSequence(
        evt({
          metaKey: true,
          altKey: true,
          key: "ArrowLeft",
          code: "ArrowLeft",
        }),
        { isMac: true },
      ),
    ).toBeNull();
  });
});

describe("terminalDeleteSequence", () => {
  it("maps Cmd+Backspace to kill-to-line-start on macOS", () => {
    expect(
      terminalDeleteSequence(
        evt({ metaKey: true, key: "Backspace", code: "Backspace" }),
        { isMac: true },
      ),
    ).toBe("\x15");
  });

  it("maps Option+Backspace to kill-word-backward on macOS", () => {
    expect(
      terminalDeleteSequence(
        evt({ altKey: true, key: "Backspace", code: "Backspace" }),
        { isMac: true },
      ),
    ).toBe("\x17");
  });

  it("maps Ctrl+Backspace to kill-word-backward off macOS", () => {
    expect(
      terminalDeleteSequence(
        evt({ ctrlKey: true, key: "Backspace", code: "Backspace" }),
        { isMac: false },
      ),
    ).toBe("\x17");
  });

  it("does not remap Ctrl+Backspace on macOS (reserved for native readline binding)", () => {
    expect(
      terminalDeleteSequence(
        evt({ ctrlKey: true, key: "Backspace", code: "Backspace" }),
        { isMac: true },
      ),
    ).toBeNull();
  });

  it("does not remap Cmd+Backspace off macOS", () => {
    expect(
      terminalDeleteSequence(
        evt({ metaKey: true, key: "Backspace", code: "Backspace" }),
        { isMac: false },
      ),
    ).toBeNull();
  });

  it("does not remap plain Backspace", () => {
    expect(
      terminalDeleteSequence(evt({ key: "Backspace", code: "Backspace" }), {
        isMac: true,
      }),
    ).toBeNull();
  });
});

describe("androidPrintableKeySequence", () => {
  it("forwards a plain soft-keyboard letter", () => {
    expect(
      androidPrintableKeySequence(
        evt({ type: "keydown", keyCode: 0, key: "a", code: "a" }),
      ),
    ).toBe("a");
  });

  it("forwards uppercase when shift is held", () => {
    expect(
      androidPrintableKeySequence(
        evt({
          type: "keydown",
          keyCode: 0,
          key: "A",
          code: "KeyA",
          shiftKey: true,
        }),
      ),
    ).toBe("A");
  });

  it("forwards non-ASCII printable keys (accented chars, CJK, emoji)", () => {
    expect(
      androidPrintableKeySequence(
        evt({ type: "keydown", keyCode: 0, key: "é", code: "KeyE" }),
      ),
    ).toBe("é");
  });

  it("ignores keypress events", () => {
    expect(
      androidPrintableKeySequence(
        evt({ type: "keypress", keyCode: 0, key: "a", code: "a" }),
      ),
    ).toBeNull();
  });

  it("ignores composing events so the IME composition flow stays in xterm", () => {
    expect(
      androidPrintableKeySequence(
        evt({
          type: "keydown",
          keyCode: 0,
          key: "a",
          code: "a",
          isComposing: true,
        }),
      ),
    ).toBeNull();
  });

  it("ignores physical-key events (keyCode set)", () => {
    expect(
      androidPrintableKeySequence(
        evt({ type: "keydown", keyCode: 65, key: "a", code: "KeyA" }),
      ),
    ).toBeNull();
  });

  it("ignores events with modifiers", () => {
    expect(
      androidPrintableKeySequence(
        evt({
          type: "keydown",
          keyCode: 0,
          key: "a",
          code: "a",
          ctrlKey: true,
        }),
      ),
    ).toBeNull();
    expect(
      androidPrintableKeySequence(
        evt({ type: "keydown", keyCode: 0, key: "a", code: "a", altKey: true }),
      ),
    ).toBeNull();
    expect(
      androidPrintableKeySequence(
        evt({
          type: "keydown",
          keyCode: 0,
          key: "a",
          code: "a",
          metaKey: true,
        }),
      ),
    ).toBeNull();
  });

  it("ignores multi-character keys (ArrowLeft, Enter, etc.)", () => {
    expect(
      androidPrintableKeySequence(
        evt({
          type: "keydown",
          keyCode: 0,
          key: "Enter",
          code: "Enter",
        }),
      ),
    ).toBeNull();
  });

  it("forwards a plain letter with keyCode 229 (Android WebView variant)", () => {
    expect(
      androidPrintableKeySequence(
        evt({ type: "keydown", keyCode: 229, key: "a", code: "KeyA" }),
      ),
    ).toBe("a");
  });

  it("forwards uppercase with keyCode 229 when shift is held", () => {
    expect(
      androidPrintableKeySequence(
        evt({
          type: "keydown",
          keyCode: 229,
          key: "A",
          code: "KeyA",
          shiftKey: true,
        }),
      ),
    ).toBe("A");
  });

  it("ignores non-printable keys with keyCode 229 (Enter)", () => {
    expect(
      androidPrintableKeySequence(
        evt({
          type: "keydown",
          keyCode: 229,
          key: "Enter",
          code: "Enter",
        }),
      ),
    ).toBeNull();
  });

  it("ignores composing events with keyCode 229", () => {
    expect(
      androidPrintableKeySequence(
        evt({
          type: "keydown",
          keyCode: 229,
          key: "a",
          code: "KeyA",
          isComposing: true,
        }),
      ),
    ).toBeNull();
  });

  it("still ignores normal physical-key events (keyCode 65)", () => {
    expect(
      androidPrintableKeySequence(
        evt({ type: "keydown", keyCode: 65, key: "a", code: "KeyA" }),
      ),
    ).toBeNull();
  });
});
