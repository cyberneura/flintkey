import { describe, expect, it, vi } from 'vitest';
import { iconKey, iconLoader, mimeOf, pickIcon, type IconRef, type IconState } from './icons';

const flush = () => new Promise((resolve) => setTimeout(resolve, 0));
const app = (path: string): IconRef => ({ kind: 'app', path });
const bytes = (text: string) => new TextEncoder().encode(text);

describe('iconLoader', () => {
  it('fetches each icon once, even while the first request is pending', async () => {
    // Arrange
    const fetchImage = vi.fn(async (_icon: IconRef) => new Uint8Array([0x89, 0x50]).buffer);
    const settled: string[] = [];
    const loader = iconLoader(fetchImage, (key) => settled.push(key));

    // Act
    loader.load([app('/Applications/A.app'), app('/Applications/B.app')]);
    loader.load([app('/Applications/A.app')]);
    await flush();
    loader.load([app('/Applications/B.app')]);
    await flush();

    // Assert
    expect(fetchImage).toHaveBeenCalledTimes(2);
    expect(settled).toEqual([iconKey(app('/Applications/A.app')), iconKey(app('/Applications/B.app'))]);
  });

  it('asks again for a failed icon only after retryFailed', async () => {
    // Arrange
    let online = false;
    const fetchImage = vi.fn(async (_icon: IconRef) => {
      if (!online) throw new Error('offline');
      return new Uint8Array([0x89, 0x50]).buffer;
    });
    const states: IconState[] = [];
    const loader = iconLoader(fetchImage, (_key, state) => states.push(state));
    loader.load([app('/Applications/A.app')]);
    await flush();

    // Act
    online = true;
    loader.load([app('/Applications/A.app')]);
    await flush();
    const retried = loader.retryFailed();
    loader.load([app('/Applications/A.app')]);
    await flush();

    // Assert
    expect(retried).toEqual([iconKey(app('/Applications/A.app'))]);
    expect(fetchImage).toHaveBeenCalledTimes(2);
    expect(states[0]).toBe('failed');
    expect(states[1]).not.toBe('failed');
  });

  it('drops an answer to a request made before reset', async () => {
    // Arrange
    let rejectOld!: (reason: unknown) => void;
    const fetchImage = vi
      .fn<(icon: IconRef) => Promise<ArrayBuffer>>()
      .mockImplementationOnce(() => new Promise((_resolve, reject) => (rejectOld = reject)))
      .mockImplementationOnce(async () => new Uint8Array([0x89, 0x50]).buffer);
    const states: IconState[] = [];
    const loader = iconLoader(fetchImage, (_key, state) => states.push(state));

    // Act
    loader.load([app('/Applications/A.app')]);
    loader.reset();
    loader.load([app('/Applications/A.app')]);
    await flush();
    rejectOld(new Error('old'));
    await flush();

    // Assert
    expect(states).toHaveLength(1);
    expect(states[0]).not.toBe('failed');
  });

  it('reports a failed icon once and does not ask for it again', async () => {
    // Arrange
    const fetchImage = vi.fn(async (_icon: IconRef): Promise<ArrayBuffer> => {
      throw new Error('no image');
    });
    const states: IconState[] = [];
    const loader = iconLoader(fetchImage, (_key, state) => states.push(state));

    // Act
    loader.load([app('/Applications/Broken.app')]);
    await flush();
    loader.load([app('/Applications/Broken.app')]);
    await flush();

    // Assert
    expect(fetchImage).toHaveBeenCalledTimes(1);
    expect(states).toEqual(['failed']);
  });
});

describe('pickIcon', () => {
  const own: IconRef = { kind: 'url', url: 'https://example.com/own.png' };
  const favicon: IconRef = { kind: 'favicon', url: 'https://example.com/' };
  const tag: IconRef = { kind: 'file', path: '/icons/work.svg' };

  it('shows the best icon that loaded, skipping failed ones', () => {
    // Act / Assert
    expect(pickIcon([own, favicon, tag], { [iconKey(own)]: 'blob:own' })).toBe('blob:own');
    expect(
      pickIcon([own, favicon, tag], { [iconKey(own)]: 'failed', [iconKey(favicon)]: 'failed', [iconKey(tag)]: 'blob:tag' }),
    ).toBe('blob:tag');
    expect(pickIcon([own], { [iconKey(own)]: 'failed' })).toBeNull();
    expect(pickIcon([], {})).toBeNull();
  });

  it('waits for a better icon that is still loading instead of showing a lower one', () => {
    // Act / Assert
    expect(pickIcon([own, favicon, tag], { [iconKey(favicon)]: 'blob:favicon', [iconKey(tag)]: 'blob:tag' })).toBeNull();
  });
});

describe('mimeOf', () => {
  it('labels SVG and leaves other formats to be recognised by their bytes', () => {
    // Act / Assert
    expect(mimeOf(bytes('﻿<?xml version="1.0"?><svg xmlns="http://www.w3.org/2000/svg"/>'))).toBe('image/svg+xml');
    expect(mimeOf(bytes('  <svg viewBox="0 0 1 1"/>'))).toBe('image/svg+xml');
    expect(mimeOf(new Uint8Array([0x89, 0x50, 0x4e, 0x47]))).toBe('');
  });
});
