/**
 * 続けて出した要求のうち、最後に出したものの結果だけを採用する。
 *
 * 検索は 1 文字ごとに走り、応答は出した順に返るとは限らない。古い応答で新しい結果を上書きしない。
 */
export function latestOnly<T>(apply: (value: T) => void) {
  let latest = 0;
  let settled: Promise<void> = Promise.resolve();

  return {
    /** 要求を出す。これ以前に出した要求の結果は捨てる。 */
    start(request: Promise<T>): Promise<void> {
      const id = ++latest;
      settled = request.then(
        (value) => {
          if (id === latest) {
            apply(value);
          }
        },
        // 失敗した要求は結果を持たない。呼び出し元が要求そのものを待って扱う。
        () => {},
      );
      return settled;
    },

    /** 出してある要求がすべて片付くまで待つ。待っている間に出た要求も待つ。 */
    async settled(): Promise<void> {
      let seen: Promise<void>;
      do {
        seen = settled;
        await seen;
      } while (seen !== settled);
    },
  };
}
