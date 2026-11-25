import { describe, expect, it } from 'vitest';
import { latestOnly } from './latest';

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

describe('latestOnly', () => {
  it('applies the result of a single request', async () => {
    // Arrange
    const applied: string[] = [];
    const results = latestOnly<string>((value) => applied.push(value));

    // Act
    await results.start(Promise.resolve('a'));

    // Assert
    expect(applied).toEqual(['a']);
  });

  it('drops an older response that arrives after a newer one', async () => {
    // Arrange
    const applied: string[] = [];
    const results = latestOnly<string>((value) => applied.push(value));
    const older = deferred<string>();
    const newer = deferred<string>();
    const olderDone = results.start(older.promise);
    const newerDone = results.start(newer.promise);

    // Act
    newer.resolve('newer');
    await newerDone;
    older.resolve('older');
    await olderDone;

    // Assert
    expect(applied).toEqual(['newer']);
  });

  it('drops an older response that arrives before the newer one', async () => {
    // Arrange
    const applied: string[] = [];
    const results = latestOnly<string>((value) => applied.push(value));
    const older = deferred<string>();
    const newer = deferred<string>();
    const olderDone = results.start(older.promise);
    const newerDone = results.start(newer.promise);

    // Act
    older.resolve('older');
    await olderDone;
    newer.resolve('newer');
    await newerDone;

    // Assert
    expect(applied).toEqual(['newer']);
  });

  it('settled waits until the latest request has been applied', async () => {
    // Arrange: Enter must run only after the results for the finished text arrive.
    const applied: string[] = [];
    const results = latestOnly<string>((value) => applied.push(value));
    const pending = deferred<string>();
    results.start(pending.promise);
    const seenWhenSettled: string[][] = [];

    // Act
    const waiting = results.settled().then(() => seenWhenSettled.push([...applied]));
    await Promise.resolve();
    const beforeResponse = seenWhenSettled.length;
    pending.resolve('found');
    await waiting;

    // Assert
    expect(beforeResponse).toBe(0);
    expect(seenWhenSettled).toEqual([['found']]);
  });

  it('settled also waits for a request made while it was waiting', async () => {
    // Arrange: Enter was pressed while search A was pending, then one more character started search B. A returns first.
    const applied: string[] = [];
    const results = latestOnly<string>((value) => applied.push(value));
    const a = deferred<string>();
    const b = deferred<string>();
    results.start(a.promise);
    let done = false;
    const waiting = results.settled().then(() => {
      done = true;
    });
    results.start(b.promise);

    // Act
    a.resolve('a');
    await new Promise((resolve) => setTimeout(resolve, 0));
    const doneAfterA = done;
    b.resolve('b');
    await waiting;

    // Assert
    expect(doneAfterA).toBe(false);
    expect(applied).toEqual(['b']);
  });

  it('settled resolves immediately when nothing was requested', async () => {
    // Arrange
    const results = latestOnly<string>(() => {});

    // Act / Assert
    await expect(results.settled()).resolves.toBeUndefined();
  });

  it('a failed request applies nothing and does not block later ones', async () => {
    // Arrange
    const applied: string[] = [];
    const results = latestOnly<string>((value) => applied.push(value));

    // Act
    await results.start(Promise.reject(new Error('boom')));
    await results.start(Promise.resolve('after'));

    // Assert
    expect(applied).toEqual(['after']);
  });
});
