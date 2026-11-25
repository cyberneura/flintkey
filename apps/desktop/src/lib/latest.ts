/**
 * Of requests issued in succession, use only the result of the last one.
 *
 * A search runs on every keystroke, and responses may arrive out of order.
 * Never overwrite a newer result with an older one.
 */
export function latestOnly<T>(apply: (value: T) => void) {
  let latest = 0;
  let settled: Promise<void> = Promise.resolve();

  return {
    /** Issues a request. Results of earlier requests are discarded. */
    start(request: Promise<T>): Promise<void> {
      const id = ++latest;
      settled = request.then(
        (value) => {
          if (id === latest) {
            apply(value);
          }
        },
        // A failed request has no result. The caller handles it by awaiting the request itself.
        () => {},
      );
      return settled;
    },

    /** Waits until all issued requests settle, including those issued while waiting. */
    async settled(): Promise<void> {
      let seen: Promise<void>;
      do {
        seen = settled;
        await seen;
      } while (seen !== settled);
    },
  };
}
