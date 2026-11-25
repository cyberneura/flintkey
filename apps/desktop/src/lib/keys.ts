export type Intent = 'next' | 'previous' | 'run' | 'hide';

export interface KeyInput {
  key: string;
  keyCode: number;
  isComposing: boolean;
  ctrlKey: boolean;
  shiftKey: boolean;
}

/** IME が変換中のキーにつける keyCode。 */
const IME_PROCESSING = 229;

/**
 * 入力欄で押されたキーの意味。ランチャーが扱わないキーは `null` (入力欄にそのまま渡す)。
 *
 * 変換中のキーは扱わない。WebKit は変換を確定する Enter を `isComposing: false` で送ってくるので、
 * keyCode も見る (見ないと、日本語を確定した瞬間にコマンドが実行される)。
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

/** 選択を `delta` だけ動かした添字。端で反対側へ回る。 */
export function moveSelection(selected: number, count: number, delta: number): number {
  if (count === 0) {
    return 0;
  }
  return (((selected + delta) % count) + count) % count;
}
