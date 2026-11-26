import { describe, expect, it } from 'vitest';
import { describeHotkey, EMPTY, formatHotkey, hotkeyFromKeyPress, KEY_OPTIONS, parseHotkey } from './hotkey';

const press = (code: string, mods: Partial<{ ctrl: boolean; alt: boolean; shift: boolean; meta: boolean }> = {}) => ({
  code,
  ctrlKey: mods.ctrl ?? false,
  altKey: mods.alt ?? false,
  shiftKey: mods.shift ?? false,
  metaKey: mods.meta ?? false,
});

describe('parseHotkey / formatHotkey', () => {
  it('reads every spelling of a combination and writes it in one order', () => {
    // Arrange
    const spellings = ['ctrl+alt+cmd+c', 'Command+Option+Control+KeyC', 'super+alt+ctrl+C'];

    for (const spelling of spellings) {
      // Act
      const written = formatHotkey(parseHotkey(spelling));

      // Assert
      expect(written, spelling).toBe('ctrl+alt+cmd+c');
    }
  });

  it('writes the combinations from the request', () => {
    // Arrange / Act / Assert
    expect(formatHotkey({ ...EMPTY, cmd: true, key: 'space' })).toBe('cmd+space');
    expect(formatHotkey({ ...EMPTY, ctrl: true, key: 'space' })).toBe('ctrl+space');
    expect(formatHotkey({ ...EMPTY, ctrl: true, shift: true, key: 'space' })).toBe('ctrl+shift+space');
  });

  it('folds the alternative key spellings', () => {
    expect(parseHotkey("shift+cmd+'").key).toBe('quote');
    expect(parseHotkey('ctrl+Digit1').key).toBe('1');
    expect(parseHotkey('ctrl+ArrowUp').key).toBe('up');
    expect(parseHotkey('ctrl+F13').key).toBe('f13');
  });

  it('writes nothing without a key, so an unfinished choice is not saved', () => {
    expect(formatHotkey({ ...EMPTY, ctrl: true })).toBe('');
    expect(parseHotkey('')).toEqual(EMPTY);
  });

  it('reads CmdOrCtrl as Command on macOS and Control elsewhere', () => {
    expect(formatHotkey(parseHotkey('CmdOrCtrl+Space', 'macos'))).toBe('cmd+space');
    expect(formatHotkey(parseHotkey('CommandOrControl+Space', 'windows'))).toBe('ctrl+space');
  });

  it('keeps an unknown word as the key so it can be shown and fixed', () => {
    expect(parseHotkey('ctrl+nope')).toEqual({ ...EMPTY, ctrl: true, key: 'nope' });
  });
});

describe('hotkeyFromKeyPress', () => {
  it('uses the physical key, not the character', () => {
    // Arrange: Option+C types "ç" on macOS, but the key is still C.
    const optionC = press('KeyC', { alt: true, meta: true });

    // Act
    const hotkey = hotkeyFromKeyPress(optionC);

    // Assert
    expect(hotkey).toEqual({ ctrl: false, alt: true, shift: false, cmd: true, key: 'c' });
  });

  it('reads the keys from the request and others', () => {
    expect(formatHotkey(hotkeyFromKeyPress(press('Space', { meta: true }))!)).toBe('cmd+space');
    expect(formatHotkey(hotkeyFromKeyPress(press('Space', { ctrl: true, shift: true }))!)).toBe('ctrl+shift+space');
    expect(hotkeyFromKeyPress(press('Digit5', { ctrl: true }))?.key).toBe('5');
    expect(hotkeyFromKeyPress(press('F19'))?.key).toBe('f19');
    expect(hotkeyFromKeyPress(press('Quote', { meta: true }))?.key).toBe('quote');
  });

  it('waits while only modifier keys are down', () => {
    for (const code of ['MetaLeft', 'ControlRight', 'AltLeft', 'ShiftLeft', 'CapsLock']) {
      expect(hotkeyFromKeyPress(press(code)), code).toBeNull();
    }
  });

  it('only gives keys that the key menu offers', () => {
    // Arrange
    const offered = new Set(KEY_OPTIONS.map((option) => option.value));
    const codes = ['Space', 'Enter', 'Tab', 'KeyZ', 'Digit0', 'F1', 'F20', 'ArrowLeft', 'Backquote', 'Slash'];

    for (const code of codes) {
      // Act
      const key = hotkeyFromKeyPress(press(code))?.key;

      // Assert
      expect(offered.has(key ?? ''), `${code} gave ${key}`).toBe(true);
    }
  });
});

describe('describeHotkey', () => {
  it('uses symbols on macOS and words elsewhere', () => {
    // Arrange
    const hotkey = parseHotkey('ctrl+shift+space');

    // Act / Assert
    expect(describeHotkey(hotkey, 'macos')).toBe('⌃⇧Space');
    expect(describeHotkey(hotkey, 'windows')).toBe('Ctrl+Shift+Space');
    expect(describeHotkey(parseHotkey('cmd+space'), 'windows')).toBe('Win+Space');
    expect(describeHotkey(EMPTY, 'macos')).toBe('');
  });
});
