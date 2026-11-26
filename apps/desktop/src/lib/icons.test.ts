import { describe, expect, it, vi } from 'vitest';
import { iconLoader } from './icons';

const flush = () => new Promise((resolve) => setTimeout(resolve, 0));

describe('iconLoader', () => {
  it('fetches each path once, even while the first request is pending', async () => {
    // Arrange
    const fetchPng = vi.fn(async (_path: string) => new Uint8Array([1, 2, 3]).buffer);
    const ready: string[] = [];
    const loader = iconLoader(fetchPng, (path) => ready.push(path));

    // Act
    loader.load(['/Applications/A.app', null, '/Applications/B.app']);
    loader.load(['/Applications/A.app']);
    await flush();
    loader.load(['/Applications/B.app']);
    await flush();

    // Assert
    expect(fetchPng.mock.calls.map(([path]) => path)).toEqual(['/Applications/A.app', '/Applications/B.app']);
    expect(ready).toEqual(['/Applications/A.app', '/Applications/B.app']);
  });

  it('does not ask again for a path that has no icon', async () => {
    // Arrange
    const fetchPng = vi.fn(async () => {
      throw new Error('no icon');
    });
    const ready: string[] = [];
    const loader = iconLoader(fetchPng, (path) => ready.push(path));

    // Act
    loader.load(['/Applications/Broken.app']);
    await flush();
    loader.load(['/Applications/Broken.app']);
    await flush();

    // Assert
    expect(fetchPng).toHaveBeenCalledTimes(1);
    expect(ready).toEqual([]);
  });
});
