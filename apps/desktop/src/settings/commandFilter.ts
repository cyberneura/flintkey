/** Which commands the tag pane shows: every command, those with no tag, or those with one tag. */
export type TagChoice = { kind: 'all' } | { kind: 'untagged' } | { kind: 'tag'; name: string };

interface Filterable {
  id: string;
  title: string;
  keywords: string[];
  tags: string[];
}

export interface TagCount {
  name: string;
  count: number;
}

/** Every tag in use with the number of commands carrying it, sorted by name. */
export function tagCounts(commands: Filterable[]): TagCount[] {
  const counts = new Map<string, number>();
  for (const command of commands) {
    // A tag written twice on one command counts once.
    for (const tag of new Set(command.tags)) {
      counts.set(tag, (counts.get(tag) ?? 0) + 1);
    }
  }
  return [...counts]
    .map(([name, count]) => ({ name, count }))
    .sort((a, b) => a.name.localeCompare(b.name));
}

export function untaggedCount(commands: Filterable[]): number {
  return commands.filter((command) => command.tags.length === 0).length;
}

/**
 * The choice to use once the list has been reloaded. A tag no command carries any more, or Untagged once every
 * command has a tag, falls back to all: its row is gone from the pane, so nothing would show what is chosen.
 */
export function settle(choice: TagChoice, commands: Filterable[]): TagChoice {
  const gone =
    (choice.kind === 'tag' && !commands.some((command) => command.tags.includes(choice.name))) ||
    (choice.kind === 'untagged' && untaggedCount(commands) === 0);
  return gone ? { kind: 'all' } : choice;
}

export function sameChoice(a: TagChoice, b: TagChoice): boolean {
  return a.kind === b.kind && (a.kind !== 'tag' || b.kind !== 'tag' || a.name === b.name);
}

/** The commands under `choice` whose title, id or a keyword contains `text` (ignoring case). */
export function filterCommands<T extends Filterable>(commands: T[], choice: TagChoice, text: string): T[] {
  const needle = text.trim().toLowerCase();
  return commands.filter((command) => {
    const inChoice =
      choice.kind === 'all' ||
      (choice.kind === 'untagged' ? command.tags.length === 0 : command.tags.includes(choice.name));
    if (!inChoice) return false;
    return (
      needle === '' ||
      command.title.toLowerCase().includes(needle) ||
      command.id.toLowerCase().includes(needle) ||
      command.keywords.some((keyword) => keyword.toLowerCase().includes(needle))
    );
  });
}
