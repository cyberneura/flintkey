/**
 * Hotkey strings as written in the YAML files (`ctrl+alt+cmd+c`), and the pieces the settings window edits.
 *
 * The desktop app decides whether a hotkey can be registered; this only reads and writes the text.
 */

export type Os = 'macos' | 'windows' | 'linux';

export interface Hotkey {
  ctrl: boolean;
  alt: boolean;
  shift: boolean;
  cmd: boolean;
  /** Key name such as `space`, `c`, `1`, `f5`, `quote`. Empty when no key is chosen yet. */
  key: string;
}

export const EMPTY: Hotkey = { ctrl: false, alt: false, shift: false, cmd: false, key: '' };

const SYMBOLS: Record<string, string> = {
  "'": 'quote',
  ',': 'comma',
  '.': 'period',
  '/': 'slash',
  ';': 'semicolon',
  '\\': 'backslash',
  '`': 'backquote',
  '[': 'bracketleft',
  ']': 'bracketright',
  '-': 'minus',
  '=': 'equal',
  esc: 'escape',
};

/** Folds the spellings the global shortcut library accepts (`KeyC`, `Digit1`, `ArrowUp`, `'`) into one name. */
function keyName(token: string): string {
  if (token in SYMBOLS) {
    return SYMBOLS[token];
  }
  const letterOrDigit = /^(?:key|digit)([a-z0-9])$/.exec(token);
  if (letterOrDigit) {
    return letterOrDigit[1];
  }
  return token.startsWith('arrow') ? token.slice('arrow'.length) : token;
}

/**
 * Reads a hotkey string. Unknown words are kept as the key, so they can still be shown and fixed.
 * `CmdOrCtrl` (accepted by the global shortcut library) means Command on macOS and Control elsewhere.
 */
export function parseHotkey(text: string, os: Os = 'macos'): Hotkey {
  const hotkey: Hotkey = { ...EMPTY };
  for (const raw of text.split('+')) {
    const token = raw.trim().toLowerCase();
    if (token === '') {
      continue;
    }
    if (token === 'ctrl' || token === 'control') {
      hotkey.ctrl = true;
    } else if (token === 'alt' || token === 'option') {
      hotkey.alt = true;
    } else if (token === 'shift') {
      hotkey.shift = true;
    } else if (token === 'cmd' || token === 'command' || token === 'super') {
      hotkey.cmd = true;
    } else if (/^(?:cmd|command)or(?:ctrl|control)$/.test(token)) {
      hotkey[os === 'macos' ? 'cmd' : 'ctrl'] = true;
    } else {
      hotkey.key = keyName(token);
    }
  }
  return hotkey;
}

/** The hotkey string to save. Empty when no key is chosen. */
export function formatHotkey(hotkey: Hotkey): string {
  if (hotkey.key === '') {
    return '';
  }
  const parts: string[] = [];
  if (hotkey.ctrl) parts.push('ctrl');
  if (hotkey.alt) parts.push('alt');
  if (hotkey.shift) parts.push('shift');
  if (hotkey.cmd) parts.push('cmd');
  parts.push(hotkey.key);
  return parts.join('+');
}

const CODES: Record<string, string> = {
  Space: 'space',
  Enter: 'enter',
  Tab: 'tab',
  Escape: 'escape',
  Backspace: 'backspace',
  Backquote: 'backquote',
  Minus: 'minus',
  Equal: 'equal',
  BracketLeft: 'bracketleft',
  BracketRight: 'bracketright',
  Backslash: 'backslash',
  Semicolon: 'semicolon',
  Quote: 'quote',
  Comma: 'comma',
  Period: 'period',
  Slash: 'slash',
  ArrowUp: 'up',
  ArrowDown: 'down',
  ArrowLeft: 'left',
  ArrowRight: 'right',
};

export interface KeyPress {
  code: string;
  ctrlKey: boolean;
  altKey: boolean;
  shiftKey: boolean;
  metaKey: boolean;
}

/**
 * The hotkey a key press means, or `null` while only modifier keys are down.
 *
 * Uses the physical key (`code`), not the character: with Option or a non-US layout held, the character
 * changes but the key the global shortcut library registers does not.
 */
export function hotkeyFromKeyPress(press: KeyPress): Hotkey | null {
  let key: string | undefined = CODES[press.code];
  const letterOrDigit = /^(?:Key([A-Z])|Digit([0-9]))$/.exec(press.code);
  if (letterOrDigit) {
    key = (letterOrDigit[1] ?? letterOrDigit[2]).toLowerCase();
  }
  if (/^F([1-9]|1[0-9]|2[0-4])$/.test(press.code)) {
    key = press.code.toLowerCase();
  }
  if (key === undefined) {
    return null;
  }
  return { ctrl: press.ctrlKey, alt: press.altKey, shift: press.shiftKey, cmd: press.metaKey, key };
}

export interface KeyOption {
  value: string;
  label: string;
}

/** The keys offered in the key menu, in menu order. */
export const KEY_OPTIONS: KeyOption[] = [
  { value: 'space', label: 'Space' },
  { value: 'enter', label: 'Return' },
  { value: 'tab', label: 'Tab' },
  ...'abcdefghijklmnopqrstuvwxyz'.split('').map((c) => ({ value: c, label: c.toUpperCase() })),
  ...'0123456789'.split('').map((c) => ({ value: c, label: c })),
  ...Array.from({ length: 20 }, (_, i) => ({ value: `f${i + 1}`, label: `F${i + 1}` })),
  { value: 'up', label: '↑' },
  { value: 'down', label: '↓' },
  { value: 'left', label: '←' },
  { value: 'right', label: '→' },
  { value: 'backquote', label: '`' },
  { value: 'minus', label: '-' },
  { value: 'equal', label: '=' },
  { value: 'bracketleft', label: '[' },
  { value: 'bracketright', label: ']' },
  { value: 'backslash', label: '\\' },
  { value: 'semicolon', label: ';' },
  { value: 'quote', label: "'" },
  { value: 'comma', label: ',' },
  { value: 'period', label: '.' },
  { value: 'slash', label: '/' },
];

export interface ModifierLabels {
  ctrl: string;
  alt: string;
  shift: string;
  cmd: string;
}

/** What the modifier keys are called on each OS. `cmd` is the Windows key on Windows. */
export function modifierLabels(os: Os): ModifierLabels {
  if (os === 'macos') {
    return { ctrl: '⌃ Control', alt: '⌥ Option', shift: '⇧ Shift', cmd: '⌘ Command' };
  }
  return { ctrl: 'Ctrl', alt: 'Alt', shift: 'Shift', cmd: os === 'windows' ? 'Win' : 'Super' };
}

/** A short human-readable form: `⌃⇧Space` on macOS, `Ctrl+Shift+Space` elsewhere. Empty without a key. */
export function describeHotkey(hotkey: Hotkey, os: Os): string {
  if (hotkey.key === '') {
    return '';
  }
  const key = KEY_OPTIONS.find((option) => option.value === hotkey.key)?.label ?? hotkey.key;
  if (os === 'macos') {
    return `${hotkey.ctrl ? '⌃' : ''}${hotkey.alt ? '⌥' : ''}${hotkey.shift ? '⇧' : ''}${hotkey.cmd ? '⌘' : ''}${key}`;
  }
  const labels = modifierLabels(os);
  const parts: string[] = [];
  if (hotkey.ctrl) parts.push(labels.ctrl);
  if (hotkey.alt) parts.push(labels.alt);
  if (hotkey.shift) parts.push(labels.shift);
  if (hotkey.cmd) parts.push(labels.cmd);
  parts.push(key);
  return parts.join('+');
}
