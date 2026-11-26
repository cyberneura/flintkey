/**
 * Image URLs for app icons, fetched once per bundle path.
 *
 * Rows ask for their icons every time the list changes, which is every keystroke. Each path is fetched at most
 * once, including while its first request is still on its way; a path that has no icon is not asked for again.
 */
export function iconLoader(fetchPng: (path: string) => Promise<ArrayBuffer>, onReady: (path: string, url: string) => void) {
  const asked = new Set<string>();

  return {
    load(paths: Iterable<string | null>) {
      for (const path of paths) {
        if (path === null || asked.has(path)) continue;
        asked.add(path);
        fetchPng(path).then(
          (png) => onReady(path, URL.createObjectURL(new Blob([png], { type: 'image/png' }))),
          // The row keeps its blank space; the list works the same without the picture.
          () => {},
        );
      }
    },
  };
}
