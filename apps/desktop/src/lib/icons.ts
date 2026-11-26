/** One picture a row can show, as the app describes it. Passed back to the `icon` command to get the image. */
export type IconRef =
  | { kind: 'app'; path: string }
  | { kind: 'file'; path: string }
  | { kind: 'url'; url: string }
  | { kind: 'favicon'; url: string };

/** An image URL once loaded, or `failed`. Absent while loading. */
export type IconState = string | 'failed';

export const iconKey = (icon: IconRef): string => JSON.stringify(icon);

/** SVG must be labelled as such to be drawn from a blob; the other formats are recognised by their bytes. */
export function mimeOf(bytes: Uint8Array): string {
  const start = new TextDecoder().decode(bytes.subarray(0, 1024)).replace(/^﻿/, '').trimStart().toLowerCase();
  return start.startsWith('<') && start.includes('<svg') ? 'image/svg+xml' : '';
}

/**
 * Fetches each icon once, including while its first request is still on its way. An icon that fails is not asked
 * for again (the app retries on its side after a while; the page is reloaded with the window).
 */
export function iconLoader(
  fetchImage: (icon: IconRef) => Promise<ArrayBuffer>,
  onSettled: (key: string, state: IconState) => void,
) {
  const asked = new Set<string>();

  return {
    /** Asks again for every icon from now on, after the configuration changed. */
    reset() {
      asked.clear();
    },

    load(icons: Iterable<IconRef>) {
      for (const icon of icons) {
        const key = iconKey(icon);
        if (asked.has(key)) continue;
        asked.add(key);
        fetchImage(icon).then(
          (buffer) => {
            const bytes = new Uint8Array(buffer);
            onSettled(key, URL.createObjectURL(new Blob([bytes], { type: mimeOf(bytes) })));
          },
          () => onSettled(key, 'failed'),
        );
      }
    },
  };
}

/**
 * The image to show for a row whose icons are `icons`, best first: the first one loaded. While a better one is
 * still loading nothing is chosen, so a lower one does not flash up and get replaced. `null` means the row shows
 * the icon of its type.
 */
export function pickIcon(icons: IconRef[], states: Record<string, IconState>): string | null {
  for (const icon of icons) {
    const state = states[iconKey(icon)];
    if (state === undefined) return null;
    if (state !== 'failed') return state;
  }
  return null;
}
