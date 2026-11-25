export type Intent = 'next' | 'previous' | 'run' | 'hide';

export interface KeyInput {
  key: string;
  keyCode: number;
  isComposing: boolean;
  ctrlKey: boolean;
  shiftKey: boolean;
}

/** The keyCode an IME gives keys during composition. */
const IME_PROCESSING = 229;

/**
 * What a key pressed in the input means. Keys the launcher does not handle are `null` (left to the input).
 *
 * Keys during IME composition are ignored. WebKit sends the Enter that commits composition with
 * `isComposing: false`, so keyCode is checked too (otherwise committing Japanese text would run a command).
 */
export function intentFor(input: KeyInput): Intent | null {
  if (input.isComposing || input.keyCode === IME_PROCESSING) {
    return null;
  }
  switch (input.key) {
    case 'Enter':
      return 'run';
    case 'Escape':
      return 'hide';
    case 'ArrowDown':
      return 'next';
    case 'ArrowUp':
      return 'previous';
    case 'Tab':
      return input.shiftKey ? 'previous' : 'next';
    case 'n':
      return input.ctrlKey ? 'next' : null;
    case 'p':
      return input.ctrlKey ? 'previous' : null;
    default:
      return null;
  }
}

/** The index after moving the selection by `delta`. Wraps around at the ends. */
export function moveSelection(selected: number, count: number, delta: number): number {
  if (count === 0) {
    return 0;
  }
  return (((selected + delta) % count) + count) % count;
}
