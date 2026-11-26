import { describe, expect, it } from 'vitest';
import { filterCommands, sameChoice, settle, tagCounts, untaggedCount, type TagChoice } from './commandFilter';

const command = (id: string, tags: string[], keywords: string[] = [], title = id) => ({ id, title, keywords, tags });

const commands = [
  command('site', ['web', 'work'], ['s']),
  command('blog', ['web'], [], 'My Blog'),
  command('deploy', ['work'], ['デプロイ']),
  command('notes', []),
];

describe('tagCounts', () => {
  it('counts each tag once per command and sorts by name', () => {
    // Arrange
    const withRepeat = [...commands, command('again', ['web', 'web'])];

    // Act
    const counts = tagCounts(withRepeat);

    // Assert
    expect(counts).toEqual([
      { name: 'web', count: 3 },
      { name: 'work', count: 2 },
    ]);
  });

  it('counts the commands without tags apart', () => {
    // Act / Assert
    expect(untaggedCount(commands)).toBe(1);
  });
});

describe('filterCommands', () => {
  const ids = (choice: TagChoice, text = '') => filterCommands(commands, choice, text).map((c) => c.id);

  it('shows every command, only untagged ones, or one tag', () => {
    // Act / Assert
    expect(ids({ kind: 'all' })).toEqual(['site', 'blog', 'deploy', 'notes']);
    expect(ids({ kind: 'untagged' })).toEqual(['notes']);
    expect(ids({ kind: 'tag', name: 'work' })).toEqual(['site', 'deploy']);
  });

  it('matches the text against title, id and keywords within the chosen tag', () => {
    // Act / Assert
    expect(ids({ kind: 'all' }, ' blog ')).toEqual(['blog']);
    expect(ids({ kind: 'all' }, 'MY')).toEqual(['blog']);
    expect(ids({ kind: 'tag', name: 'work' }, 'デプ')).toEqual(['deploy']);
    expect(ids({ kind: 'tag', name: 'web' }, 'deploy')).toEqual([]);
  });
});

describe('settle', () => {
  it('falls back to all when the chosen row is no longer in the pane', () => {
    // Arrange
    const allTagged = commands.filter((c) => c.tags.length > 0);

    // Act / Assert
    expect(settle({ kind: 'tag', name: 'gone' }, commands)).toEqual({ kind: 'all' });
    expect(settle({ kind: 'untagged' }, allTagged)).toEqual({ kind: 'all' });
  });

  it('keeps a choice that still has commands', () => {
    // Act / Assert
    expect(settle({ kind: 'tag', name: 'web' }, commands)).toEqual({ kind: 'tag', name: 'web' });
    expect(settle({ kind: 'untagged' }, commands)).toEqual({ kind: 'untagged' });
  });
});

describe('sameChoice', () => {
  it('compares tag names only between tags', () => {
    // Act / Assert
    expect(sameChoice({ kind: 'tag', name: 'a' }, { kind: 'tag', name: 'a' })).toBe(true);
    expect(sameChoice({ kind: 'tag', name: 'a' }, { kind: 'tag', name: 'b' })).toBe(false);
    expect(sameChoice({ kind: 'all' }, { kind: 'untagged' })).toBe(false);
    expect(sameChoice({ kind: 'all' }, { kind: 'all' })).toBe(true);
  });
});
