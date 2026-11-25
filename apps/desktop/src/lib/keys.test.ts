import { describe, expect, it } from 'vitest';
import { intentFor, moveSelection, type KeyInput } from './keys';

function key(key: string, overrides: Partial<KeyInput> = {}): KeyInput {
  return { key, keyCode: 0, isComposing: false, ctrlKey: false, shiftKey: false, ...overrides };
}

describe('intentFor', () => {
  it.each([
    ['Enter', {}, 'run'],
    ['Escape', {}, 'hide'],
    ['ArrowDown', {}, 'next'],
    ['ArrowUp', {}, 'previous'],
    ['Tab', {}, 'next'],
    ['Tab', { shiftKey: true }, 'previous'],
    ['n', { ctrlKey: true }, 'next'],
    ['p', { ctrlKey: true }, 'previous'],
  ] as const)('%s %o means %s', (name, overrides, expected) => {
    // Arrange
    const input = key(name, overrides);

    // Act
    const intent = intentFor(input);

    // Assert
    expect(intent).toBe(expected);
  });

  it.each(['n', 'p', 'a', ' ', 'Backspace', 'ArrowLeft'])('leaves a plain %j to the input field', (name) => {
    expect(intentFor(key(name))).toBeNull();
  });

  it.each(['Enter', 'Escape', 'ArrowDown', 'ArrowUp', 'Tab'])('ignores %s while the IME is composing', (name) => {
    expect(intentFor(key(name, { isComposing: true, keyCode: 229 }))).toBeNull();
  });

  it('ignores Enter while composing even when the engine reports the real key code', () => {
    // Arrange
    const composing = key('Enter', { isComposing: true, keyCode: 13 });

    // Act
    const intent = intentFor(composing);

    // Assert
    expect(intent).toBeNull();
  });

  it('ignores the Enter that confirms a composition, which WebKit reports as not composing', () => {
    // Arrange
    const confirming = key('Enter', { isComposing: false, keyCode: 229 });

    // Act
    const intent = intentFor(confirming);

    // Assert
    expect(intent).toBeNull();
  });
});

describe('moveSelection', () => {
  it.each([
    [0, 3, 1, 1],
    [2, 3, 1, 0],
    [0, 3, -1, 2],
    [1, 3, -1, 0],
    [0, 1, 1, 0],
    [0, 0, 1, 0],
    [0, 0, -1, 0],
  ])('from %i of %i by %i gives %i', (selected, count, delta, expected) => {
    expect(moveSelection(selected, count, delta)).toBe(expected);
  });
});
